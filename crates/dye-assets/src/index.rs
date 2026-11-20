use std::{collections::HashMap, sync::Arc};

use crate::{
    asset::{AssetId, KindId, KindTable, Layering},
    source::PackSource,
};

#[derive(Clone)]
pub struct Layer {
    pub pack: Arc<str>,
    pub source: Arc<dyn PackSource>,
    pub sub_path: Arc<str>,
    pub fingerprint: u64,
}

impl Layer {
    fn same_file(&self, other: &Layer) -> bool {
        self.pack == other.pack
            && self.sub_path == other.sub_path
            && self.fingerprint == other.fingerprint
    }
}

#[derive(Default)]
pub struct AssetIndex {
    by_kind: Vec<HashMap<AssetId, Vec<Layer>>>,
}

impl AssetIndex {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, kind: KindId, id: AssetId, layer: Layer) {
        let k = kind.0 as usize;
        if self.by_kind.len() <= k {
            self.by_kind.resize_with(k + 1, HashMap::new);
        }
        self.by_kind[k].entry(id).or_default().push(layer);
    }

    pub fn layers(&self, kind: KindId, id: &AssetId) -> &[Layer] {
        self.by_kind
            .get(kind.0 as usize)
            .and_then(|m| m.get(id))
            .map_or(&[], Vec::as_slice)
    }

    pub fn iter_kind(&self, kind: KindId) -> impl Iterator<Item = (&AssetId, &[Layer])> {
        self.by_kind
            .get(kind.0 as usize)
            .into_iter()
            .flatten()
            .map(|(id, layer)| (id, layer.as_slice()))
    }

    pub(crate) fn diff(&self, new: &AssetIndex, kinds: &KindTable) -> Vec<(KindId, AssetId)> {
        let empty = HashMap::new();
        let mut out = Vec::new();

        for k in 0..self.by_kind.len().max(new.by_kind.len()) {
            let kind = KindId(k as u16);
            let layering = kinds.entry(kind).layering;
            let old = self.by_kind.get(k).unwrap_or(&empty);
            let curr = new.by_kind.get(k).unwrap_or(&empty);

            let ids = old
                .keys()
                .chain(curr.keys().filter(|id| !old.contains_key(*id)));
            for id in ids {
                let a = effective(old.get(id), layering);
                let b = effective(curr.get(id), layering);
                if a.len() != b.len() || !a.iter().zip(b).all(|(x, y)| x.same_file(y)) {
                    out.push((kind, id.clone()));
                }
            }
        }

        out
    }
}

fn effective(layers: Option<&Vec<Layer>>, layering: Layering) -> &[Layer] {
    let all = layers.map_or(&[][..], Vec::as_slice);

    match layering {
        Layering::Override => &all[all.len().saturating_sub(1)..],
        Layering::Merge => all,
    }
}
