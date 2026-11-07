//! File dialogs, exports, clipboard copies, renames, and trashing on a worker thread

use std::{
    borrow::Cow,
    path::{Path, PathBuf},
    sync::{Arc, mpsc},
    thread,
};

use anyhow::{Context, Result, ensure};
use image::RgbaImage;

use crate::{
    utils::filename,
    viewer::{catalog, transform::View},
};

use super::{image_decode, system_trash};

pub enum Task {
    Pick {
        folder: bool,
        start: Option<PathBuf>,
    },
    Export {
        pixels: Arc<RgbaImage>,
        source: PathBuf,
        view: View,
    },
    Copy {
        pixels: Arc<RgbaImage>,
        view: View,
    },
    Rename {
        source: PathBuf,
        name: String,
    },
    Trash(PathBuf),
}

pub enum Outcome {
    Picked(PathBuf),
    /// The user dismissed a file dialog
    Cancelled,
    Exported,
    Copied,
    Renamed {
        old: PathBuf,
        new: PathBuf,
    },
    Trashed(PathBuf),
}

pub struct Actions {
    sender: mpsc::Sender<Task>,
    pub results: mpsc::Receiver<Result<Outcome>>,
}

impl Actions {
    pub fn new(ctx: egui::Context) -> Self {
        let (sender, rx) = mpsc::channel();
        let (tx, results) = mpsc::channel();

        thread::spawn(move || {
            // Keep clipboard ownership alive between copy requests, including on X11
            let mut clipboard = None;

            while let Ok(task) = rx.recv() {
                let result = perform(task, &mut clipboard);

                if tx.send(result).is_err() {
                    break;
                }

                ctx.request_repaint();
            }
        });

        Self { sender, results }
    }

    pub fn request(&self, task: Task) {
        let _ = self.sender.send(task);
    }
}

/// Opens the folder containing a file without waiting for the file manager
pub fn reveal(path: &Path) -> Result<()> {
    let folder = path.parent().context("The image has no parent folder")?;
    open::that_detached(folder).context("Could not open the file manager")
}

fn perform(task: Task, clipboard: &mut Option<arboard::Clipboard>) -> Result<Outcome> {
    match task {
        Task::Pick { folder, start } => Ok(pick(folder, start)),
        Task::Export {
            pixels,
            source,
            view,
        } => export(&pixels, &source, &view),
        Task::Copy { pixels, view } => copy(&pixels, &view, clipboard),
        Task::Rename { source, name } => rename(source, &name),
        Task::Trash(path) => {
            system_trash::delete(&path)
                .context("Could not move image to the system trash")?;
            Ok(Outcome::Trashed(path))
        }
    }
}

fn pick(folder: bool, start: Option<PathBuf>) -> Outcome {
    let title = if folder { "Open folder" } else { "Open file" };
    let mut dialog = rfd::FileDialog::new().set_title(title);

    if let Some(start) = start {
        dialog = dialog.set_directory(start);
    }

    let picked = if folder {
        dialog.pick_folder()
    } else {
        let extensions: Vec<_> = catalog::supported_extensions().collect();
        dialog
            .add_filter("Images and videos", &extensions)
            .add_filter("All files", &["*"])
            .pick_file()
    };

    picked.map_or(Outcome::Cancelled, Outcome::Picked)
}

fn export(pixels: &RgbaImage, source: &Path, view: &View) -> Result<Outcome> {
    let stem = source.file_stem().unwrap_or_default().to_string_lossy();
    let mut dialog = rfd::FileDialog::new()
        .set_title("Export a copy")
        .set_file_name(format!("{stem}-flowa.png"))
        .add_filter("PNG", &["png"])
        .add_filter("JPEG", &["jpg", "jpeg"])
        .add_filter("WebP", &["webp"])
        .add_filter("TIFF", &["tif", "tiff"])
        .add_filter("BMP", &["bmp"]);

    if let Some(parent) = source.parent() {
        dialog = dialog.set_directory(parent);
    }

    let Some(path) = dialog.save_file() else {
        return Ok(Outcome::Cancelled);
    };

    ensure!(
        !same_file(&path, source),
        "Choose a different file. Export creates a copy and preserves your original."
    );

    image_decode::export(pixels, &path, view)?;
    Ok(Outcome::Exported)
}

