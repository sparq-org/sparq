//! Persistent planner statistics built by an explicit ANALYZE operation.
//!
//! The catalog is deliberately separate from the graph files: [`analyze`] writes
//! `stats.bin` beside a saved/mmap store, and [`StatsCatalog::load`] never scans
//! the graph. Install a loaded catalog with [`with_stats_catalog`] while planning
//! queries. The feature is opt-in and affects join order only.

use sparq_core::dict::Id;
use sparq_core::Graph;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::{self, Read, Write};
use std::path::Path;
use std::sync::Arc;

const MAGIC: &[u8; 8] = b"SPQSTAT1";
const FILE_NAME: &str = "stats.bin";
const DEFAULT_BUCKETS: usize = 64;

// [SONNET-4.6] Keep the nested accumulator readable and clippy-clean.
type ValueFrequencies = (BTreeMap<Id, u64>, BTreeMap<Id, u64>);

/// One deterministic equi-depth value-histogram bucket.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistogramBucket {
    /// Inclusive upper dictionary-id boundary.
    pub upper: Id,
    /// Rows represented by this bucket.
    pub count: u64,
    /// Distinct values represented by this bucket.
    pub distinct: u64,
}

/// Statistics for one predicate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PredicateStats {
    /// Number of triples carrying the predicate.
    pub count: u64,
    /// Equi-depth histogram of subject ids.
    pub subjects: Vec<HistogramBucket>,
    /// Equi-depth histogram of object ids.
    pub objects: Vec<HistogramBucket>,
}

/// Versioned statistics catalog loaded without touching permutation indexes.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StatsCatalog {
    predicates: BTreeMap<Id, PredicateStats>,
}

impl StatsCatalog {
    /// Returns statistics for a predicate dictionary id.
    pub fn predicate(&self, predicate: Id) -> Option<&PredicateStats> {
        self.predicates.get(&predicate)
    }

    /// Number of predicates represented by the catalog.
    pub fn len(&self) -> usize {
        self.predicates.len()
    }

    /// Whether the catalog contains no predicates.
    pub fn is_empty(&self) -> bool {
        self.predicates.is_empty()
    }

    /// Loads `stats.bin` from a saved graph directory.
    pub fn load(dir: impl AsRef<Path>) -> io::Result<Self> {
        let mut bytes = Vec::new();
        std::fs::File::open(dir.as_ref().join(FILE_NAME))?.read_to_end(&mut bytes)?;
        decode(&bytes)
    }

    /// Deterministic encoded size, useful for storage regression gates.
    pub fn encoded_bytes(&self) -> usize {
        encode(self).len()
    }
}

/// Result of an explicit [`analyze`] operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AnalyzeMetrics {
    /// Triples inspected.
    pub triples: u64,
    /// Predicate records written.
    pub predicates: u64,
    /// Histogram buckets written across both value columns.
    pub buckets: u64,
    /// Exact catalog file size.
    pub bytes: u64,
}

/// Explicitly rebuilds and atomically publishes `stats.bin` beside a saved graph.
pub fn analyze(graph: &Graph, dir: impl AsRef<Path>) -> io::Result<AnalyzeMetrics> {
    let dir = dir.as_ref();
    let mut values: BTreeMap<Id, ValueFrequencies> = BTreeMap::new();
    let mut triples = 0u64;
    for [s, p, o] in graph.iter_ids() {
        let (subjects, objects) = values.entry(p).or_default();
        *subjects.entry(s).or_default() += 1;
        *objects.entry(o).or_default() += 1;
        triples += 1;
    }
    let predicates = values
        .into_iter()
        .map(|(p, (s, o))| {
            let count = s.values().sum();
            (
                p,
                PredicateStats {
                    count,
                    subjects: histogram(&s, DEFAULT_BUCKETS),
                    objects: histogram(&o, DEFAULT_BUCKETS),
                },
            )
        })
        .collect();
    let catalog = StatsCatalog { predicates };
    let bytes = encode(&catalog);
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join("stats.bin.tmp");
    {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(tmp, dir.join(FILE_NAME))?;
    let buckets = catalog
        .predicates
        .values()
        .map(|s| s.subjects.len() + s.objects.len())
        .sum::<usize>() as u64;
    Ok(AnalyzeMetrics {
        triples,
        predicates: catalog.len() as u64,
        buckets,
        bytes: bytes.len() as u64,
    })
}

fn histogram(freq: &BTreeMap<Id, u64>, limit: usize) -> Vec<HistogramBucket> {
    if freq.is_empty() {
        return Vec::new();
    }
    let total: u64 = freq.values().sum();
    let target = total.div_ceil(limit.min(freq.len()) as u64);
    let mut out = Vec::new();
    let (mut count, mut distinct) = (0u64, 0u64);
    for (&upper, &n) in freq {
        count += n;
        distinct += 1;
        if count >= target && out.len() + 1 < limit {
            out.push(HistogramBucket { upper, count, distinct });
            count = 0;
            distinct = 0;
        }
    }
    if distinct != 0 {
        out.push(HistogramBucket { upper: *freq.last_key_value().unwrap().0, count, distinct });
    }
    out
}

fn encode(catalog: &StatsCatalog) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(MAGIC);
    put_u64(&mut out, catalog.len() as u64);
    for (&predicate, stats) in &catalog.predicates {
        out.extend_from_slice(&predicate.to_le_bytes());
        put_u64(&mut out, stats.count);
        put_hist(&mut out, &stats.subjects);
        put_hist(&mut out, &stats.objects);
    }
    out
}

