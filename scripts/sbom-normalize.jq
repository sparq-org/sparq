# [OPUS-4.8] sq-toze.30 (GS-6 / F-6): deterministic, idempotent normalization of a
# cargo-cyclonedx CycloneDX SBOM so the PUBLISHED document carries NO host-revealing
# absolute build path.
#
# cargo-cyclonedx 0.5.9 emits, for every workspace / path dependency, refs + purls that
# embed the CI runner's absolute build directory (and the local layout):
#
#   bom-ref (component / dependency edge):
#       path+file:///abs/build/dir/crates/<name>#<version>
#       path+file:///abs/build/dir/crates/<name>#<version> bin-target-N   (root build targets)
#   purl:
#       pkg:cargo/<name>@<version>?download_url=file://.                   (root component)
#       pkg:cargo/<name>@<version>?download_url=file://../<rel>            (workspace members)
#       pkg:cargo/<name>@<version>?download_url=file://.#src/lib.rs        (root build targets)
#
# Registry deps are already host-independent
#   (registry+https://github.com/rust-lang/crates.io-index#<name>@<version>) and untouched.
#
# This filter rewrites every leaking ref to the canonical, host-independent CycloneDX form
#   pkg:cargo/<name>@<version>[<suffix>]
# preserving the real component identity (name@version) plus any trailing target suffix, and
# rewriting EVERY place a ref appears — metadata.component.bom-ref, that component's nested
# build-target components, each top-level component's bom-ref + purl, and every
# dependencies[].ref / dependsOn[] edge — so the internal reference graph stays consistent.
#
# Properties:
#   * Deterministic: a pure function of the input (no time / host / RNG); same in -> same out.
#   * Idempotent: the canonical form contains no "path+file" and no "download_url=file://",
#     so the guard conditions are false on a second pass -> byte-identical output.
#   * Validity-preserving: only ref/purl strings change; the dependency graph is rewritten in
#     lock-step (component bom-ref, every ref and every dependsOn edge), so all internal
#     references still resolve.

# Canonicalise a single bom-ref / dependsOn string.
#   path+file:///abs/.../<name>#<version><suffix>  ->  pkg:cargo/<name>@<version><suffix>
# where <suffix> is whatever trails the version (e.g. " bin-target-0"); usually empty.
def canon_ref:
  if type == "string" and startswith("path+file://") and test("#") then
    (sub("#.*$"; "") | sub("^.*/"; "")) as $name        # crate name = basename before '#'
    | (sub("^[^#]*#"; "")) as $rest                      # everything after the first '#'
    | ($rest | sub("^(?<v>[^ ]*)"; "")) as $suffix       # trailing suffix after the version token
    | ($rest | sub("^(?<v>[^ ]*).*$"; "\(.v)")) as $version
    | "pkg:cargo/\($name)@\($version)\($suffix)"
  elif type == "string" and startswith("git+") and test("#") then
    # [FABLE-5] sq-gg0qq.2 (GS-6): a GIT dependency (today only jeswr/solid-oidc-verifier).
    # cargo-cyclonedx 0.5.9 emits
    #   git+https://<host>/<owner>/<repo>?rev=<sha>#<version>
    # (the fragment is the bare version; the crate name is the repo basename — true for
    # every git dep this workspace allows, deny.toml pins the allow-list). Canonicalise to
    # the same host-independent form as every other component; the pin (rev) stays
    # discoverable in Cargo.lock + supply-chain/config.toml, not in the published ref.
    (sub("#.*$"; "") | sub("\\?.*$"; "") | sub("^.*/"; "")) as $name
    | (sub("^[^#]*#"; "")) as $version
    | "pkg:cargo/\($name)@\($version)"
  else
    .
  end;

