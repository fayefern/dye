use std::{
    collections::{BTreeMap, HashMap},
    fs::{self, File},
    hash::Hasher,
    io::{self, Read},
    path::{Path, PathBuf},
    sync::{Mutex, PoisonError},
    time::UNIX_EPOCH,
};

use fnv::FnvHasher;
use include_dir::{Dir, DirEntry};
use zip::ZipArchive;

use crate::error::PathError;

pub trait PackSource: Send + Sync {
    fn label(&self) -> String;
    fn list(&self) -> io::Result<Listing>;
    fn read(&self, path: &str) -> io::Result<Vec<u8>>;
}

fn check_safe(path: &str) -> Result<(), PathError> {
    if path.is_empty() {
        return Err(PathError::Empty);
    }
    if path.contains('\\') {
        return Err(PathError::Backslash);
    }
    if let Some(c) = path.chars().find(|c| matches!(c, '\0' | ':')) {
        return Err(PathError::BadChar(c));
    }
    if path.starts_with('/') {
        return Err(PathError::Absolute);
    }
    if path.split('/').any(|s| matches!(s, "" | "." | "..")) {
        return Err(PathError::BadSegment);
    }

    Ok(())
}

fn check_name(path: &str) -> Result<(), PathError> {
    for c in path.chars() {
        match c {
            'a'..='z' | '0'..='9' | '_' | '-' | '.' | '/' => {}
            'A'..='Z' => return Err(PathError::Uppercase),
            c => return Err(PathError::BadChar(c)),
        }
    }

    Ok(())
}

fn is_hidden(path: &str) -> bool {
    path.split('/').any(|s| s.starts_with('.')) || path.starts_with("__MACOSX/")
}

fn mix(parts: &[u64]) -> u64 {
    let mut hasher = FnvHasher::default();
    for part in parts {
        hasher.write(&part.to_le_bytes());
    }

    hasher.finish()
}

fn invalid(e: PathError) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, e)
}

pub struct DirSource {
    root: PathBuf,
}

impl DirSource {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }
}

impl PackSource for DirSource {
    fn label(&self) -> String {
        self.root.display().to_string()
    }

    fn list(&self) -> io::Result<Listing> {
        let mut out = Listing::default();
        let mut stack = vec![(self.root.clone(), String::new())];

        while let Some((dir, prefix)) = stack.pop() {
            for read in fs::read_dir(&dir)? {
                let entry = read?;
                let os_name = entry.file_name();
                let Some(name) = os_name.to_str() else {
                    out.rejected.push(RejectedEntry {
                        path: os_name.to_string_lossy().into_owned(),
                        reason: PathError::NotUtf8,
                    });
                    continue;
                };
                let rel = if prefix.is_empty() {
                    name.to_owned()
                } else {
                    format!("{prefix}/{name}")
                };
                if is_hidden(&rel) {
                    continue;
                }

                let ty = entry.file_type()?;
                if ty.is_symlink() {
                    out.rejected.push(RejectedEntry {
                        path: rel,
                        reason: PathError::Symlink,
                    });
                } else if ty.is_dir() {
                    if let Err(e) = check_safe(&rel) {
                        out.rejected.push(RejectedEntry {
                            path: rel,
                            reason: e,
                        });
                        continue;
                    }
                    let meta = entry.metadata()?;
                    let mtime = meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_nanos() as u64);
                    out.files.push(FileEntry {
                        path: rel,
                        size: meta.len(),
                        fingerprint: mix(&[meta.len(), mtime]),
                    });
                }
            }
        }

        out.files.sort_by(|a, b| a.path.cmp(&b.path));

        Ok(out)
    }

    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        check_safe(path).map_err(invalid)?;

        fs::read(self.root.join(path))
    }
}

pub struct ZipSource {
    path: PathBuf,
    listing: Listing,
    slots: HashMap<String, usize>,
    idle: Mutex<Vec<ZipArchive<File>>>,
}

impl ZipSource {
    const MAX_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
    const MAX_IDLE_ARCHIVE: usize = 8;

    pub fn open(path: impl Into<PathBuf>) -> io::Result<Self> {
        let path = path.into();
        let mut archive = ZipArchive::new(File::open(&path)?).map_err(io::Error::from)?;

        let mut listing = Listing::default();
        let mut slots = HashMap::new();

        for idx in 0..archive.len() {
            let file = archive.by_index_raw(idx).map_err(io::Error::from)?;
            if file.is_dir() {
                continue;
            }
            let name = file.name()?.to_string();
            if is_hidden(&name) {
                continue;
            }
            if file.is_symlink() {
                listing.rejected.push(RejectedEntry {
                    path: name,
                    reason: PathError::Symlink,
                });
            } else if let Err(e) = check_safe(&name) {
                listing.rejected.push(RejectedEntry {
                    path: name,
                    reason: e,
                });
            } else if slots.contains_key(&name) {
                listing.rejected.push(RejectedEntry {
                    path: name,
                    reason: PathError::Duplicate,
                });
            } else {
                slots.insert(name.clone(), idx);
                listing.files.push(FileEntry {
                    path: name,
                    size: file.size(),
                    fingerprint: mix(&[file.size(), file.crc32() as u64]),
                });
            }
        }

        listing.files.sort_by(|a, b| a.path.cmp(&b.path));

        Ok(Self {
            path,
            listing,
            slots,
            idle: Mutex::new(vec![archive]),
        })
    }

