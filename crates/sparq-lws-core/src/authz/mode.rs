// AUTHORED-BY Claude Opus 4.8
//! Web Access Control access modes + the HTTP-method → required-mode mapping.
//!
//! The four WAC modes are NOT hierarchical in the data model (an ACL grants each explicitly), but
//! `Write` subsumes `Append` (an agent who may write a container may also append to it) and `Control`
//! governs the resource's own `.acl`. The handler maps each HTTP operation to the single mode it
//! requires; reading/writing an `.acl` resource always requires `Control`.
//!
//! Ported (semantics, not code) from prod-solid-server `src/authz/mode.ts` + `types.ts`.

/// The access modes Web Access Control distinguishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AccessMode {
    Read,
    Write,
    Append,
    Control,
}

impl AccessMode {
    /// The `acl:` mode IRI this mode corresponds to.
    pub fn acl_iri(self) -> &'static str {
        match self {
            AccessMode::Read => "http://www.w3.org/ns/auth/acl#Read",
            AccessMode::Write => "http://www.w3.org/ns/auth/acl#Write",
            AccessMode::Append => "http://www.w3.org/ns/auth/acl#Append",
            AccessMode::Control => "http://www.w3.org/ns/auth/acl#Control",
        }
    }

    /// The lowercase token used in the `WAC-Allow` header (`read`/`write`/`append`/`control`).
    pub fn token(self) -> &'static str {
        match self {
            AccessMode::Read => "read",
            AccessMode::Write => "write",
            AccessMode::Append => "append",
            AccessMode::Control => "control",
        }
    }

    /// Map an `acl:` mode IRI to an [`AccessMode`], or `None` for an unrecognised IRI.
    pub fn from_acl_iri(iri: &str) -> Option<AccessMode> {
        match iri {
            "http://www.w3.org/ns/auth/acl#Read" => Some(AccessMode::Read),
            "http://www.w3.org/ns/auth/acl#Write" => Some(AccessMode::Write),
            "http://www.w3.org/ns/auth/acl#Append" => Some(AccessMode::Append),
            "http://www.w3.org/ns/auth/acl#Control" => Some(AccessMode::Control),
            _ => None,
        }
    }
}

/// The `.acl` auxiliary-resource suffix (`<resource>.acl`, `<container>/.acl`).
pub const ACL_SUFFIX: &str = ".acl";

/// Whether an IRI names an ACL auxiliary resource (ends in `.acl`).
///
/// This is the predicate the WAC resolver and the handler use to gate ACL access at Control; it
/// stays EXACT-CASE on purpose because the resolver only ever derives a lowercase `…/x.acl`
/// (`acl_for` = `format!("{resource}.acl")`), so an exact-case match is what governs access. The
/// broader, case-insensitive [`is_acl_auxiliary_suffix`] is what the create/mint chokepoint uses to
/// fail closed against any case variant.
pub fn is_acl_resource(iri: &str) -> bool {
    iri.ends_with(ACL_SUFFIX)
}

/// Whether an IRI's FINAL path segment ends in the load-bearing `.acl` auxiliary suffix, matched
/// CASE-INSENSITIVELY.
///
/// This is the create/mint-side guard, NOT the access-side predicate. It mirrors [`is_acl_resource`]
/// (the access-side predicate the WAC resolver consults) but is broader in robustness: case-insensitive
/// and applied to the final path segment, so an Append-only POST can NEVER mint a child that the WAC
/// resolver will later consult as another resource's load-bearing ACL — even via a case variant
/// (`secret.ACL`) or a container-child slug (`secret.acl/`). The check is applied to the final path
/// segment (after the last `/`) so a `.acl` appearing only mid-path cannot false-positive, while both
/// `…/secret.acl` (a resource) and `…/.acl` (a container's own ACL) are caught.
///
/// SCOPE — `.acl` ONLY (deliberate). The ACL auxiliary is the only auxiliary this server treats as
/// load-bearing: the WAC resolver consults `<resource>.acl`, and the PUT/PATCH create paths only
/// special-case `.acl`. A name ending in `.meta` is the LWS surface's linkset record, and
/// `ldp::target::parse_target` refuses it, and any path through it, for every LDP method; a POST
/// mints the stem with a suffix of its own, never such a name, so this mint guard need not
/// cover it.
///
/// FORWARD-LOOKING INVARIANT: if/when `.meta` (or any other auxiliary) becomes load-bearing — i.e.
/// the resolver or a metadata path starts consulting it — it MUST be guarded UNIFORMLY across the
/// whole surface (POST mint AND PUT/PATCH create AND DELETE AND the read/access path), not POST-only.
/// A POST-only guard, as this finding showed, gives no security benefit while a PUT/PATCH hole remains.
pub fn is_acl_auxiliary_suffix(iri: &str) -> bool {
    // The final path segment: everything after the last '/'. For a container child IRI ending in a
    // trailing '/', the segment before that slash is what was minted; strip one trailing slash so a
    // `Slug: foo.acl` requesting a CONTAINER child (`…/foo.acl/`) is still caught.
    let trimmed = iri.strip_suffix('/').unwrap_or(iri);
    let segment = trimmed.rsplit('/').next().unwrap_or(trimmed);
    segment.to_ascii_lowercase().ends_with(ACL_SUFFIX)
}

