//! sq-3jtd.5 — TRUSTED per-resource creator/owner provenance.
//!
//! `acp:CreatorAgent` / `acp:OwnerAgent` matchers grant the *creator* / *owner* of the
//! resource being accessed. "Who created `<r>`" is structural storage metadata the
//! storage layer (PSS) knows when it mints the resource — it is **not** in the pod or the
//! ACR. This type is the TRUSTED CHANNEL by which that caller supplies the facts.
//!
//! # Security boundary (design doc §2.4)
//!
//! The creator/owner fact MUST arrive here, from the trusted caller — it is **never**
//! read from the resource graph. A writer who can put `<r> solidx:creator <self>` in a
//! document they control must not thereby grant themselves access; the loader only ever
//! synthesizes `solidx:creator`/`solidx:owner` facts from THIS map, never from pod
//! content. Supplying a value through this API is an assertion of trust by the caller.

use rustc_hash::FxHashMap;

/// The trusted creator and owner WebID of one resource. `None` = unknown to the caller
/// (so no `CreatorAgent`/`OwnerAgent` grant can ever fire on that resource — fail-closed).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct ResourceProvenance {
    creator: Option<String>,
    owner: Option<String>,
}

/// Caller-supplied (TRUSTED) per-resource creator/owner WebIDs, the input to
/// `acp:CreatorAgent` / `acp:OwnerAgent` matcher resolution. Built by the storage layer
/// and passed to [`crate::PodStore::materialize_acp_with`] /
/// [`crate::materialize_acp_with`]; an empty map (the default) means no creator/owner is
/// known, so no `CreatorAgent`/`OwnerAgent` matcher ever grants — fully fail-closed.
///
/// These facts are asserted by the caller and are **never** read from the resource graph —
/// see the type-level docs above for the full trust boundary.
///
/// # Examples
///
/// ```
/// use sparq_solid::AccessProvenance;
///
/// let mut prov = AccessProvenance::default();
/// prov.set_creator("https://pod.ex/notes/n1.ttl", "https://alice.ex/card#me");
/// prov.set_owner("https://pod.ex/notes/n1.ttl", "https://alice.ex/card#me");
/// assert_eq!(prov.creator("https://pod.ex/notes/n1.ttl"), Some("https://alice.ex/card#me"));
/// assert_eq!(prov.creator("https://pod.ex/notes/n2.ttl"), None); // unknown -> fail-closed
/// ```
#[derive(Debug, Clone, Default)]
pub struct AccessProvenance {
    by_resource: FxHashMap<String, ResourceProvenance>,
}

impl AccessProvenance {
    /// An empty provenance map (no creator/owner known for any resource).
    pub fn new() -> AccessProvenance {
        AccessProvenance::default()
    }

    /// Assert (TRUSTED) that `creator` is the creator WebID of `resource`. Replaces any
    /// previous creator for that resource. The value is taken on trust from the caller —
    /// see the [`AccessProvenance`] type docs for the security boundary.
    pub fn set_creator(&mut self, resource: impl Into<String>, creator: impl Into<String>) {
        self.by_resource.entry(resource.into()).or_default().creator = Some(creator.into());
    }

    /// Assert (TRUSTED) that `owner` is the owner WebID of `resource`. Replaces any
    /// previous owner for that resource.
    pub fn set_owner(&mut self, resource: impl Into<String>, owner: impl Into<String>) {
        self.by_resource.entry(resource.into()).or_default().owner = Some(owner.into());
    }

    /// The trusted creator WebID of `resource`, if the caller supplied one.
    pub fn creator(&self, resource: &str) -> Option<&str> {
        self.by_resource.get(resource).and_then(|p| p.creator.as_deref())
    }

    /// The trusted owner WebID of `resource`, if the caller supplied one.
    pub fn owner(&self, resource: &str) -> Option<&str> {
        self.by_resource.get(resource).and_then(|p| p.owner.as_deref())
    }

    /// Whether any creator or owner fact was supplied (an empty map materializes no
    /// `CreatorAgent`/`OwnerAgent` grants).
    pub fn is_empty(&self) -> bool {
        self.by_resource.is_empty()
    }

    /// Keep the facts of only the resources `exists` accepts. Internal — the store drops
    /// the facts of a resource that no longer exists, so one created again later under the
    /// same IRI does not inherit them.
    pub(crate) fn retain(&mut self, mut exists: impl FnMut(&str) -> bool) {
        self.by_resource.retain(|r, _| exists(r));
    }