    fn checkout(&self) -> io::Result<ZipArchive<File>> {
        let pooled = self
            .idle
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop();

        match pooled {
            Some(a) => Ok(a),
            None => ZipArchive::new(File::open(&self.path)?).map_err(io::Error::from),
        }
    }

    fn give_back(&self, archive: ZipArchive<File>) {
        let mut idle = self.idle.lock().unwrap_or_else(PoisonError::into_inner);
        if idle.len() < Self::MAX_IDLE_ARCHIVE {
            idle.push(archive);
        }
    }
}

impl PackSource for ZipSource {
    fn label(&self) -> String {
        self.path.display().to_string()
    }

    fn list(&self) -> io::Result<Listing> {
        let files = self
            .listing
            .files
            .iter()
            .map(|f| FileEntry {
                path: f.path.clone(),
                size: f.size,
                fingerprint: f.fingerprint,
            })
            .collect();
        let rejected = self
            .listing
            .rejected
            .iter()
            .map(|r| RejectedEntry {
                path: r.path.clone(),
                reason: r.reason.clone(),
            })
            .collect();

        Ok(Listing { files, rejected })
    }

    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        check_safe(path).map_err(invalid)?;
        let &slot = self
            .slots
            .get(path)
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))?;

        let mut archive = self.checkout()?;
        let file = archive.by_index(slot).map_err(io::Error::from)?;
        let declared = file.size();
        if declared > Self::MAX_ENTRY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "zip entry too large",
            ));
        }

        let mut result = Vec::with_capacity(declared.min(16 * 1024 * 1024) as usize);
        file.take(Self::MAX_ENTRY_BYTES + 1)
            .read_to_end(&mut result)?;
        if result.len() as u64 > Self::MAX_ENTRY_BYTES {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "zip entry too large",
            ));
        }

        self.give_back(archive);

        Ok(result)
    }
}

pub struct EmbeddedSource {
    label: &'static str,
    dir: &'static Dir<'static>,
}

impl EmbeddedSource {
    pub const fn new(label: &'static str, dir: &'static Dir<'static>) -> Self {
        Self { label, dir }
    }
}

impl PackSource for EmbeddedSource {
    fn label(&self) -> String {
        self.label.to_owned()
    }

    fn list(&self) -> io::Result<Listing> {
        let mut result = Listing::default();

        fn walk(dir: &Dir<'static>, result: &mut Listing) {
            for entry in dir.entries() {
                match entry {
                    DirEntry::Dir(d) => walk(d, result),
                    DirEntry::File(f) => {
                        let path = rel_path(f.path());
                        if is_hidden(&path) {
                            continue;
                        }
                        match check_safe(&path) {
                            Ok(()) => result.files.push(FileEntry {
                                path,
                                size: f.contents().len() as u64,
                                fingerprint: mix(&[f.contents().len() as u64]),
                            }),
                            Err(e) => result.rejected.push(RejectedEntry { path, reason: e }),
                        }
                    }
                }
            }
        }

        walk(self.dir, &mut result);
        result.files.sort_by(|a, b| a.path.cmp(&b.path));

        Ok(result)
    }

    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        check_safe(path).map_err(invalid)?;

        self.dir
            .get_file(path)
            .map(|f| f.contents().to_vec())
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

pub struct MemorySource {
    label: String,
    files: BTreeMap<String, Vec<u8>>,
}

impl MemorySource {
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            files: BTreeMap::new(),
        }
    }

    pub fn with(mut self, path: &str, bytes: impl Into<Vec<u8>>) -> Self {
        self.files.insert(path.to_owned(), bytes.into());

        self
    }
}

impl PackSource for MemorySource {
    fn label(&self) -> String {
        self.label.clone()
    }

    fn list(&self) -> io::Result<Listing> {
        let files = self
            .files
            .iter()
            .map(|(p, b)| FileEntry {
                path: p.clone(),
                size: b.len() as u64,
                fingerprint: mix(&[b.len() as u64]),
            })
            .collect();
        let rejected = Vec::new();

        Ok(Listing { files, rejected })
    }

    fn read(&self, path: &str) -> io::Result<Vec<u8>> {
        check_safe(path).map_err(invalid)?;

        self.files
            .get(path)
            .cloned()
            .ok_or_else(|| io::Error::from(io::ErrorKind::NotFound))
    }
}

pub struct FileEntry {
    pub path: String,
    pub size: u64,
    pub fingerprint: u64,
}

pub struct RejectedEntry {
    pub path: String,
    pub reason: PathError,
}

#[derive(Default)]
pub struct Listing {
    pub files: Vec<FileEntry>,
    pub rejected: Vec<RejectedEntry>,
}

fn rel_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
