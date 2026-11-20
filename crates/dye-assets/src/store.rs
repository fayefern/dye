use std::{
    collections::HashSet,
    panic::{self, AssertUnwindSafe},
    sync::Arc,
    time::Duration,
};

use crossbeam_channel::{Receiver, Sender, unbounded};
use dye_core::JobPool;

use crate::{
    asset::{AnyArc, Asset, AssetId, KindId, KindTable, Layering},
    error::{AssetDiag, AssetError, Problem},
    handle::{AnyHandle, Handle},
    index::{AssetIndex, Layer},
    parse::{Input, InputEntry, ParseCx, Shared},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum State {
    Unloaded,
    Loading,
    Ready,
    Missing,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScopeId(u32);

struct Scope {
    name: String,
    held: HashSet<u32>,
}

struct Slot {
    kind: KindId,
    id: AssetId,
    state: State,
    value: Option<AnyArc>,
    version: u32,
    holders: u32,
    deps: Vec<u32>,
    job: u32,
    pinned: bool,
}

struct Loaded {
    value: AnyArc,
    deps: Vec<u32>,
}

struct Done {
    slot: u32,
    job: u32,
    result: Result<Loaded, AssetError>,
}

#[derive(Default)]
pub struct Changes {
    list: Vec<AnyHandle>,
}

impl Changes {
    pub fn iter(&self) -> impl Iterator<Item = AnyHandle> + '_ {
        self.list.iter().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.list.is_empty()
    }

    pub fn contains<T>(&self, h: Handle<T>) -> bool {
        self.list.iter().any(|c| c.slot == h.slot)
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Progress {
    pub settled: usize,
    pub total: usize,
}

impl Progress {
    pub fn is_done(&self) -> bool {
        self.settled == self.total
    }

    pub fn fraction(&self) -> f32 {
        if self.total == 0 {
            1.0
        } else {
            self.settled as f32 / self.total as f32
        }
    }
}

pub struct Assets {
    shared: Arc<Shared>,
    index: Arc<AssetIndex>,
    pool: Arc<JobPool>,
    slots: Vec<Slot>,
    scopes: Vec<Option<Scope>>,
    tx: Sender<Done>,
    rx: Receiver<Done>,
    inflight: usize,
    changes: Changes,
}

impl Assets {
    pub fn new(kinds: KindTable, index: Arc<AssetIndex>, pool: Arc<JobPool>) -> Self {
        let (tx, rx) = unbounded();

        Self {
            shared: Arc::new(Shared::new(kinds)),
            index,
            pool,
            slots: Vec::new(),
            scopes: Vec::new(),
            tx,
            rx,
            inflight: 0,
            changes: Changes::default(),
        }
    }

    pub fn handle<T: Asset>(&mut self, id: &str) -> Handle<T> {
        let h = self.shared.handle::<T>(id);
        self.sync();

        h
    }

    pub fn get<T: Asset>(&self, h: Handle<T>) -> &T {
        self.slots
            .get(h.slot as usize)
            .and_then(|s| s.value.as_deref())
            .and_then(|v| v.downcast_ref::<T>())
            .unwrap_or_else(|| self.shared.kinds.nil::<T>())
    }

    pub fn get_arc<T: Asset>(&self, h: Handle<T>) -> Arc<T> {
        self.slots
            .get(h.slot as usize)
            .and_then(|s| s.value.clone())
            .and_then(|v| v.downcast::<T>().ok())
            .unwrap_or_else(|| self.shared.kinds.nil_arc::<T>())
    }

    pub fn state<T>(&self, h: Handle<T>) -> State {
        self.slots
            .get(h.slot as usize)
            .map_or(State::Unloaded, |s| s.state)
    }

    pub fn id<T>(&self, h: Handle<T>) -> Option<&AssetId> {
        self.slots.get(h.slot as usize).map(|s| &s.id)
    }

    pub fn version<T>(&self, h: Handle<T>) -> u32 {
        self.slots.get(h.slot as usize).map_or(0, |s| s.version)
    }

    pub fn scope(&mut self, name: impl Into<String>) -> ScopeId {
        self.scopes.push(Some(Scope {
            name: name.into(),
            held: HashSet::new(),
        }));

        ScopeId(self.scopes.len() as u32 - 1)
    }

    pub fn scope_name(&self, scope: ScopeId) -> Option<&str> {
        match self.scopes.get(scope.0 as usize) {
            Some(Some(s)) => Some(&s.name),
            _ => None,
        }
    }

    pub fn require<T: Asset>(&mut self, scope: ScopeId, h: Handle<T>) {
        self.sync();
        let Some(Some(sc)) = self.scopes.get_mut(scope.0 as usize) else {
            return;
        };
        if sc.held.insert(h.slot) {
            self.acquire(h.slot);
        }
    }

    pub fn release_scope(&mut self, scope: ScopeId) {
        let Some(sc) = self.scopes.get_mut(scope.0 as usize).and_then(Option::take) else {
            return;
        };
        for slot in sc.held {
            self.release(slot);
        }
    }

    pub fn progress(&self, scope: ScopeId) -> Progress {
        let Some(Some(sc)) = self.scopes.get(scope.0 as usize) else {
            return Progress {
                settled: 0,
                total: 0,
            };
        };
        let mut seen = HashSet::new();
        let mut stack: Vec<u32> = sc.held.iter().copied().collect();
        let mut settled = 0;

        while let Some(i) = stack.pop() {
            if !seen.insert(i) {
                continue;
            }
            let s = &self.slots[i as usize];
            if !matches!(s.state, State::Loading | State::Unloaded) {
                settled += 1;
            }
            stack.extend(&s.deps);
        }

        Progress {
            settled,
            total: seen.len(),
        }
    }

    pub fn commit(&mut self) -> &Changes {
        self.changes.list.clear();
        self.pump(None);

        &self.changes
    }

    pub fn wait(&mut self, scope: ScopeId) {
        self.changes.list.clear();
        loop {
            if self.progress(scope).is_done() || self.inflight == 0 {
                break;
            }
            self.pump(Some(Duration::from_millis(10)));
        }
    }

    pub fn changed<T: Asset>(&self) -> impl Iterator<Item = Handle<T>> + '_ {
        let kind = self.shared.kinds.id_of::<T>();

        self.changes
            .list
            .iter()
            .filter(move |c| Some(c.kind) == kind)
            .map(|c| Handle::new(c.slot))
    }

    pub fn take_diagnostics(&self) -> Vec<AssetDiag> {
        self.shared.drain_diags()
    }

    fn sync(&mut self) {
        for (kind, id) in self.shared.entries_from(self.slots.len()) {
            let pinned = id.is_nil();
            self.slots.push(Slot {
                kind,
                id,
                state: if pinned {
                    State::Ready
                } else {
                    State::Unloaded
                },
                value: None,
                version: 0,
                holders: 0,
                deps: Vec::new(),
                job: 0,
                pinned,
            });
        }
    }

    fn pump(&mut self, wait: Option<Duration>) {
        self.sync();
        if let Some(t) = wait
            && self.inflight > 0
            && let Ok(done) = self.rx.recv_timeout(t)
        {
            self.apply(done);
        }
        while let Ok(done) = self.rx.try_recv() {
            self.apply(done);
        }
    }

    fn acquire(&mut self, idx: u32) {
        let slot = &mut self.slots[idx as usize];
        slot.holders += 1;
        if slot.state == State::Unloaded {
            self.start(idx);
        }
    }

    fn start(&mut self, idx: u32) {
        let slot = &mut self.slots[idx as usize];
        let entry = self.shared.kinds.entry(slot.kind);
        let layers = self.index.layers(slot.kind, &slot.id);
        if layers.is_empty() {
            slot.state = State::Missing;
            self.shared
                .diag(entry.kind, slot.id.to_string(), Problem::Missing);
            return;
        }
        let picked: Vec<Layer> = match entry.layering {
            Layering::Override => layers.last().into_iter().cloned().collect(),
            Layering::Merge => layers.to_vec(),
        };

        slot.state = State::Loading;
        slot.job += 1;
        let (kind, id, job) = (slot.kind, slot.id.clone(), slot.job);
        let (shared, tx) = (self.shared.clone(), self.tx.clone());
        self.inflight += 1;
        self.pool.spawn(move || {
            let result = load(&shared, kind, &id, picked);
            let _ = tx.send(Done {
                slot: idx,
                job,
                result,
            });
        });
    }

    fn release(&mut self, idx: u32) {
        let mut stack = vec![idx];
        while let Some(i) = stack.pop() {
            let slot = &mut self.slots[i as usize];
            slot.holders = slot.holders.saturating_sub(1);
            if slot.holders > 0 || slot.pinned {
                continue;
            }
            slot.job += 1;
            let had_value = slot.value.take().is_some();
            slot.state = State::Unloaded;
            stack.append(&mut slot.deps);
            if had_value {
                slot.version += 1;
                self.changes.list.push(AnyHandle {
                    slot: i,
                    kind: slot.kind,
                });
            }
        }
    }

    fn apply(&mut self, done: Done) {
        self.sync();
        self.inflight -= 1;
        let idx = done.slot;
        let slot = &mut self.slots[idx as usize];
        if slot.job != done.job {
            return;
        }

        match done.result {
            Ok(Loaded { value, deps }) => {
                let old = std::mem::replace(&mut slot.deps, deps.clone());
                slot.value = Some(value);
                slot.state = State::Ready;
                slot.version += 1;
                self.changes.list.push(AnyHandle {
                    slot: idx,
                    kind: slot.kind,
                });
                for &d in &deps {
                    if !old.contains(&d) {
                        self.acquire(d);
                    }
                }
                for &d in &old {
                    if !deps.contains(&d) {
                        self.release(d);
                    }
                }
            }
            Err(e) => {
                let name = self.shared.kinds.entry(slot.kind).kind;
                self.shared
                    .diag(name, slot.id.to_string(), Problem::Failed(e));
                slot.state = State::Failed;
                let had_value = slot.value.take().is_some();
                let old = std::mem::take(&mut slot.deps);
                if had_value {
                    slot.version += 1;
                    self.changes.list.push(AnyHandle {
                        slot: idx,
                        kind: slot.kind,
                    });
                }
                for d in old {
                    self.release(d);
                }
            }
        }
    }
}

fn load(
    shared: &Shared,
    kind: KindId,
    id: &AssetId,
    layers: Vec<Layer>,
) -> Result<Loaded, AssetError> {
    let mut files = Vec::with_capacity(layers.len());
    for l in layers {
        let bytes = l.source.read(&l.sub_path).map_err(|e| AssetError::Io {
            path: format!("{}!/{}", l.source.label(), l.sub_path),
            message: e.to_string(),
        })?;
        files.push(InputEntry {
            pack: l.pack,
            sub_path: l.sub_path,
            bytes,
        });
    }

    let entry = shared.kinds.entry(kind);
    let mut cx = ParseCx::new(shared, id, entry.kind);
    // Modder-supplied data must never take a worker (or the game) down.
    let parsed = panic::catch_unwind(AssertUnwindSafe(|| {
        (entry.parse)(&mut cx, Input { entries: files })
    }));
    let value = parsed.unwrap_or(Err(AssetError::Panic))?;

    Ok(Loaded {
        value,
        deps: cx.into_deps(),
    })
}