    /// Forget every fact about `resource`. Internal (see [`AccessProvenance::retain`]).
    pub(crate) fn forget(&mut self, resource: &str) {
        self.by_resource.remove(resource);
    }

    /// Forget every fact. Internal (see [`AccessProvenance::retain`]).
    pub(crate) fn clear(&mut self) {
        self.by_resource.clear();
    }

    /// Iterate `(resource, creator?, owner?)` for every resource with a fact. Internal —
    /// used by the loader to synthesize `solidx:creator`/`solidx:owner` reasoning facts.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, Option<&str>, Option<&str>)> {
        self.by_resource
            .iter()
            .map(|(r, p)| (r.as_str(), p.creator.as_deref(), p.owner.as_deref()))
    }
}

#[cfg(test)]
mod tests {
    // sq-2xdr — behaviour tests for the trusted creator/owner channel. The
    // `iter()` accessor is `pub(crate)` (consumed by the loader), so it can only be
    // exercised from an in-crate test, not an integration test.
    use super::*;

    #[test]
    fn empty_map_knows_no_creator_or_owner() {
        // `new()` and `default()` must agree: an empty, fail-closed map.
        let prov = AccessProvenance::new();
        assert!(prov.is_empty());
        assert_eq!(prov.creator("r"), None);
        assert_eq!(prov.owner("r"), None);
        assert_eq!(prov.iter().count(), 0);
    }

    #[test]
    fn set_creator_and_owner_are_independent_per_resource() {
        let mut prov = AccessProvenance::new();
        prov.set_creator("r1", "https://alice.ex/#me");
        // setting only the creator leaves the owner unknown (fail-closed for OwnerAgent)
        assert_eq!(prov.creator("r1"), Some("https://alice.ex/#me"));
        assert_eq!(prov.owner("r1"), None);
        assert!(!prov.is_empty());

        prov.set_owner("r1", "https://bob.ex/#me");
        // owner is now set WITHOUT disturbing the creator on the same entry
        assert_eq!(prov.creator("r1"), Some("https://alice.ex/#me"));
        assert_eq!(prov.owner("r1"), Some("https://bob.ex/#me"));

        // a second resource carrying only an owner is fully independent
        prov.set_owner("r2", "https://carol.ex/#me");
        assert_eq!(prov.creator("r2"), None);
        assert_eq!(prov.owner("r2"), Some("https://carol.ex/#me"));
    }

    #[test]
    fn set_replaces_previous_value() {
        let mut prov = AccessProvenance::new();
        prov.set_creator("r1", "https://old.ex/#me");
        prov.set_creator("r1", "https://new.ex/#me");
        assert_eq!(prov.creator("r1"), Some("https://new.ex/#me"));
        prov.set_owner("r1", "https://o1.ex/#me");
        prov.set_owner("r1", "https://o2.ex/#me");
        assert_eq!(prov.owner("r1"), Some("https://o2.ex/#me"));
    }

    #[test]
    fn iter_yields_every_resource_with_its_optional_facts() {
        let mut prov = AccessProvenance::new();
        prov.set_creator("r1", "https://alice.ex/#me");
        prov.set_owner("r1", "https://alice.ex/#me");
        prov.set_owner("r2", "https://bob.ex/#me"); // creator unknown
        prov.set_creator("r3", "https://carol.ex/#me"); // owner unknown

        let mut seen: Vec<(String, Option<String>, Option<String>)> = prov
            .iter()
            .map(|(r, c, o)| (r.to_owned(), c.map(str::to_owned), o.map(str::to_owned)))
            .collect();
        seen.sort();
        assert_eq!(
            seen,
            vec![
                (
                    "r1".to_owned(),
                    Some("https://alice.ex/#me".to_owned()),
                    Some("https://alice.ex/#me".to_owned())
                ),
                ("r2".to_owned(), None, Some("https://bob.ex/#me".to_owned())),
                (
                    "r3".to_owned(),
                    Some("https://carol.ex/#me".to_owned()),
                    None
                ),
            ]
        );
    }

    #[test]
    fn impl_into_string_accepts_owned_keys_and_values() {
        // the `impl Into<String>` bounds accept owned String keys/values, not just &str.
        let mut prov = AccessProvenance::new();
        prov.set_creator(String::from("r1"), String::from("https://alice.ex/#me"));
        assert_eq!(prov.creator("r1"), Some("https://alice.ex/#me"));
    }
}
