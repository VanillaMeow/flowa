//! Catalog entries, ordering, and navigation rules

use std::{path::PathBuf, time::SystemTime};

use super::formats::MediaKind;
use crate::settings::SortOrder;

#[derive(Clone, Debug)]
pub struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub kind: MediaKind,
    pub size: u64,
    pub modified: SystemTime,
}

// Scan data has no worker or filesystem dependencies
pub struct ScannedFolder {
    pub folder: PathBuf,
    pub selected: Option<PathBuf>,
    pub entries: Vec<Entry>,
}

pub fn sort(files: &mut [Entry], order: SortOrder, reverse: bool) {
    files.sort_by(|a, b| {
        let name = || {
            natord::compare_ignore_case(&a.name, &b.name)
                .then_with(|| a.path.cmp(&b.path))
        };
        let cmp = match order {
            SortOrder::Name => name(),
            SortOrder::Modified => b.modified.cmp(&a.modified).then_with(name),
            SortOrder::Size => b.size.cmp(&a.size).then_with(name),
        };
        if reverse { cmp.reverse() } else { cmp }
    });
}

pub fn next_index(
    current: usize,
    len: usize,
    delta: isize,
    wrap: bool,
) -> Option<usize> {
    if len == 0 {
        return None;
    }
    let next = current as isize + delta;
    Some(if wrap {
        next.rem_euclid(len as isize) as usize
    } else {
        next.clamp(0, len as isize - 1) as usize
    })
}
