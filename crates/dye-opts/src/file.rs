use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

use serde::{Serialize, de::DeserializeOwned};

use crate::warning::{Issues, OptWarning, WarningKind};

pub trait OptFile: Serialize + DeserializeOwned + Default {
    const FILE_NAME: &'static str;
    const VERSION: u32;
    const TEMPLATE: Option<&'static str> = None;

    fn migrate(_table: &mut toml::Table, _from_version: u32) {}

    fn validate(&mut self, _issues: &mut Issues) {}
}

pub struct Loaded<T> {
    pub value: T,
    pub path: PathBuf,
    pub existed: bool,
    pub warnings: Vec<OptWarning>,
}

impl<T: OptFile> Loaded<T> {
    pub fn revalidate(&mut self) {
        let mut issues = Issues::default();
        self.value.validate(&mut issues);
        let path = self.path.clone();
        self.warnings
            .extend(issues.0.into_iter().map(|(field, reason)| OptWarning {
                path: path.clone(),
                kind: WarningKind::Invalid { field, reason },
            }));
    }
}

pub fn load<T: OptFile>(dir: &Path) -> Loaded<T> {
    let path = dir.join(T::FILE_NAME);
    let mut kinds = Vec::new();

    let (value, existed) = match fs::read_to_string(&path) {
        Ok(text) => (parse::<T>(&text, &mut kinds), true),
        Err(e) if e.kind() == io::ErrorKind::NotFound => (T::default(), false),
        Err(e) => {
            kinds.push(WarningKind::Io(e.to_string()));
            (T::default(), true)
        }
    };

    finish(&path, value, kinds, existed)
}

pub fn load_str<T: OptFile>(text: &str, label: &Path) -> Loaded<T> {
    let mut kinds = Vec::new();
    let value = parse::<T>(text, &mut kinds);

    finish(label, value, kinds, true)
}

fn parse<T: OptFile>(text: &str, kinds: &mut Vec<WarningKind>) -> T {
    let mut table: toml::Table = match text.parse() {
        Ok(t) => t,
        Err(e) => {
            let line = e.span().map(|s| text[..s.start].matches('\n').count() + 1);
            kinds.push(WarningKind::Syntax {
                line,
                message: e.message().to_string(),
            });
            return T::default();
        }
    };

    let version = table
        .remove("version")
        .and_then(|v| v.as_integer())
        .map_or(1, |v| v.max(1) as u32);
    if version > T::VERSION {
        kinds.push(WarningKind::NewerVersion {
            found: version,
            supported: T::VERSION,
        })
    }
    for from in version..T::VERSION {
        T::migrate(&mut table, from);
    }

    let (value, unknown) = match deserialize::<T>(&table) {
        Ok(r) => r,
        Err(_) => salvage::<T>(table, kinds),
    };
    kinds.extend(unknown.into_iter().map(WarningKind::UnknownKey));

    value
}

fn deserialize<T: OptFile>(table: &toml::Table) -> Result<(T, Vec<String>), toml::de::Error> {
    let mut unknown = Vec::new();
    let value = serde_ignored::deserialize(toml::Value::Table(table.clone()), |p| {
        unknown.push(p.to_string())
    })?;

    Ok((value, unknown))
}

fn salvage<T: OptFile>(table: toml::Table, kinds: &mut Vec<WarningKind>) -> (T, Vec<String>) {
    let mut good = toml::Table::new();
    for (key, val) in table {
        let mut probe = toml::Table::new();
        probe.insert(key.clone(), val.clone());
        match deserialize::<T>(&probe) {
            Ok(_) => {
                good.insert(key, val);
            }
            Err(e) => kinds.push(WarningKind::BadSection {
                section: key,
                message: e.message().to_string(),
            }),
        }
    }

    deserialize::<T>(&good).unwrap_or_default()
}

fn finish<T: OptFile>(
    path: &Path,
    mut value: T,
    mut kinds: Vec<WarningKind>,
    existed: bool,
) -> Loaded<T> {
    let mut issues = Issues::default();
    value.validate(&mut issues);
    kinds.extend(
        issues
            .0
            .into_iter()
            .map(|(field, reason)| WarningKind::Invalid { field, reason }),
    );

    let buf = path.to_path_buf();
    let warnings = kinds
        .into_iter()
        .map(|kind| OptWarning {
            path: buf.clone(),
            kind,
        })
        .collect();

    Loaded {
        value,
        path: buf,
        existed,
        warnings,
    }
}

pub fn save<T: OptFile>(dir: &Path, value: &T) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    atomic_write(&dir.join(T::FILE_NAME), render(value).as_bytes())
}

pub fn render<T: OptFile>(value: &T) -> String {
    let body = toml::to_string_pretty(value).unwrap_or_default();
    format!("version = {}\n\n{body}", T::VERSION)
}

pub fn write_default<T: OptFile>(dir: &Path) -> io::Result<()> {
    match T::TEMPLATE {
        Some(t) => {
            fs::create_dir_all(dir)?;
            atomic_write(&dir.join(T::FILE_NAME), t.as_bytes())
        }
        None => save(dir, &T::default()),
    }
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = path.with_extension("tmp");
    {
        let mut f = fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
    }

    fs::rename(&tmp, path)
}

pub fn parse_value<T: DeserializeOwned>(s: &str) -> Result<T, String> {
    toml::Value::String(s.trim().to_lowercase())
        .try_into()
        .map_err(|e| e.message().to_string())
}
