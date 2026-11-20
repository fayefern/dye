use std::{
    collections::{BTreeSet, HashMap, HashSet, hash_map::Entry},
    fs, io,
    mem::replace,
    path::Path,
    sync::Arc,
};

use semver::Version;

use crate::{
    asset::{AssetId, KindId, KindTable, is_valid_single},
    error::{AssetDiag, Problem},
    index::{AssetIndex, Layer},
    manifest::{NetworkSide, PackManifest, read_manifest},
    source::{DirSource, PackSource, ZipSource, check_name},
};

pub struct PackInfo {
    pub id: Arc<str>,
    pub name: String,
    pub version: Version,
    pub label: String,
}

pub struct Discovered {
    pub index: AssetIndex,
    pub packs: Vec<PackInfo>,
    pub diags: Vec<AssetDiag>,
}

struct Candidate {
    manifest: PackManifest,
    source: Arc<dyn PackSource>,
}

fn warn(subject: impl Into<String>, msg: impl Into<String>) -> AssetDiag {
    AssetDiag {
        kind: "pack",
        subject: subject.into(),
        problem: Problem::Warning(msg.into()),
    }
}

pub fn scan_packs_dir(dir: &Path) -> (Vec<Arc<dyn PackSource>>, Vec<AssetDiag>) {
    let mut sources: Vec<Arc<dyn PackSource>> = Vec::new();
    let mut diags = Vec::new();

    let read = match fs::read_dir(dir) {
        Ok(r) => r,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return (sources, diags),
        Err(e) => {
            diags.push(warn(
                dir.display().to_string(),
                format!("cannot read packs folder: {e}"),
            ));
            return (sources, diags);
        }
    };
    let mut paths: Vec<_> = read.filter_map(Result::ok).map(|e| e.path()).collect();
    paths.sort();

    for path in paths {
        let hidden = path
            .file_name()
            .and_then(|n| n.to_str())
            .is_none_or(|n| n.starts_with('.'));
        if hidden {
            continue;
        }
        if path.is_dir() {
            sources.push(Arc::new(DirSource::new(&path)));
        } else if path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("zip"))
        {
            match ZipSource::open(&path) {
                Ok(z) => sources.push(Arc::new(z)),
                Err(e) => diags.push(warn(
                    path.display().to_string(),
                    format!("cannot open zip: {e}"),
                )),
            }
        }
    }

    (sources, diags)
}

fn load_candidate(
    source: Arc<dyn PackSource>,
    side: NetworkSide,
    diags: &mut Vec<AssetDiag>,
) -> Option<Candidate> {
    let label = source.label();
    let loaded = match read_manifest(&*source) {
        Ok(l) => l,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let wrapped = source.list().ok().and_then(|l| {
                l.files.into_iter().find_map(|f| {
                    let (dir, name) = f.path.split_once('/')?;
                    (name == "pack.toml").then(|| dir.to_owned())
                })
            });
            diags.push(warn(label, match wrapped {
                Some(dir) => format!("no pack.toml at the root, but found `{dir}/pack.toml`; zip the folder's contents, not the folder; skipped"), None => "no pack.toml at the root; skipped".to_owned()
            }));
            return None;
        }
        Err(e) => {
            diags.push(warn(label, format!("could not read pack.toml: {e}")));
            return None;
        }
    };

    diags.extend(
        loaded
            .warnings
            .iter()
            .map(|w| warn(w.path.display().to_string(), w.kind.to_string())),
    );
    let manifest = loaded.value;
    if !manifest.is_usable() || !side.accepts(manifest.meta.side) {
        return None;
    }

    Some(Candidate { manifest, source })
}

fn index_pack(
    kinds: &KindTable,
    side: NetworkSide,
    c: &Candidate,
    index: &mut AssetIndex,
    diags: &mut Vec<AssetDiag>,
) {
    let label = c.source.label();
    let pack: Arc<str> = c.manifest.meta.id.as_str().into();
    let listing = match c.source.list() {
        Ok(l) => l,
        Err(e) => {
            diags.push(warn(label, format!("could not list files: {e}")));
            return;
        }
    };
    diags.extend(
        listing
            .rejected
            .iter()
            .map(|r| warn(&label, format!("`{}`: {}", r.path, r.reason))),
    );

    let mut noted: HashSet<String> = HashSet::new();
    let mut picked: HashMap<(KindId, AssetId), (usize, Layer)> = HashMap::new();

    for f in &listing.files {
        let mut parts = f.path.splitn(3, '/');
        let (Some(ns), Some(kd), Some(iy)) = (parts.next(), parts.next(), parts.next()) else {
            continue;
        };
        if !is_valid_single(ns) {
            if noted.insert(ns.to_owned()) {
                diags.push(warn(&label, format!("`{ns}` is not a valid namespace folder (a..=z, 0..=9, `_`, starting with a letter); ignored")));
            }
            continue;
        }
        let Some(kind) = kinds.by_dir(kd) else {
            if noted.insert(format!("{ns}/{kd}")) {
                diags.push(warn(
                    &label,
                    format!("`{ns}/{kd}` is not a known asset folder; ignored"),
                ));
            }
            continue;
        };
        let entry = kinds.entry(kind);
        if !side.accepts(entry.side) {
            continue;
        }
        if let Err(e) = check_name(&f.path) {
            diags.push(warn(&label, format!("`{}`: {e}", f.path)));
            continue;
        }
        let Some((stem, ext)) = iy.rsplit_once('.') else {
            diags.push(warn(&label, format!("`{}`: file has no extension", f.path)));
            continue;
        };
        let Some(rank) = entry.extensions.iter().position(|x| *x == ext) else {
            diags.push(warn(
                &label,
                format!("`{}`: unsupported {} format `.{ext}`", f.path, entry.kind),
            ));
            continue;
        };
        let id = match AssetId::new(ns, stem) {
            Ok(id) => id,
            Err(e) => {
                diags.push(warn(&label, format!("`{}`: {e}", f.path)));
                continue;
            }
        };
        if id.is_nil() {
            diags.push(warn(
                &label,
                format!("`{}`: `{id}` is builtin and cannot be overridden", f.path),
            ));
            continue;
        }

        let layer = Layer {
            pack: pack.clone(),
            source: c.source.clone(),
            sub_path: f.path.as_str().into(),
        };
        match picked.entry((kind, id)) {
            Entry::Vacant(v) => {
                v.insert((rank, layer));
            }
            Entry::Occupied(mut o) => {
                let new_wins = rank < o.get().0;
                let lost = if new_wins {
                    o.get().1.sub_path.clone()
                } else {
                    layer.sub_path.clone()
                };
                diags.push(warn(
                    &label,
                    format!(
                        "`{lost}` ignored: another {} format of `{}` exists in this pack",
                        entry.kind,
                        o.key().1
                    ),
                ));
                if new_wins {
                    o.insert((rank, layer));
                }
            }
        }
    }

    for ((kind, id), (_, layer)) in picked {
        index.insert(kind, id, layer);
    }
}

