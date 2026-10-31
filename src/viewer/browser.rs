//! Catalog state, selection, and ordering without filesystem work

use std::path::PathBuf;

use crate::settings::SortOrder;

use super::catalog::{self, Entry, ScannedFolder};

pub(crate) struct Browser {
    entries: Vec<Entry>,
    folder: Option<PathBuf>,
    index: usize,
    serial: u64,
}

impl Browser {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            folder: None,
            index: 0,
            serial: 0,
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn current(&self) -> Option<&Entry> {
        self.entries.get(self.index)
    }

    pub fn folder(&self) -> Option<&std::path::Path> {
        self.folder.as_deref()
    }

    pub fn index(&self) -> usize {
        self.index
    }

    pub fn generation(&self) -> u64 {
        self.serial
    }

    pub fn select(&mut self, index: usize) -> Option<PathBuf> {
        let path = self.entries.get(index)?.path.clone();
        self.index = index;

        Some(path)
    }

    pub fn next(&self, delta: isize, wrap: bool) -> Option<usize> {
        catalog::next_index(self.index, self.entries.len(), delta, wrap)
            .filter(|next| *next != self.index)
    }

    pub fn refresh_path(&self) -> Option<PathBuf> {
        self.current()
            .map(|entry| entry.path.clone())
            .or_else(|| self.folder.clone())
    }

    pub fn begin_scan(&mut self) -> u64 {
        self.serial += 1;

        self.serial
    }

    pub fn replace(&mut self, result: ScannedFolder, order: SortOrder, reverse: bool) {
        let mut entries = result.entries;
        catalog::sort(&mut entries, order, reverse);

        self.index = result
            .selected
            .and_then(|path| entries.iter().position(|entry| entry.path == path))
            .unwrap_or(0);
        self.entries = entries;
        self.folder = Some(result.folder);
    }

    pub fn sort(&mut self, order: SortOrder, reverse: bool) {
        let selected = self.current().map(|entry| entry.path.clone());

        catalog::sort(&mut self.entries, order, reverse);

        if let Some(path) = selected {
            self.index = self
                .entries
                .iter()
                .position(|entry| entry.path == path)
                .unwrap_or(0);
        }
    }

    pub fn rename(
        &mut self,
        old: &std::path::Path,
        new: Entry,
        order: SortOrder,
        reverse: bool,
    ) {
        if let Some(entry) = self.entries.iter_mut().find(|entry| entry.path == old) {
            *entry = new;
        }

        self.sort(order, reverse);
    }

    pub fn remove(&mut self, path: &std::path::Path) -> Option<usize> {
        let index = self.entries.iter().position(|entry| entry.path == path)?;

        self.entries.remove(index);
        self.index = index.min(self.entries.len().saturating_sub(1));

        Some(self.index)
    }
}
