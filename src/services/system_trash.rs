//! System trash integration for native and sandboxed applications

use std::path::Path;

use anyhow::{Context, Result};

#[cfg(target_os = "linux")]
pub fn delete(path: &Path) -> Result<()> {
    let file = open_for_trash(path)?;
    let connection = zbus::blocking::connection::Builder::session()
        .context("Could not locate the desktop session bus")?
        .method_timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Could not connect to the desktop Trash portal")?;

    trash_file(&file, &connection)
}

#[cfg(not(target_os = "linux"))]
pub fn delete(path: &Path) -> Result<()> {
    trash::delete(path).context("The native trash operation failed")
}

#[cfg(target_os = "linux")]
fn open_for_trash(path: &Path) -> Result<std::fs::File> {
    use std::{fs::OpenOptions, os::unix::fs::OpenOptionsExt};

    OpenOptions::new()
        .read(true)
        .write(true)
        // Do not trash a symlink's target when the link itself was selected
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("Could not open {} for trashing", path.display()))
}

#[cfg(target_os = "linux")]
fn trash_file(
    file: &std::fs::File,
    connection: &zbus::blocking::Connection,
) -> Result<()> {
    use std::os::fd::AsFd;

    let descriptor = zbus::zvariant::Fd::from(file.as_fd());

    let reply = connection
        .call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.Trash"),
            "TrashFile",
            &(descriptor,),
        )
        .context("The desktop Trash portal request failed")?;

    let result: u32 = reply
        .body()
        .deserialize()
        .context("The desktop Trash portal returned an invalid response")?;

    anyhow::ensure!(
        result == 1,
        "The desktop Trash portal could not trash the image"
    );

    Ok(())
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::{
        fs::File,
        io::{Read, Seek, Write},
        os::{fd::AsFd, unix::net::UnixStream},
        sync::mpsc,
        time::Duration,
    };

    use anyhow::Result;
    use zbus::{blocking::Connection, zvariant::Fd};

    use super::{open_for_trash, trash_file};

    struct TestPortal {
        response: Option<u32>,
        received: mpsc::Sender<Vec<u8>>,
    }

    #[zbus::interface(name = "org.freedesktop.portal.Trash")]
    impl TestPortal {
        fn trash_file(&self, descriptor: Fd<'_>) -> zbus::fdo::Result<u32> {
            let read_write = || -> Result<()> {
                let mut file = File::from(descriptor.as_fd().try_clone_to_owned()?);
                let mut contents = Vec::new();
                file.read_to_end(&mut contents)?;
                file.rewind()?;
                file.write_all(&contents)?;
                self.received.send(contents)?;

                Ok(())
            };
            read_write()
                .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;

            self.response.ok_or_else(|| {
                zbus::fdo::Error::NotSupported("Trash unavailable".into())
            })
        }
    }

    fn portal_connections(portal: TestPortal) -> Result<(Connection, Connection)> {
        let (client_socket, server_socket) = UnixStream::pair()?;
        let server = std::thread::spawn(move || {
            zbus::blocking::connection::Builder::async_io_unix_stream(server_socket)
                .server(zbus::Guid::generate())?
                .p2p()
                .serve_at("/org/freedesktop/portal/desktop", portal)?
                .build()
        });
        let client =
            zbus::blocking::connection::Builder::async_io_unix_stream(client_socket)
                .p2p()
                .method_timeout(Duration::from_secs(3))
                .build()?;
        let server = server
            .join()
            .map_err(|_| anyhow::anyhow!("Test portal thread panicked"))??;

        Ok((client, server))
    }

    #[test]
    fn trash_portal_receives_writable_image_and_controls_the_result() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("image.png");
        std::fs::write(&source, b"Selected image")?;

        for response in [Some(1), Some(0), Some(2), None] {
            let (received, contents) = mpsc::channel();
            let (connection, _server) =
                portal_connections(TestPortal { response, received })?;
            let file = open_for_trash(&source)?;
            let result = trash_file(&file, &connection);

            assert_eq!(
                result.is_ok(),
                response == Some(1),
                "Response {response:?}: {result:?}"
            );
            assert_eq!(
                contents.recv_timeout(Duration::from_secs(1))?,
                b"Selected image"
            );
            // Only the host portal moves files; refusal must never trigger a local fallback
            assert_eq!(std::fs::read(&source)?, b"Selected image");
        }

        Ok(())
    }
}
