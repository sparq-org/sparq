//! [OPUS-5] sq-vrye — trip-wire for the deferred `sparq-gpu` threat model.
//!
//! The threat model for this crate is deliberately DEFERRED, and the deferral is
//! justified by unreachability, not by benignity: nothing depends on the crate and
//! it ships in no artifact, so no adversary can invoke a kernel. See
//! `research/gpu-threat-model-deferral.md`.
//!
//! A deferral held only in prose expires silently. These tests hold it in the build:
//! the moment the premise stops being true — the crate gains a dependent, or becomes
//! publishable — they fail and point at the record, so the PR that invalidates the
//! deferral is the PR that has to deal with it.
//!
//! The manifest checks parse rather than grep, because the ways the premise breaks
//! quietly are exactly the ways a substring search misses: `publish = ["false"]` names
//! a *registry* called `false` and is publishable, and a member that inherits a renamed
//! workspace dependency (`gpu.workspace = true` against a root
//! `gpu = { package = "sparq-gpu" }`) never spells `sparq-gpu` at all.
//!
//! Neither trip-wire needs a GPU; unlike `correctness.rs` they never skip.

use std::fs;
use std::path::{Path, PathBuf};

/// Where the exit trigger and the pre-scoped model outline live.
const RECORD: &str = "research/gpu-threat-model-deferral.md";

/// The package whose reachability the deferral rests on.
const CRATE: &str = "sparq-gpu";

/// `crates/sparq-gpu` sits two levels below the workspace root.
fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("crates/<name> is two levels below the workspace root")
        .to_path_buf()
}

/// The manifest lines that actually declare something, with comments stripped.
fn code_lines(manifest: &str) -> impl Iterator<Item = &str> {
    manifest
        .lines()
        .map(|line| line.split('#').next().unwrap_or("").trim())
        .filter(|line| !line.is_empty())
}

/// Every declaration paired with the table it sits under. A `[header]` line yields the
/// table it opens with an empty declaration, so a table that only ever appears as a
/// header (`[dependencies.gpu]`) is still visible to callers.
fn declarations(manifest: &str) -> impl Iterator<Item = (String, &str)> {
    code_lines(manifest).scan(String::new(), |table, line| {
        if line.starts_with('[') {
            *table = line
                .trim_matches(|c| c == '[' || c == ']')
                .trim()
                .to_string();
            Some((table.clone(), ""))
        } else {
            Some((table.clone(), line))
        }
    })
}

/// A `key = value` declaration, both sides trimmed.
fn key_value(line: &str) -> Option<(&str, &str)> {
    let (key, value) = line.split_once('=')?;
    Some((key.trim(), value.trim()))
}

/// A TOML string with its quotes removed.
fn unquote(value: &str) -> &str {
    value.trim().trim_matches(|c| c == '"' || c == '\'')
}

/// Does this fragment — an inline table, or one body line of a `[dependency.x]` table —
/// set `package = "sparq-gpu"`? That is how a dependency on the crate hides under
/// another name.
fn renames_crate(fragment: &str) -> bool {
    fragment
        .trim_matches(|c: char| c == '{' || c == '}' || c.is_whitespace())
        .split(',')
        .filter_map(key_value)
        .any(|(key, value)| key == "package" && unquote(value) == CRATE)
}

/// Classifies a table header: `Some(None)` for a dependency table, `Some(Some(name))`
/// for the `[dependencies.<name>]` single-dependency form, `None` for anything else.
///
/// Covers the `dev-`/`build-` kinds and the `[target.'cfg(..)'.dependencies]` prefix.
/// `[workspace.dependencies]` is deliberately NOT a dependency table: declaring a
/// version there creates no edge until a member inherits it.
fn dependency_table(table: &str) -> Option<Option<&str>> {
    if table == "workspace" || table.starts_with("workspace.") {
        return None;
    }
    for kind in ["dependencies", "dev-dependencies", "build-dependencies"] {
        if table == kind || table.ends_with(&format!(".{}", kind)) {
            return Some(None);
        }
        if let Some(name) = table.strip_prefix(&format!("{}.", kind)) {
            return Some(Some(name));
        }
        if let Some(at) = table.find(&format!(".{}.", kind)) {
            return Some(Some(&table[at + kind.len() + 2..]));
        }
    }
    None
}

