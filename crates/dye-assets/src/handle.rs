use std::{fmt::Debug, hash::Hash, marker::PhantomData};

use crate::asset::KindId;

pub struct Handle<T> {
    pub slot: u32,
    _marker: PhantomData<fn() -> T>,
}

impl<T> Handle<T> {
    pub(crate) const fn new(slot: u32) -> Self {
        Self {
            slot,
            _marker: PhantomData,
        }
    }
}

impl<T> Clone for Handle<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Handle<T> {}

impl<T> PartialEq for Handle<T> {
    fn eq(&self, other: &Self) -> bool {
        self.slot == other.slot
    }
}

impl<T> Eq for Handle<T> {}

impl<T> Hash for Handle<T> {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.slot.hash(state);
    }
}

impl<T> Debug for Handle<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_tuple("Handle").field(&self.slot).finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct AnyHandle {
    pub slot: u32,
    pub kind: KindId,
}
