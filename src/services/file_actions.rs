use std::{
    borrow::Cow,
    path::PathBuf,
    sync::{Arc, mpsc},
    thread,
};

use anyhow::{Context, Result, ensure};
use image::{DynamicImage, RgbaImage};

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
    Reveal(PathBuf),
}

pub enum Outcome {
    Picked(Option<PathBuf>),
    Exported,
    Copied,
    Renamed { old: PathBuf, new: PathBuf },
    Trashed(PathBuf),
    Revealed,
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

fn perform(task: Task, clipboard: &mut Option<arboard::Clipboard>) -> Result<Outcome> {
    match task {
        Task::Pick { folder, start } => {
            let mut dialog = rfd::FileDialog::new().set_title(if folder {
                "Open image folder"
            } else {
                "Open image"
            });
            if let Some(start) = start {
                dialog = dialog.set_directory(start);
            }

            if folder {
                Ok(Outcome::Picked(dialog.pick_folder()))
            } else {
                let mut extensions = catalog::EXTENSIONS.to_vec();

                if cfg!(feature = "avif") {
                    extensions.push("avif");
                }
                if cfg!(feature = "heif") {
                    extensions.extend(["heic", "heif"]);
                }

                Ok(Outcome::Picked(
                    dialog
                        .add_filter("Images", &extensions)
                        .add_filter("All files", &["*"])
                        .pick_file(),
                ))
            }
        }
        Task::Export {
            pixels,
            source,
            view,
        } => {
            let stem = source.file_stem().unwrap_or_default().to_string_lossy();
            let dialog = rfd::FileDialog::new()
                .set_title("Export a copy")
                .set_file_name(format!("{stem}-flowa.png"))
                .add_filter("PNG", &["png"])
                .add_filter("JPEG", &["jpg", "jpeg"])
                .add_filter("WebP", &["webp"])
                .add_filter("TIFF", &["tiff"]);
            let dialog = if let Some(parent) = source.parent() {
                dialog.set_directory(parent)
            } else {
                dialog
            };

            if let Some(path) = dialog.save_file() {
                // Canonical paths also catch aliases and symlinks to the original
                let same = path == source
                    || path
                        .canonicalize()
                        .ok()
                        .zip(source.canonicalize().ok())
                        .is_some_and(|(a, b)| a == b);

                ensure!(
                    !same,
                    "Choose a different file. Export creates a copy and preserves your original."
                );

                image_decode::export(
                    &pixels,
                    &path,
                    view.rotation,
                    view.flip_x,
                    view.flip_y,
                )?;
                Ok(Outcome::Exported)
            } else {
                Ok(Outcome::Picked(None))
            }
        }
        Task::Copy { pixels, view } => {
            let mut image = DynamicImage::ImageRgba8(pixels.as_ref().clone());

            if view.flip_x {
                image = image.fliph();
            }

            if view.flip_y {
                image = image.flipv();
            }

            image = match view.rotation % 4 {
                1 => image.rotate90(),
                2 => image.rotate180(),
                3 => image.rotate270(),
                _ => image,
            };

            let rgba = image.into_rgba8();

            if clipboard.is_none() {
                *clipboard = Some(
                    arboard::Clipboard::new()
                        .context("Could not connect to the clipboard")?,
                );
            }

            if let Some(clipboard) = clipboard {
                clipboard
                    .set_image(arboard::ImageData {
                        width: rgba.width() as usize,
                        height: rgba.height() as usize,
                        bytes: Cow::Borrowed(rgba.as_raw()),
                    })
                    .context("Could not copy image")?;
            }

            Ok(Outcome::Copied)
        }
        Task::Rename { source, name } => {
            ensure!(
                filename::is_valid(&name),
                "Choose a valid filename without path separators or reserved characters."
            );

            let new = source
                .parent()
                .context("The image has no parent folder")?
                .join(name);

            if new == source {
                return Ok(Outcome::Renamed { old: source, new });
            }

            ensure!(
                !new.try_exists()
                    .context("Could not check the new filename")?,
                "A file with that name already exists."
            );

            std::fs::rename(&source, &new).context("Could not rename image")?;
            Ok(Outcome::Renamed { old: source, new })
        }
        Task::Trash(path) => {
            system_trash::delete(&path)
                .context("Could not move image to the system trash")?;
            Ok(Outcome::Trashed(path))
        }
        Task::Reveal(path) => {
            let folder = path.parent().context("The image has no parent folder")?;
            open::that(folder).context("Could not open the file manager")?;
            Ok(Outcome::Revealed)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rename_refuses_to_replace_an_existing_image() -> Result<()> {
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
        assert_eq!(std::fs::read(source)?, b"original");
        assert_eq!(std::fs::read(destination)?, b"keep me");
        Ok(())
    }

    #[test]
    fn rename_moves_only_the_selected_file() -> Result<()> {
        let dir = tempfile::tempdir()?;
        let source = dir.path().join("a.png");
        std::fs::write(&source, b"original")?;
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
        Ok(())
    }
}
