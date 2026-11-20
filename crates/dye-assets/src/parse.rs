use std::{
    collections::HashMap,
    mem::take,
    sync::{Arc, Mutex, PoisonError},
};

use crate::{
    asset::{Asset, AssetId, KindId, KindTable},
    error::{AssetDiag, AssetError, Problem},
    handle::Handle,
};

pub struct InputEntry {
    pub pack: Arc<str>,
    pub sub_path: Arc<str>,
    pub bytes: Vec<u8>,
}

impl InputEntry {
    pub fn ext(&self) -> &str {
        self.sub_path.rsplit_once('.').map_or("", |(_, e)| e)
    }

    pub fn text(&self) -> Result<&str, AssetError> {
        std::str::from_utf8(&self.bytes)
            .map_err(|e| AssetError::Parse(format!("not valid UTF-8: {e}")))
    }
}

pub struct Input {
    pub entries: Vec<InputEntry>,
}

impl Input {
    pub fn top(&self) -> &InputEntry {
        self.entries
            .last()
            .expect("the loader never passes empty input")
    }
}

pub struct ParseCx<'a> {
    shared: &'a Shared,
    id: &'a AssetId,
    kind: &'static str,
    deps: Vec<u32>,
}

impl<'a> ParseCx<'a> {
    pub(crate) fn new(shared: &'a Shared, id: &'a AssetId, kind: &'static str) -> Self {
        Self {
            shared,
            id,
            kind,
            deps: Vec::new(),
        }
    }

    pub fn id(&self) -> &AssetId {
        self.id
    }

    pub fn handle<T: Asset>(&mut self, raw: &str) -> Handle<T> {
        let h = self.shared.handle::<T>(raw);
        self.deps.push(h.slot);

        h
    }

    pub fn warn(&self, msg: impl Into<String>) {
        self.shared
            .diag(self.kind, self.id.to_string(), Problem::Warning(msg.into()));
    }

    pub(crate) fn into_deps(mut self) -> Vec<u32> {
        self.deps.sort_unstable();
        self.deps.dedup();

        self.deps
    }
}

#[derive(Default)]
struct Interner {
    map: HashMap<(KindId, AssetId), u32>,
    entries: Vec<(KindId, AssetId)>,
}

pub(crate) struct Shared {
    pub kinds: KindTable,
    interner: Mutex<Interner>,
    diags: Mutex<Vec<AssetDiag>>,
}

impl Shared {
    pub fn new(kinds: KindTable) -> Self {
        Self {
            kinds,
            interner: Mutex::default(),
            diags: Mutex::default(),
        }
    }

    fn intern(&self, kind: KindId, id: AssetId) -> u32 {
        let mut g = self.interner.lock().unwrap_or_else(PoisonError::into_inner);
        let key = (kind, id);
        if let Some(&i) = g.map.get(&key) {
            return i;
        }
        let i = g.entries.len() as u32;
        g.entries.push(key.clone());
        g.map.insert(key, i);

        i
    }

    pub fn handle<T: Asset>(&self, raw: &str) -> Handle<T> {
        let kind = self
            .kinds
            .id_of::<T>()
            .unwrap_or_else(|| panic!("asset kind `{}` is not registered", T::KIND));
        let id = AssetId::parse(raw).unwrap_or_else(|e| {
            self.diag(T::KIND, raw.to_owned(), Problem::BadId(e.to_string()));
            AssetId::nil()
        });

        Handle::new(self.intern(kind, id))
    }

    pub fn diag(&self, kind: &'static str, subject: String, problem: Problem) {
        self.diags
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(AssetDiag {
                kind,
                subject,
                problem,
            });
    }

    pub fn entries_from(&self, from: usize) -> Vec<(KindId, AssetId)> {
        let g = self.interner.lock().unwrap_or_else(PoisonError::into_inner);

        g.entries.get(from..).unwrap_or_default().to_vec()
    }

    pub fn drain_diags(&self) -> Vec<AssetDiag> {
        take(&mut *self.diags.lock().unwrap_or_else(PoisonError::into_inner))
    }

    pub fn lookup(&self, kind: KindId, id: &AssetId) -> Option<u32> {
        let g = self.interner.lock().unwrap_or_else(PoisonError::into_inner);

        g.map.get(&(kind, id.clone())).copied()
    }
}
