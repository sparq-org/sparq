# sparq-jsonld-registry

<!-- Keep this crate README aligned with the repository template. -->

`sparq-jsonld-registry` provides a small, bundled registry of well-known JSON-LD 1.1
contexts. It implements `sparq_jsonld::DocumentLoader` without filesystem or network
access. Exact registered IRIs resolve from bytes compiled into the crate; every other IRI
fails closed with the same error as `NoopLoader`.

The crate is experimental, unpublished, and opt-in: consumers depend on it explicitly.
See `contexts/PROVENANCE.md` for the source and licensing record of each bundled context.

## 🚀 Quickstart

Add `sparq-jsonld-registry` as an explicit dependency and pass a `RegistryLoader` to the
JSON-LD operation that needs bundled context resolution.

## ✨ Features

- Exact-match resolution for bundled, well-known JSON-LD 1.1 context IRIs.
- No filesystem or network access.
- Fail-closed handling for every unregistered IRI.
- Per-context provenance and licensing records.

## 📚 Learn more

- Bundled-context provenance: `contexts/PROVENANCE.md`.
- JSON-LD usage guidance: `skills/data-formats/SKILL.md`.
- JSON-LD 1.1: <https://www.w3.org/TR/json-ld11/>.

## License

MIT © the sparq authors. Bundled contexts retain the licenses recorded in
`contexts/PROVENANCE.md`.
