// BlankNode has no Ord implementation. Keep the parser's membership semantics
// with a fixed-key hasher on the zkVM target, while native imports stay intact.
use std::collections::HashSet as Set;
use std::collections::hash_map::DefaultHasher;
use std::hash::{BuildHasherDefault, Hash};
use std::ops::{Deref, DerefMut};

#[derive(Clone)]
pub(crate) struct HashSet<T>(Set<T, BuildHasherDefault<DefaultHasher>>);

impl<T> Default for HashSet<T> {
    fn default() -> Self {
        Self(Set::with_hasher(BuildHasherDefault::default()))
    }
}

impl<T> HashSet<T> {
    pub fn new() -> Self {
        Self::default()
    }
}

impl<T> Deref for HashSet<T> {
    type Target = Set<T, BuildHasherDefault<DefaultHasher>>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl<T> DerefMut for HashSet<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

impl<T> IntoIterator for HashSet<T> {
    type Item = T;
    type IntoIter = std::collections::hash_set::IntoIter<T>;
    fn into_iter(self) -> Self::IntoIter {
        self.0.into_iter()
    }
}

impl<T: Eq + Hash> Extend<T> for HashSet<T> {
    fn extend<I: IntoIterator<Item = T>>(&mut self, iter: I) {
        self.0.extend(iter);
    }
}