# Canonicalise a purl. cargo-cyclonedx attaches, ONLY to workspace / path components, a
# `?download_url=file://<host-path>` qualifier — and, on the root build-target components
# (bin-target-N), additionally a `#<src-file>` subpath (e.g. `#src/main.rs`). Both encode the
# CI runner's filesystem layout and neither belongs in a *package* purl: the canonical cargo
# purl is `pkg:cargo/<name>@<version>`. We strip both, but ONLY when the workspace-local
# download_url qualifier is present, so registry purls
#   (pkg:cargo/<name>@<version>, no qualifier, no subpath) are returned byte-for-byte unchanged.
# [OPUS-4.8] sq-uujh: extend GS-6/sq-toze.30 to the build-target `#src/...` subpath, which the
# original filter left behind (it stripped the query up to `#` but preserved the fragment),
# leaving the only non-canonical purls in the SBOM and a purl/bom-ref mismatch on those rows.
def canon_purl:
  if type == "string" and test("[?&]download_url=file://") then
    # Drop the download_url qualifier together with any trailing host-derived #subpath.
    sub("[?&]download_url=file://.*$"; "")
  elif type == "string" and test("[?&]vcs_url=") then
    # [FABLE-5] sq-gg0qq.2 (GS-7): a GIT dependency's purl —
    #   pkg:cargo/<name>@<version>?vcs_url=git%2Bhttps://<host>/<owner>/<repo>%40<sha>
    # The canonical cargo purl carries NO query/fragment (scripts/check-sbom-purl-canonical.py
    # asserts ^pkg:cargo/[^?#]+@[^?#]+$); the exact rev pin remains in Cargo.lock +
    # supply-chain/config.toml. vcs_url is the only qualifier cargo-cyclonedx 0.5.9 emits
    # for a git dep, so the tail-strip is exact.
    sub("[?&]vcs_url=.*$"; "")
  else
    .
  end;

# [OPUS-4.8] sq-toze.26 (GS-1 / N1): emit a per-component CycloneDX `supplier`
# (organizationalEntity, NTIA "Supplier Name" slot) — derived HONESTLY from the
# component's identity in the RAW cargo-cyclonedx output, never fabricated.
#
# cargo-cyclonedx 0.5.9 leaves `supplier`/`publisher` empty on every component; only the
# 1.3-era `author` field is sometimes present (originator identity). NTIA's Minimum
# Elements names "Supplier Name" = the entity that supplies the component, and its guidance
# is explicit that where the supplier is not determinable it should be omitted / marked
# unknown rather than guessed. We classify each component by the SIGNAL cargo-cyclonedx
# encodes in its raw `bom-ref` (this MUST run BEFORE canon_ref, which strips that signal):
#
#   * registry+https://github.com/rust-lang/crates.io-index#<name>@<ver>
#       -> a crates.io-published crate. Supplier-of-record = the crates.io registry (the
#          distributor that supplied the component). supplier.name = "crates.io",
#          supplier.url = the crate's crates.io page (derived from the name). The crate's
#          own `author` (where present) is carried into `publisher` (the originator who
#          published it) — distinct from the distributing supplier.
#   * path+file://<abs>/crates/sparq-*#<ver>
#       -> a FIRST-PARTY workspace crate this project authors and ships. Supplier = the
#          project, matching the top-level supplier in supply-chain/vex.cdx.json
#          ({name:"Jesse Wright", url:["https://github.com/sparq-org/sparq"]}).
#   * path+file://<abs>/vendor/<name>#<ver>
#       -> a VENDORED UPSTREAM crate (today only `spargebra`, a [patch.crates-io] PATH
#          replacement of a crates.io-published crate). Its supplier-of-record is crates.io
#          (NOT this project — attributing it to sparq would be FABRICATION); treated like a
#          registry crate (supplier "crates.io" + the crate's crates.io URL; publisher from
#          author). This is why we cannot collapse "path+file => first-party".
#   * git+https://github.com/jeswr/<repo>?rev=<sha>#<version>
#       -> a GIT dependency pinned to the maintainer's own repository (today only
#          solid-oidc-verifier, sq-gg0qq.2). Supplier = the repository owner (the same
#          identity as the VEX top-level supplier), url = the repository. [FABLE-5]
#   * anything else (none today)
#       -> supplier NOT determinable -> OMITTED (no supplier emitted). Honest per NTIA.
#
# Idempotent + non-destructive: we never overwrite a `supplier` already present (so a future
# cargo-cyclonedx that populates it wins), and the derivation is a pure function of the raw
# bom-ref / author, so a second pass is byte-identical. Build-target sub-components (the root
# component's bin/lib targets) inherit via the fix_component recursion: they are under
# /crates/sparq-* and so get the first-party supplier, matching their parent.

