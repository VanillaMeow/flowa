//! Background work, codecs, filesystem access, and desktop integration

pub(crate) mod file_actions;
pub(crate) mod folder_scan;
pub(crate) mod image_decode;
pub(crate) mod image_loader;
pub(crate) mod resize;
pub(crate) mod system_trash;
pub(crate) mod thumbnails;
pub(crate) mod tiles;
pub(crate) mod worker;

#[cfg(test)]
mod tests;