fn copy(
    pixels: &RgbaImage,
    view: &View,
    clipboard: &mut Option<arboard::Clipboard>,
) -> Result<Outcome> {
    let rgba = image_decode::apply_view(pixels, view).into_rgba8();

    let clipboard = match clipboard {
        Some(clipboard) => clipboard,
        None => clipboard.insert(
            arboard::Clipboard::new().context("Could not connect to the clipboard")?,
        ),
    };

    clipboard
        .set_image(arboard::ImageData {
            width: rgba.width() as usize,
            height: rgba.height() as usize,
            bytes: Cow::Borrowed(rgba.as_raw()),
        })
        .context("Could not copy image")?;

    Ok(Outcome::Copied)
}

fn rename(source: PathBuf, name: &str) -> Result<Outcome> {
    ensure!(
        filename::is_valid(name),
        "Choose a valid filename without path separators or reserved characters."
    );

    let new = source
        .parent()
        .context("The image has no parent folder")?
        .join(name);

    if new == source {
        return Ok(Outcome::Renamed { old: source, new });
    }

    // Case-insensitive file systems report a case-only rename's target as existing
    if same_file(&new, &source) {
        std::fs::rename(&source, &new).context("Could not rename image")?;
    } else {
        rename_without_replacing(&source, &new)?;
    }

    Ok(Outcome::Renamed { old: source, new })
}

/// Whether two paths name the same file, including aliases and symlinks
fn same_file(first: &Path, second: &Path) -> bool {
    if first == second {
        return true;
    }

    match (first.canonicalize(), second.canonicalize()) {
        (Ok(first), Ok(second)) => first == second,
        _ => false,
    }
}

#[cfg(target_os = "linux")]
fn rename_without_replacing(source: &Path, destination: &Path) -> Result<()> {
    use std::{ffi::CString, io, os::unix::ffi::OsStrExt};

    let source_name = CString::new(source.as_os_str().as_bytes())?;
    let destination_name = CString::new(destination.as_os_str().as_bytes())?;

    // SAFETY: Both paths are NUL-terminated and borrowed for the duration of the call
    let result = unsafe {
        libc::renameat2(
            libc::AT_FDCWD,
            source_name.as_ptr(),
            libc::AT_FDCWD,
            destination_name.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };

    if result == 0 {
        return Ok(());
    }

    let error = io::Error::last_os_error();
    match error.raw_os_error() {
        Some(libc::EEXIST) => anyhow::bail!("A file with that name already exists."),
        // Some file systems, such as older FUSE drivers, cannot rename atomically
        Some(libc::EINVAL | libc::ENOSYS) => check_then_rename(source, destination),
        _ => Err(error).context("Could not rename image"),
    }
}

#[cfg(not(target_os = "linux"))]
fn rename_without_replacing(source: &Path, destination: &Path) -> Result<()> {
    check_then_rename(source, destination)
}

fn check_then_rename(source: &Path, destination: &Path) -> Result<()> {
    ensure!(
        !destination
            .try_exists()
            .context("Could not check the new filename")?,
        "A file with that name already exists."
    );

    std::fs::rename(source, destination).context("Could not rename image")
}

#[cfg(test)]
mod tests {
    use anyhow::Result;

    use super::{Outcome, Task, perform};

    #[test]
    fn rename_preserves_other_files_and_refuses_collisions() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("a.png");
        let destination = dir.path().join("b.png");
        std::fs::write(&source, b"original")?;
        std::fs::write(&destination, b"keep me")?;
        assert!(
            perform(
                Task::Rename {
                    source: source.clone(),
                    name: "b.png".into()
                },
                &mut None
            )
            .is_err()
        );
        assert_eq!(std::fs::read(&source)?, b"original");
        assert_eq!(std::fs::read(&destination)?, b"keep me");

        let result = perform(
            Task::Rename {
                source: source.clone(),
                name: "renamed.png".into(),
            },
            &mut None,
        )?;
        assert!(matches!(result, Outcome::Renamed { .. }));
        assert!(!source.exists());
        assert_eq!(std::fs::read(dir.path().join("renamed.png"))?, b"original");
        assert_eq!(std::fs::read(destination)?, b"keep me");

        Ok(())
    }

    #[test]
    fn renaming_only_the_letter_case_succeeds() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("photo.png");
        std::fs::write(&source, b"original")?;

        perform(
            Task::Rename {
                source,
                name: "Photo.png".into(),
            },
            &mut None,
        )?;

        assert_eq!(std::fs::read(dir.path().join("Photo.png"))?, b"original");
        Ok(())
    }
}