# The crates.io project page for a published crate. Keyed off the component's own `name`
# field (always present + correct), NOT the bom-ref basename — the registry bom-ref's
# basename is the index name (`crates.io-index`), not the crate, whereas the crate name
# sits AFTER the '#'. Using `.name` is correct for both the registry and the vendored form.
def cratesio_url:
  "https://crates.io/crates/\(.name)";

# Derive the organizationalEntity supplier for one component from its RAW bom-ref.
# Returns null when the supplier is not honestly determinable (caller then omits it).
def derive_supplier($author):
  (."bom-ref" // "") as $ref
  | if ($ref | startswith("registry+https://github.com/rust-lang/crates.io-index")) then
      {name: "crates.io", url: [cratesio_url]}
    elif ($ref | test("^path\\+file://.*/vendor/")) then
      # vendored [patch.crates-io] upstream crate -> crates.io is the supplier-of-record
      {name: "crates.io", url: [cratesio_url]}
    elif ($ref | test("^path\\+file://.*/crates/sparq")) then
      # first-party workspace crate -> the project (matches the VEX top-level supplier)
      {name: "Jesse Wright", url: ["https://github.com/sparq-org/sparq"]}
    elif ($ref | test("^git\\+https://github\\.com/jeswr/")) then
      # [FABLE-5] sq-gg0qq.2 (GS-1): a GIT dependency pinned to the MAINTAINER'S OWN
      # repository (today only solid-oidc-verifier; deny.toml's sources allow-list keeps
      # this set closed). Supplier-of-record = the repository owner — the same identity as
      # the VEX top-level supplier, honestly determinable from the pinned source URL.
      # A git dep from any OTHER host/owner still falls through to null (omitted honestly).
      {name: "Jesse Wright", url: [($ref | sub("^git\\+"; "") | sub("\\?.*$"; ""))]}
    else
      null    # not determinable -> omitted honestly
    end;

# Attach `supplier` (and, for crates.io-supplied components, carry `author`->`publisher` as
# the originator identity) WITHOUT overwriting anything already present. Must run on the RAW
# component (before canon_ref/canon_purl), since the classification reads the raw bom-ref.
def add_supplier:
  (.author // null) as $author
  | (if has("supplier") then null else (derive_supplier($author)) end) as $sup
  | (if $sup == null then . else .supplier = $sup end)
  # publisher (the entity that published the component) only where it adds signal: a
  # crates.io-supplied component whose own author is known. Never invented; never clobbered.
  | (if (has("publisher") | not)
        and ($author != null)
        and ($sup != null) and ($sup.name == "crates.io")
      then .publisher = $author else . end);

def fix_component:
  add_supplier
  | (if has("bom-ref") then ."bom-ref" |= canon_ref else . end)
  | (if has("purl") then .purl |= canon_purl else . end)
  | (if has("components") then .components |= map(fix_component) else . end);

# [OPUS-4.8] sq-toze.28 (GS-4 / CDX-3): the SBOM generator now emits CycloneDX 1.5
# natively (cargo-cyclonedx --spec-version 1.5). On a 1.5 document, populate the
# 1.5-only `metadata.lifecycles` slot with the single phase we can honestly assert:
# the BOM is produced from the fully-resolved dependency tree during the build, i.e.
# CycloneDX lifecycle phase `build`. We do this here (not in the generator) so BOTH
# publication call-sites (scripts/gen-sbom-vex.sh and supply-chain.yml#sbom) get the
# field uniformly, and only on 1.5+ where the slot is schema-valid. Idempotent: the
# assignment is unconditional but value-stable, so a second pass is byte-identical.
def add_lifecycles:
  if (.specVersion? == "1.5" or .specVersion? == "1.6") then
    .metadata = ((.metadata // {}) | (.lifecycles = [{"phase": "build"}]))
  else . end;

# root metadata component (+ its nested build-target components, via fix_component recursion)
( if (.metadata? and .metadata.component?) then
    .metadata.component |= fix_component
  else . end )
# every top-level component (recursively)
| ( if has("components") then .components |= map(fix_component) else . end )
# dependency graph: ref + every dependsOn edge
| ( if has("dependencies") then
      .dependencies |= map(
        (if has("ref") then .ref |= canon_ref else . end)
        | (if has("dependsOn") then .dependsOn |= map(canon_ref) else . end)
      )
    else . end )
# CycloneDX 1.5+ metadata.lifecycles (build phase) — see add_lifecycles above.
| add_lifecycles
