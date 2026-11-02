//! System trash integration for native and sandboxed applications

use std::path::Path;

use anyhow::{Context, Result};

#[cfg(target_os = "linux")]
pub fn delete(path: &Path) -> Result<()> {
    let connection = zbus::blocking::connection::Builder::session()
        .context("Could not locate the desktop session bus")?
        .method_timeout(std::time::Duration::from_secs(30))
        .build()
        .context("Could not connect to the desktop Trash portal")?;

    delete_with_connection(path, &connection)
}

#[cfg(not(target_os = "linux"))]
pub fn delete(path: &Path) -> Result<()> {
    trash::delete(path).context("The native trash operation failed")
}

#[cfg(target_os = "linux")]
fn delete_with_connection(
    path: &Path,
    connection: &zbus::blocking::Connection,
) -> Result<()> {
    use std::{
        fs::OpenOptions,
        os::{fd::AsFd, unix::fs::OpenOptionsExt},
    };

    let file = OpenOptions::new()
        .read(true)
        .write(true)
        // Do not trash a symlink's target when the link itself was selected
        .custom_flags(libc::O_NOFOLLOW)
        .open(path)
        .with_context(|| format!("Could not open {} for trashing", path.display()))?;
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
        path::PathBuf,
        time::Duration,
    };

    use zbus::{blocking::Connection, zvariant::Fd};

    use super::*;

    struct TestPortal {
        source: PathBuf,
        destination: PathBuf,
        result: u32,
    }

    #[zbus::interface(name = "org.freedesktop.portal.Trash")]
    impl TestPortal {
        fn trash_file(&self, descriptor: Fd<'_>) -> zbus::fdo::Result<u32> {
            let operation = || -> Result<()> {
                let mut file = File::from(descriptor.as_fd().try_clone_to_owned()?);
                let mut contents = Vec::new();
                file.read_to_end(&mut contents)?;

                // Verify that the descriptor grants the read/write access required by the portal
                file.rewind()?;
                file.write_all(&contents)?;

                if self.result == 1 {
                    std::fs::write(&self.destination, contents)?;
                    std::fs::remove_file(&self.source)?;
                }

                Ok(())
            };

            operation().map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;

            Ok(self.result)
        }
    }

    fn portal_connections(
        portal: Option<TestPortal>,
    ) -> Result<(Connection, Connection)> {
        let (client_socket, server_socket) = UnixStream::pair()?;
        let server = std::thread::spawn(move || -> zbus::Result<Connection> {
            let mut builder =
                zbus::blocking::connection::Builder::async_io_unix_stream(
                    server_socket,
                )
                .server(zbus::Guid::generate())?
                .p2p();

            if let Some(portal) = portal {
                builder =
                    builder.serve_at("/org/freedesktop/portal/desktop", portal)?;
            }

            builder.build()
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
    fn portal_receives_the_selected_image_with_read_write_access() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("image.png");
        let destination = directory.path().join("desktop-trash.png");
        std::fs::write(&source, b"Selected image")?;
        let (connection, _server) = portal_connections(Some(TestPortal {
            source: source.clone(),
            destination: destination.clone(),
            result: 1,
        }))?;

        delete_with_connection(&source, &connection)?;

        assert!(!source.exists());
        assert_eq!(std::fs::read(destination)?, b"Selected image");

        Ok(())
    }

    #[test]
    fn unsuccessful_portal_responses_preserve_the_image() -> Result<()> {
        for result in [0, 2] {
            let directory = tempfile::tempdir()?;
            let source = directory.path().join("image.png");
            let destination = directory.path().join("desktop-trash.png");
            std::fs::write(&source, b"Keep this image")?;
            let (connection, _server) = portal_connections(Some(TestPortal {
                source: source.clone(),
                destination: destination.clone(),
                result,
            }))?;

            let error = delete_with_connection(&source, &connection)
                .expect_err("An unsuccessful portal response must fail");

            assert!(error.to_string().contains("could not trash the image"));
            assert_eq!(std::fs::read(source)?, b"Keep this image");
            assert!(!destination.exists());
        }

        Ok(())
    }

    #[test]
    fn unavailable_portal_keeps_the_image_without_local_fallback() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let source = directory.path().join("image.png");
        std::fs::write(&source, b"Keep this image")?;
        let (connection, _server) = portal_connections(None)?;

        let error = delete_with_connection(&source, &connection)
            .expect_err("An unavailable portal must fail");

        assert!(error.to_string().contains("Trash portal request failed"));
        assert_eq!(std::fs::read(source)?, b"Keep this image");

        Ok(())
    }

    #[test]
    fn trashing_a_symlink_does_not_trash_its_target() -> Result<()> {
        let directory = tempfile::tempdir()?;
        let target = directory.path().join("original.png");
        let link = directory.path().join("link.png");
        std::fs::write(&target, b"Keep the original image")?;
        std::os::unix::fs::symlink(&target, &link)?;
        let (connection, _server) = portal_connections(None)?;

        assert!(delete_with_connection(&link, &connection).is_err());
        assert!(link.is_symlink());
        assert_eq!(std::fs::read(target)?, b"Keep the original image");

        Ok(())
    }
}
