use std::{
    any::{Any, TypeId},
    collections::HashMap,
    fmt::Display,
    sync::Arc,
};

use crate::{
    error::{AssetError, IdError},
    manifest::NetworkSide,
    parse::{Input, ParseCx},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layering {
    Override,
    Merge,
}

pub trait Asset: Send + Sync + 'static {
    const KIND: &'static str;
    const DIR: &'static str;
    const EXTENSIONS: &'static [&'static str];
    const SIDE: NetworkSide = NetworkSide::Both;
    const LAYERING: Layering = Layering::Override;

    fn nil() -> Self;

    fn parse(cx: &mut ParseCx, input: Input) -> Result<Self, AssetError>
    where
        Self: Sized;
}

pub(crate) struct KindEntry {
    pub kind: &'static str,
    pub dir: &'static str,
    pub extensions: &'static [&'static str],
    pub side: NetworkSide,
    pub layering: Layering,
    pub nil: AnyArc,
    pub parse: fn(&mut ParseCx, Input) -> Result<AnyArc, AssetError>,
}

pub(crate) type AnyArc = Arc<dyn Any + Send + Sync>;

#[derive(Clone, Copy, Debug)]
pub struct KindDesc {
    pub id: KindId,
    pub kind: &'static str,
    pub dir: &'static str,
    pub extensions: &'static [&'static str],
    pub side: NetworkSide,
    pub layering: Layering,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KindId(pub(crate) u16);

#[derive(Default)]
pub struct KindTable {
    entries: Vec<KindEntry>,
    by_type: HashMap<TypeId, KindId>,
}

impl KindTable {
    pub fn register<T: Asset>(&mut self) -> KindId {
        if let Some(&id) = self.by_type.get(&TypeId::of::<T>()) {
            return id;
        }
        assert!(
            is_valid_single(T::DIR),
            "asset folder `{}` is not a valid name",
            T::DIR
        );
        assert!(
            self.entries.iter().all(|e| e.kind != T::KIND),
            "asset kind `{}` registered by two different types",
            T::KIND
        );
        assert!(
            self.entries.iter().all(|e| e.dir != T::DIR),
            "asset folder `{}` is used by two kinds",
            T::DIR
        );
        let id = KindId(self.entries.len() as u16);
        self.entries.push(KindEntry {
            kind: T::KIND,
            dir: T::DIR,
            extensions: T::EXTENSIONS,
            side: T::SIDE,
            layering: T::LAYERING,
            nil: Arc::new(T::nil()),
            parse: parse_erased::<T>,
        });
        self.by_type.insert(TypeId::of::<T>(), id);

        id
    }

    pub fn by_dir(&self, dir: &str) -> Option<KindId> {
        self.entries
            .iter()
            .position(|e| e.dir == dir)
            .map(|i| KindId(i as u16))
    }

    pub fn id_of<T: Asset>(&self) -> Option<KindId> {
        self.by_type.get(&TypeId::of::<T>()).copied()
    }

    pub fn descriptions(&self) -> impl Iterator<Item = KindDesc> + '_ {
        self.entries.iter().enumerate().map(|(i, e)| KindDesc {
            id: KindId(i as u16),
            kind: e.kind,
            dir: e.dir,
            extensions: e.extensions,
            side: e.side,
            layering: e.layering,
        })
    }

    pub(crate) fn entry(&self, id: KindId) -> &KindEntry {
        &self.entries[id.0 as usize]
    }

    fn nil_entry<T: Asset>(&self) -> &AnyArc {
        let id = self
            .id_of::<T>()
            .unwrap_or_else(|| panic!("asset kind `{}` is not registered", T::KIND));

        &self.entry(id).nil
    }

    pub(crate) fn nil<T: Asset>(&self) -> &T {
        self.nil_entry::<T>()
            .downcast_ref()
            .expect("nil value has the registered type")
    }

    pub(crate) fn nil_arc<T: Asset>(&self) -> Arc<T> {
        self.nil_entry::<T>()
            .clone()
            .downcast()
            .unwrap_or_else(|_| unreachable!("nil value has the registered type"))
    }
}

fn parse_erased<T: Asset>(cx: &mut ParseCx, input: Input) -> Result<AnyArc, AssetError> {
    T::parse(cx, input).map(|v| Arc::new(v) as AnyArc)
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AssetId {
    namespace: Box<str>,
    identity: Box<str>,
}

impl AssetId {
    pub fn new(namespace: &str, identity: &str) -> Result<Self, IdError> {
        if !is_valid_single(namespace) {
            return Err(IdError::Namespace(namespace.to_owned()));
        }
        if !is_valid_multiple(identity) {
            return Err(IdError::Identity(identity.to_owned()));
        }

        Ok(Self {
            namespace: namespace.into(),
            identity: identity.into(),
        })
    }

    pub fn parse(s: &str) -> Result<Self, IdError> {
        match s.split_once(':') {
            Some((namespace, identity)) => Self::new(namespace, identity),
            None => Err(IdError::Format(s.to_owned())),
        }
    }

    pub fn nil() -> Self {
        Self {
            namespace: "core".into(),
            identity: "nil".into(),
        }
    }

    pub fn is_nil(&self) -> bool {
        &*self.namespace == "core" && &*self.identity == "nil"
    }

    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    pub fn identity(&self) -> &str {
        &self.identity
    }
}

impl Display for AssetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}:{}", self.namespace, self.identity)
    }
}

pub(crate) fn is_valid_single(id: &str) -> bool {
    let mut chars = id.chars();
    id.len() <= 32
        && chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'))
}

pub(crate) fn is_valid_multiple(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id.split('/').all(|seg| {
            !seg.is_empty()
                && seg
                    .chars()
                    .all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'))
        })
}