/// Map an HTTP method + target to the single WAC [`AccessMode`] the operation requires.
///
///  - **Any operation on an `.acl` resource requires [`AccessMode::Control`]** — reading or writing
///    access rules is the `acl:Control` privilege, regardless of the HTTP method.
///  - `GET`/`HEAD`/`OPTIONS` → [`AccessMode::Read`].
///  - `POST` to a **container** → [`AccessMode::Append`] (adding a member is appending; `Write`
///    subsumes `Append`, so a writer also satisfies it). `POST` to a non-container → `Write` (the
///    handler rejects it on other grounds anyway).
///  - `PUT`/`PATCH`/`DELETE` (and anything else that mutates) → [`AccessMode::Write`].
///
/// `is_container` is whether the target IRI names a container (trailing slash); it only affects POST.
pub fn mode_for_operation(method: &str, target: &str, is_container: bool) -> AccessMode {
    if is_acl_resource(target) {
        return AccessMode::Control;
    }
    match method {
        "GET" | "HEAD" | "OPTIONS" => AccessMode::Read,
        "POST" => {
            if is_container {
                AccessMode::Append
            } else {
                AccessMode::Write
            }
        }
        // PUT, PATCH, DELETE, and any other mutating verb.
        _ => AccessMode::Write,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn acl_iris_round_trip() {
        for m in [
            AccessMode::Read,
            AccessMode::Write,
            AccessMode::Append,
            AccessMode::Control,
        ] {
            assert_eq!(AccessMode::from_acl_iri(m.acl_iri()), Some(m));
        }
        assert_eq!(AccessMode::from_acl_iri("http://example.org/Other"), None);
    }

    #[test]
    fn is_acl_resource_matches_dot_acl() {
        assert!(is_acl_resource("https://pod.example/a/b.acl"));
        assert!(is_acl_resource("https://pod.example/a/.acl"));
        assert!(!is_acl_resource("https://pod.example/a/b"));
        assert!(!is_acl_resource("https://pod.example/a/"));
    }

    #[test]
    fn is_acl_auxiliary_suffix_catches_acl_case_insensitively() {
        // The exact-case `.acl` the resolver consults.
        assert!(is_acl_auxiliary_suffix("https://pod.example/a/secret.acl"));
        assert!(is_acl_auxiliary_suffix("https://pod.example/a/.acl"));
        // Case variants MUST be caught at the mint chokepoint (defence-in-depth, even though the
        // resolver itself only derives lowercase `.acl`).
        assert!(is_acl_auxiliary_suffix("https://pod.example/a/secret.ACL"));
        assert!(is_acl_auxiliary_suffix("https://pod.example/a/secret.Acl"));
        // A CONTAINER child minted with an `.acl` slug (trailing slash) is caught too.
        assert!(is_acl_auxiliary_suffix("https://pod.example/a/secret.acl/"));
        assert!(is_acl_auxiliary_suffix("https://pod.example/a/secret.ACL/"));
    }

    #[test]
    fn is_acl_auxiliary_suffix_does_not_guard_meta() {
        // `.meta` names are reserved by `parse_target` for every LDP method, not by this mint
        // guard, which covers `.acl` alone (see the predicate doc).
        assert!(!is_acl_auxiliary_suffix(
            "https://pod.example/a/secret.meta"
        ));
        assert!(!is_acl_auxiliary_suffix("https://pod.example/a/.meta"));
        assert!(!is_acl_auxiliary_suffix(
            "https://pod.example/a/secret.META"
        ));
    }

    #[test]
    fn is_acl_auxiliary_suffix_allows_benign_names() {
        // Plain resources / containers are NOT auxiliary.
        assert!(!is_acl_auxiliary_suffix("https://pod.example/a/secret"));
        assert!(!is_acl_auxiliary_suffix("https://pod.example/a/secret/"));
        assert!(!is_acl_auxiliary_suffix("https://pod.example/a/note.ttl"));
        assert!(!is_acl_auxiliary_suffix("https://pod.example/a/photo.jpg"));
        // A `.acl` appearing only MID-path (not the final segment) must NOT false-positive — only the
        // leaf segment is what gets minted/consulted.
        assert!(!is_acl_auxiliary_suffix("https://pod.example/x.acl/child"));
        // A name that merely CONTAINS "acl" but does not END in the suffix is fine.
        assert!(!is_acl_auxiliary_suffix("https://pod.example/a/aclremap"));
        assert!(!is_acl_auxiliary_suffix("https://pod.example/a/metadata"));
    }

    #[test]
    fn reading_an_acl_requires_control_regardless_of_method() {
        let acl = "https://pod.example/a/.acl";
        assert_eq!(mode_for_operation("GET", acl, false), AccessMode::Control);
        assert_eq!(mode_for_operation("HEAD", acl, false), AccessMode::Control);
        assert_eq!(mode_for_operation("PUT", acl, false), AccessMode::Control);
        assert_eq!(
            mode_for_operation("DELETE", acl, false),
            AccessMode::Control
        );
        assert_eq!(mode_for_operation("PATCH", acl, false), AccessMode::Control);
    }

    #[test]
    fn read_methods_require_read() {
        let r = "https://pod.example/a/b";
        assert_eq!(mode_for_operation("GET", r, false), AccessMode::Read);
        assert_eq!(mode_for_operation("HEAD", r, false), AccessMode::Read);
        assert_eq!(mode_for_operation("OPTIONS", r, false), AccessMode::Read);
    }

    #[test]
    fn post_to_container_is_append_else_write() {
        let c = "https://pod.example/a/";
        let r = "https://pod.example/a/b";
        assert_eq!(mode_for_operation("POST", c, true), AccessMode::Append);
        assert_eq!(mode_for_operation("POST", r, false), AccessMode::Write);
    }

    #[test]
    fn mutating_methods_require_write() {
        let r = "https://pod.example/a/b";
        assert_eq!(mode_for_operation("PUT", r, false), AccessMode::Write);
        assert_eq!(mode_for_operation("PATCH", r, false), AccessMode::Write);
        assert_eq!(mode_for_operation("DELETE", r, false), AccessMode::Write);
    }
}
