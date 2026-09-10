<!-- [OPUS-4.8] sq-h0tr / sq-im8u — mdBook table of contents.

This SUMMARY mirrors the structure of the usage-skills router (skills/SKILL.md):
an introduction, then a "Getting started" section. As of sq-im8u the existing
content pages are thin {{#include}} wrappers — they single-source their prose
(build-time content injection) from the canonical README.md / skills/<surface>/SKILL.md
anchors rather than carrying their own copy, so the guide cannot drift from the
source of truth. This SUMMARY and the per-surface page set are the only places
original prose may live (a one-line section heading / intro). The full per-surface
page set is part of the content-migration beads, not this scaffold. Keep this in
sync with src/. -->

# Summary

[Introduction](./introduction.md)

# Getting started

- [Install & build from source](./getting-started/install.md)
- [Capabilities at a glance](./getting-started/capabilities.md)
- [Rust crate catalogue](./getting-started/rust-crates.md)
