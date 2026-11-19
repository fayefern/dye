use std::{collections::HashMap, sync::Arc};

use crate::{
    asset::{AssetId, KindId},
    source::PackSource,
};

#[derive(Clone)]
pub struct Layer {
    pub pack: Arc<str>,
    pub source: Arc<dyn PackSource>,
    pub sub_path: Arc<str>,
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
}