fn put_u64(out: &mut Vec<u8>, n: u64) {
    out.extend_from_slice(&n.to_le_bytes());
}
fn put_hist(out: &mut Vec<u8>, hist: &[HistogramBucket]) {
    put_u64(out, hist.len() as u64);
    for b in hist {
        out.extend_from_slice(&b.upper.to_le_bytes());
        put_u64(out, b.count);
        put_u64(out, b.distinct);
    }
}

fn decode(bytes: &[u8]) -> io::Result<StatsCatalog> {
    let mut at = 0usize;
    let take = |at: &mut usize, n: usize| -> io::Result<&[u8]> {
        let end = at
            .checked_add(n)
            .filter(|&end| end <= bytes.len())
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "truncated statistics catalog"))?;
        let slice = &bytes[*at..end];
        *at = end;
        Ok(slice)
    };
    if take(&mut at, MAGIC.len())? != MAGIC {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "unsupported statistics catalog"));
    }
    let read_u64 = |at: &mut usize| -> io::Result<u64> {
        Ok(u64::from_le_bytes(take(at, 8)?.try_into().unwrap()))
    };
    let n = read_u64(&mut at)?;
    let min_record = std::mem::size_of::<Id>() + 24;
    if n > (bytes.len().saturating_sub(at) / min_record) as u64 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid statistics record count"));
    }
    let mut predicates = BTreeMap::new();
    for _ in 0..n {
        let p = Id::from_le_bytes(take(&mut at, std::mem::size_of::<Id>())?.try_into().unwrap());
        let count = read_u64(&mut at)?;
        let mut read_hist = || -> io::Result<Vec<HistogramBucket>> {
            let n = read_u64(&mut at)?;
            if n > (bytes.len().saturating_sub(at) / (std::mem::size_of::<Id>() + 16)) as u64 {
                return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid histogram bucket count"));
            }
            let mut h = Vec::with_capacity(n as usize);
            for _ in 0..n {
                let upper = Id::from_le_bytes(take(&mut at, std::mem::size_of::<Id>())?.try_into().unwrap());
                h.push(HistogramBucket {
                    upper,
                    count: read_u64(&mut at)?,
                    distinct: read_u64(&mut at)?,
                });
            }
            Ok(h)
        };
        let subjects = read_hist()?;
        let objects = read_hist()?;
        predicates.insert(p, PredicateStats { count, subjects, objects });
    }
    if at != bytes.len() {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "trailing statistics catalog data"));
    }
    Ok(StatsCatalog { predicates })
}

thread_local! {
    static ACTIVE: RefCell<Option<Arc<StatsCatalog>>> = const { RefCell::new(None) };
}

struct Guard(Option<Arc<StatsCatalog>>);
impl Drop for Guard {
    fn drop(&mut self) {
        ACTIVE.with(|a| *a.borrow_mut() = self.0.take());
    }
}

/// Installs a catalog for every query planned inside `f`.
pub fn with_stats_catalog<T>(catalog: &Arc<StatsCatalog>, f: impl FnOnce() -> T) -> T {
    let previous = ACTIVE.with(|a| a.borrow_mut().replace(Arc::clone(catalog)));
    let _guard = Guard(previous);
    f()
}

pub(crate) fn predicate_ndv(predicate: Id, position: usize) -> Option<u64> {
    ACTIVE.with(|a| {
        let a = a.borrow();
        let stats = a.as_ref()?.predicate(predicate)?;
        let hist = match position {
            0 => &stats.subjects,
            2 => &stats.objects,
            _ => return None,
        };
        Some(hist.iter().map(|b| b.distinct).sum())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn analyze_round_trips_deterministically_and_rejects_corruption() {
        let graph = Graph::load_str(
            "<x:s1> <x:p> <x:o1> .\n<x:s2> <x:p> <x:o1> .\n<x:s3> <x:q> <x:o2> .",
            "ntriples",
        )
        .unwrap();
        let dir = std::env::temp_dir().join(format!("sparq-stats-{}", std::process::id()));
        let first = analyze(&graph, &dir).unwrap();
        let bytes = std::fs::read(dir.join(FILE_NAME)).unwrap();
        let second = analyze(&graph, &dir).unwrap();
        assert_eq!(first, second);
        assert_eq!(bytes, std::fs::read(dir.join(FILE_NAME)).unwrap());
        let catalog = StatsCatalog::load(&dir).unwrap();
        assert_eq!(catalog.len(), 2);
        let mut wrong = bytes;
        wrong.push(0);
        assert!(decode(&wrong).is_err(), "mutation must be detected");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn installed_catalog_is_result_equivalent_and_guard_declines() {
        let mut data = String::new();
        for i in 0..40 {
            data.push_str(&format!("<x:s{i}> <x:p> <x:o{}> .\n", i % 7));
            if i % 3 == 0 {
                data.push_str(&format!("<x:o{}> <x:q> <x:v{i}> .\n", i % 7));
            }
        }
        let graph = Graph::load_str(&data, "ntriples").unwrap();
        let dir = std::env::temp_dir().join(format!("sparq-stats-diff-{}", std::process::id()));
        analyze(&graph, &dir).unwrap();
        let catalog = Arc::new(StatsCatalog::load(&dir).unwrap());
        let sparql = "SELECT ?s ?v WHERE { ?s <x:p> ?o . ?o <x:q> ?v }";
        let fallback = crate::query(&graph, sparql).unwrap();
        let planned = with_stats_catalog(&catalog, || crate::query(&graph, sparql)).unwrap();
        assert_eq!(fallback.vars, planned.vars);
        assert_eq!(fallback.rows, planned.rows, "catalog planning must preserve the solution bag");
        let declined = crate::query(&graph, sparql).unwrap();
        assert_eq!(fallback.rows, declined.rows, "guard must restore the prior planner path");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