pub fn discover(
    kinds: &KindTable,
    builtin: Arc<dyn PackSource>,
    others: Vec<Arc<dyn PackSource>>,
    side: NetworkSide,
) -> Discovered {
    let mut diags = Vec::new();
    let builtin = load_candidate(builtin, side, &mut diags);
    let reserved = builtin
        .as_ref()
        .map(|c| (c.manifest.meta.id.clone(), c.manifest.meta.version.clone()));

    let mut found: Vec<Candidate> = Vec::new();
    for source in others {
        let Some(c) = load_candidate(source, side, &mut diags) else {
            continue;
        };
        let id = &c.manifest.meta.id;
        if reserved.as_ref().is_some_and(|(r, _)| r == id) {
            diags.push(warn(
                c.source.label(),
                format!("pack id `{id}` is reserved by the builtin pack; skipped"),
            ));
            continue;
        }
        match found.iter().position(|f| f.manifest.meta.id == *id) {
            None => found.push(c),
            Some(i) => {
                let newer = c.manifest.meta.version > found[i].manifest.meta.version;
                let loser = if newer { replace(&mut found[i], c) } else { c };
                diags.push(warn(
                    loser.source.label(),
                    format!(
                        "duplicate pack id `{}`; the higher version is used, then the first found",
                        loser.manifest.meta.id
                    ),
                ));
            }
        }
    }
    found.sort_by(|a, b| a.manifest.meta.id.cmp(&b.manifest.meta.id));

    loop {
        let mut have: HashMap<String, Version> = found
            .iter()
            .map(|c| (c.manifest.meta.id.clone(), c.manifest.meta.version.clone()))
            .collect();
        have.extend(reserved.clone());

        let before = found.len();
        found.retain(|c| {
            let id = &c.manifest.meta.id;
            for (dep, spec) in &c.manifest.dependencies {
                if spec.optional() {
                    continue;
                }
                let problem = match have.get(dep) {
                    Some(v) if spec.req().matches(v) => continue,
                    Some(v) => format!("needs `{dep}` {} but {v} is installed", spec.req()),
                    None => format!("needs `{dep}` {} but none is installed", spec.req()),
                };
                diags.push(warn(
                    c.source.label(),
                    format!("pack `{id}` {problem}; skipped"),
                ));
                return false;
            }
            true
        });
        if found.len() == before {
            break;
        }
    }

    let n = found.len();
    let by_id: HashMap<String, usize> = found
        .iter()
        .enumerate()
        .map(|(i, c)| (c.manifest.meta.id.clone(), i))
        .collect();
    let mut succ: Vec<Vec<usize>> = vec![Vec::new(); n];
    let mut indegree = vec![0usize; n];

    for (i, c) in found.iter().enumerate() {
        let m = &c.manifest;
        let first = m
            .dependencies
            .keys()
            .chain(&m.order.after)
            .filter_map(|d| by_id.get(d));
        let last = m.order.before.iter().filter_map(|d| by_id.get(d));

        for &j in first {
            succ[j].push(i);
            indegree[i] += 1;
        }
        for &j in last {
            succ[i].push(j);
            indegree[j] += 1;
        }
    }
    let mut ready: BTreeSet<usize> = (0..n).filter(|&i| indegree[i] == 0).collect();
    let mut order = Vec::with_capacity(n);

    while let Some(i) = ready.pop_first() {
        order.push(i);

        for &s in &succ[i] {
            indegree[s] -= 1;
            if indegree[s] == 0 {
                ready.insert(s);
            }
        }
    }
    let mut ordered: Vec<Candidate> = builtin.into_iter().collect();
    let mut slots: Vec<Option<Candidate>> = found.into_iter().map(Some).collect();

    for &i in &order {
        ordered.push(slots[i].take().expect("each pack is emitted once"));
    }
    for c in slots.into_iter().flatten() {
        diags.push(warn(
            c.source.label(),
            format!(
                "pack `{}` is part of a dependency or order cycle; skipped",
                c.manifest.meta.id
            ),
        ));
    }

    let mut index = AssetIndex::new();
    let mut packs = Vec::with_capacity(ordered.len());

    for c in &ordered {
        index_pack(kinds, side, c, &mut index, &mut diags);
        packs.push(PackInfo {
            id: c.manifest.meta.id.as_str().into(),
            name: c.manifest.meta.name.clone(),
            version: c.manifest.meta.version.clone(),
            label: c.source.label(),
        });
    }

    Discovered {
        index,
        packs,
        diags,
    }
}
