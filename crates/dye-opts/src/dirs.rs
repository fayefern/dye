use std::{
    array, fs, io,
    path::{self, Path, PathBuf},
};

use directories::ProjectDirs;

#[derive(Clone, Debug)]
pub struct RootDir {
    root: PathBuf,
    overrides: [Option<PathBuf>; DirSlot::ALL.len()],
}

impl RootDir {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let root = path::absolute(&root).unwrap_or(root);

        Self {
            root,
            overrides: array::from_fn(|_| None),
        }
    }

    pub fn platform_default() -> Self {
        let root = ProjectDirs::from("", "", "dye")
            .map(|d| d.data_dir().to_path_buf())
            .unwrap_or_else(|| PathBuf::from("dye-data"));

        Self::new(root)
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn options(&self) -> PathBuf {
        self.path(DirSlot::Options)
    }
    pub fn logs(&self) -> PathBuf {
        self.path(DirSlot::Logs)
    }
    pub fn saves(&self) -> PathBuf {
        self.path(DirSlot::Saves)
    }
    pub fn packs(&self) -> PathBuf {
        self.path(DirSlot::Packs)
    }

    pub fn path(&self, slot: DirSlot) -> PathBuf {
        match &self.overrides[slot.index()] {
            Some(p) => p.clone(),
            None => self.root.join(slot.default_name()),
        }
    }

    pub fn set_override(&mut self, slot: DirSlot, path: impl AsRef<Path>) {
        let p = path.as_ref();
        let resolved = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.root.join(p)
        };
        self.overrides[slot.index()] = Some(resolved);
    }

    pub fn clear_override(&mut self, slot: DirSlot) {
        self.overrides[slot.index()] = None;
    }

    pub fn is_overridden(&self, slot: DirSlot) -> bool {
        self.overrides[slot.index()].is_some()
    }

    pub fn ensure(&self, slot: DirSlot) -> io::Result<PathBuf> {
        let p = self.path(slot);
        fs::create_dir_all(&p)?;

        Ok(p)
    }

    pub fn entries(&self) -> impl Iterator<Item = (DirSlot, PathBuf, bool)> + '_ {
        DirSlot::ALL
            .into_iter()
            .map(|s| (s, self.path(s), self.is_overridden(s)))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DirSlot {
    Options,
    Logs,
    Saves,
    Packs,
}

impl DirSlot {
    pub const ALL: [Self; 4] = [Self::Options, Self::Logs, Self::Saves, Self::Packs];

    pub const fn default_name(self) -> &'static str {
        match self {
            Self::Options => "data/options",
            Self::Logs => "data/logs",
            Self::Saves => "data/saves",
            Self::Packs => "data/packs",
        }
    }

    pub const fn index(self) -> usize {
        self as usize
    }
}