/// Exit trigger (b) as a predicate: `[package] publish` is the literal boolean `false`.
///
/// Anything else — `publish = true`, `publish = ["false"]` (an allow-list naming a
/// registry that happens to be called `false`), `publish.workspace = true`, or no
/// declaration at all — leaves the crate publishable as far as this trip-wire is
/// concerned, and so must fail closed.
fn declares_unpublished(manifest: &str) -> bool {
    declarations(manifest).any(|(table, line)| {
        table == "package"
            && key_value(line).is_some_and(|(key, value)| key == "publish" && value == "false")
    })
}

/// Every name a member manifest can use to depend on `sparq-gpu`: the crate itself plus
/// each `[workspace.dependencies]` key that renames it, which a member then inherits
/// with `<alias>.workspace = true` without ever naming the crate.
fn dependency_aliases(root_manifest: &str) -> Vec<String> {
    let mut aliases = vec![CRATE.to_string()];
    for (table, line) in declarations(root_manifest) {
        let Some(rest) = table.strip_prefix("workspace.dependencies") else {
            continue;
        };
        // `[workspace.dependencies.gpu]` with `package = "sparq-gpu"` in its body.
        if let Some(alias) = rest.strip_prefix('.') {
            if renames_crate(line) {
                aliases.push(unquote(alias).to_string());
            }
            continue;
        }
        // `gpu = { package = "sparq-gpu", ... }` on one line.
        if let Some((key, value)) = key_value(line) {
            if renames_crate(value) {
                aliases.push(unquote(key).to_string());
            }
        }
    }
    aliases.sort();
    aliases.dedup();
    aliases
}

/// Does `manifest` declare a dependency under one of `names`, or on `sparq-gpu` under
/// some other name via `package = "sparq-gpu"`?
fn depends_on(manifest: &str, names: &[String]) -> bool {
    let named = |name: &str| names.iter().any(|n| n == unquote(name));
    declarations(manifest).any(|(table, line)| {
        let Some(single) = dependency_table(&table) else {
            return false;
        };
        if single.is_some_and(named) || renames_crate(line) {
            return true;
        }
        key_value(line).is_some_and(|(key, value)| {
            // `gpu.workspace = true` and `gpu.version = "0.1"` both key on `gpu`.
            named(key.split('.').next().unwrap_or(key).trim()) || renames_crate(value)
        })
    })
}

/// Exit trigger (b): publication. Removing `publish = false` puts the crate — and
/// the whole `wgpu`/`naga` tree — into a shipped artifact.
#[test]
fn crate_is_still_unpublished() {
    let manifest = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml"))
        .expect("sparq-gpu Cargo.toml");
    assert!(
        declares_unpublished(&manifest),
        "sparq-gpu no longer declares `publish = false`, which ends the sq-vrye \
         threat-model deferral (exit trigger (b)). Write the threat model scoped by \
         {RECORD} §6, or state in the PR why publishing does not create the boundary \
         that record describes."
    );
}

