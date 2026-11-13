//! Exact-name side index. Positions refer only to the in-memory Vec, never durable slot ids.
use crate::{Graph, Term};
use hashbrown::HashTable;
#[cfg(not(test))]
use std::collections::hash_map::RandomState as NameHashes;
#[cfg(test)]
type NameHashes = std::hash::BuildHasherDefault<std::collections::hash_map::DefaultHasher>;
use std::hash::BuildHasher;

pub(super) struct Positions {
    pub first: usize,
    // Only duplicate names allocate; add_named deliberately permits them.
    pub duplicates: Vec<usize>,
}

#[derive(Default)]
pub(super) struct NamedIndex {
    table: HashTable<Positions>,
    hashes: NameHashes,
}

impl NamedIndex {
    pub fn from_named(named: &[(Term, Graph)]) -> Self {
        let mut index = Self::default();
        for end in 1..=named.len() {
            index.append(&named[..end]);
        }
        index
    }

    pub fn positions(&self, named: &[(Term, Graph)], name: &Term) -> Option<&Positions> {
        self.table.find(self.hashes.hash_one(name), |p| {
            #[cfg(test)]
            COMPARISONS.with(|count| count.set(count.get() + 1));
            named[p.first].0 == *name
        })
    }

    pub fn append(&mut self, named: &[(Term, Graph)]) {
        let i = named.len() - 1;
        let hash = self.hashes.hash_one(&named[i].0);
        if let Some(p) = self
            .table
            .find_mut(hash, |p| named[p.first].0 == named[i].0)
        {
            p.duplicates.push(i);
        } else {
            self.table.insert_unique(
                hash,
                Positions {
                    first: i,
                    duplicates: Vec::new(),
                },
                |p| self.hashes.hash_one(&named[p.first].0),
            );
        }
    }

    pub fn heap_bytes(&self) -> usize {
        self.table.allocation_size()
            + self
                .table
                .iter()
                .map(|p| p.duplicates.capacity() * std::mem::size_of::<usize>())
                .sum::<usize>()
    }
}

#[cfg(test)]
thread_local! {
    pub(super) static COMPARISONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}
