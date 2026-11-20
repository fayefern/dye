use std::{collections::BTreeMap, io, path::PathBuf};

use dye_opt::file::{Loaded, OptFile, load_str};
use semver::{Version, VersionReq};
use serde::{Deserialize, Serialize};

use crate::source::PackSource;

pub fn read_manifest(source: &dyn PackSource) -> io::Result<Loaded<PackManifest>> {
    let bytes = source.read(PackManifest::FILE_NAME)?;
    let text =
        String::from_utf8(bytes).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    let label = PathBuf::from(format!("{}!/{}", source.label(), PackManifest::FILE_NAME));

    Ok(load_str(&text, &label))
}

#[derive(Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PackManifest {
    pub meta: PackMeta,
    pub dependencies: BTreeMap<String, Dependency>,
    pub order: OrderOverride,
}

impl PackManifest {
    pub fn is_usable(&self) -> bool {
        is_valid_id(&self.meta.id)
    }
}

impl OptFile for PackManifest {
    const FILE_NAME: &'static str = "pack.toml";
    const VERSION: u32 = 1;

    fn validate(&mut self, issues: &mut dye_opt::warning::Issues) {
        if !is_valid_id(&self.meta.id) {
            issues.invalid(
                "meta.id",
                "must be 1..=32 chars of a..=z, 0..=9 and `_`, starting with a letter. skipped",
            );
        }
        if self.meta.name.trim().is_empty() {
            self.meta.name = self.meta.id.clone();
        }

        if self.dependencies.remove(&self.meta.id).is_some() {
            issues.invalid("dependencies", "a pack cannot depend on itself. removed");
        }
        for list in [&mut self.order.after, &mut self.order.before] {
            list.retain(|id| {
                let ok = is_valid_id(id) && *id != self.meta.id;
                if !ok {
                    issues.invalid("order", format!("`{id}` is not a valid pack id. removed"));
                }

                ok
            });
        }

        let bad: Vec<_> = self
            .dependencies
            .keys()
            .filter(|k| !is_valid_id(k))
            .cloned()
            .collect();
        for key in bad {
            self.dependencies.remove(&key);
            issues.invalid(
                "dependencies",
                format!("`{key}` is not a valid pack id. removed"),
            );
        }
    }
}

fn is_valid_id(id: &str) -> bool {
    let mut chars = id.chars();
    id.len() <= 32
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'))
}

#[derive(Serialize, Deserialize)]
#[serde(default)]
pub struct PackMeta {
    pub id: String,
    pub name: String,
    pub version: Version,
    pub description: String,
    pub authors: Vec<String>,
    pub side: NetworkSide,
}

impl Default for PackMeta {
    fn default() -> Self {
        Self {
            id: String::new(),
            name: String::new(),
            version: Version::new(0, 0, 0),
            description: String::new(),
            authors: Vec::new(),
            side: NetworkSide::Both,
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkSide {
    Client,
    Server,
    #[default]
    Both,
}

impl NetworkSide {
    pub fn accepts(self, other: NetworkSide) -> bool {
        self == Self::Both || other == Self::Both || self == other
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(untagged)]
pub enum Dependency {
    Req(VersionReq),
    Detailed {
        #[serde(default = "star")]
        version: VersionReq,
        #[serde(default)]
        optional: bool,
    },
}

fn star() -> VersionReq {
    VersionReq::STAR
}

impl Dependency {
    pub fn req(&self) -> &VersionReq {
        match self {
            Self::Req(r) | Self::Detailed { version: r, .. } => r,
        }
    }

    pub fn optional(&self) -> bool {
        matches!(self, Self::Detailed { optional: true, .. })
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct OrderOverride {
    pub after: Vec<String>,
    pub before: Vec<String>,
}