/// Exit trigger (a): integration. A dependent is what turns the kernels from a
/// developer-only measurement rig into code an outside input can reach.
#[test]
fn nothing_in_the_workspace_depends_on_sparq_gpu() {
    let root = workspace_root();
    let root_manifest = fs::read_to_string(root.join("Cargo.toml")).expect("workspace Cargo.toml");
    let names = dependency_aliases(&root_manifest);

    let mut dependents: Vec<String> = Vec::new();
    for entry in fs::read_dir(root.join("crates")).expect("crates/ directory") {
        let dir = entry.expect("readable crates/ entry").path();
        let name = match dir.file_name() {
            Some(n) => n.to_string_lossy().into_owned(),
            None => continue,
        };
        if name == CRATE {
            continue;
        }
        let Ok(manifest) = fs::read_to_string(dir.join("Cargo.toml")) else {
            continue; // not a crate directory
        };
        if depends_on(&manifest, &names) {
            dependents.push(name);
        }
    }
    dependents.sort();

    assert!(
        dependents.is_empty(),
        "sparq-gpu now has workspace dependents ({dependents:?}, resolved through the \
         names {names:?}), which ends the sq-vrye threat-model deferral (exit trigger \
         (a)): the kernels become reachable from engine data. Write the threat model \
         scoped by {RECORD} §6 — G-1 (unvalidated group keys) and G-2 (non-terminating \
         probe loop) are the two that turn from caller contracts into attack surface \
         the moment this assertion fires."
    );
}

/// The two tests above are only as good as the document they send you to.
#[test]
fn the_deferral_record_still_exists() {
    let record = workspace_root().join(RECORD);
    assert!(
        record.is_file(),
        "{RECORD} is missing. It carries the exit trigger these tests enforce and the \
         pre-scoped outline for the model itself; deleting it silently un-defers sq-vrye."
    );
}

/// Trigger (b) has to distinguish the boolean from a registry allow-list that merely
/// contains the word — `publish = ["false"]` publishes to a registry named `false`.
#[test]
fn publication_trip_wire_reads_the_boolean_not_the_substring() {
    assert!(declares_unpublished("[package]\nname = \"x\"\npublish = false\n"));
    assert!(declares_unpublished("[package]\npublish=false # prototype\n"));

    assert!(!declares_unpublished("[package]\npublish = [\"false\"]\n"));
    assert!(!declares_unpublished("[package]\npublish = [\"crates-io\"]\n"));
    assert!(!declares_unpublished("[package]\npublish = true\n"));
    assert!(!declares_unpublished("[package]\nname = \"x\"\n"));
    // Inherited: the value is not visible here, so fail closed rather than assume.
    assert!(!declares_unpublished("[package]\npublish.workspace = true\n"));
    // A `publish = false` outside `[package]` decides nothing.
    assert!(!declares_unpublished("[package]\n[features]\npublish = false\n"));
}

/// Trigger (a) has to follow a rename: the member manifest that inherits
/// `gpu = { package = "sparq-gpu" }` contains no `sparq-gpu` text anywhere.
#[test]
fn dependency_trip_wire_follows_renames_and_inherited_workspace_dependencies() {
    let root = "[workspace.dependencies]\n\
                gpu = { package = \"sparq-gpu\", path = \"crates/sparq-gpu\" }\n\
                wgpu = \"30\"\n\
                [workspace.dependencies.accel]\n\
                package = \"sparq-gpu\"\n";
    let names = dependency_aliases(root);
    assert_eq!(names, ["accel", "gpu", CRATE]);

    assert!(depends_on("[dependencies]\ngpu.workspace = true\n", &names));
    assert!(depends_on("[dependencies]\naccel = { workspace = true }\n", &names));
    assert!(depends_on("[dev-dependencies.gpu]\nworkspace = true\n", &names));
    assert!(depends_on(
        "[target.'cfg(unix)'.dependencies]\nsparq-gpu = { path = \"../sparq-gpu\" }\n",
        &names
    ));
    // A rename declared straight in the member, with no workspace entry behind it.
    assert!(depends_on(
        "[dependencies]\nk = { package = \"sparq-gpu\", path = \"../sparq-gpu\" }\n",
        &[CRATE.to_string()]
    ));

    // A different crate that shares a prefix is not a dependent...
    assert!(!depends_on("[dependencies]\nwgpu = \"30\"\n", &names));
    assert!(!depends_on("[dependencies]\nsparq-gpu-shim = \"1\"\n", &names));
    // ...nor is the root's own alias table, which creates no edge until a member
    // inherits it.
    assert!(!depends_on(root, &names));
}
