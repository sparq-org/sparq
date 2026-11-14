# Query compiler agreement

This private test harness compares the pinned native and WebAssembly compilers
on every operator program for a plan marked passing by the query coverage manifest. It
compares the ABI and decoded bytecode, then independently derives a WebAssembly
verification key and compares its bytes with the native key. This is research
grade and not externally audited.

Install with `npm ci --ignore-scripts` in this directory. The corpus live test in
`sparq-zk-query` uses the existing toolchain gating helper and requires Node on
PATH. The workflow selects the supported Node runtime. Comparator controls run
with `node check.mjs --self-test`.

The checker takes a package directory, native artifact path, native key path and
private cache directory. An optional final argument selects the CRS cache. Using
`--compile-only` in place of the native key compares ABI and bytecode and leaves
the key comparison pending; it does not satisfy the full corpus ratchet.

The corpus records the approved pinned XPath dependency loading failure by name
for each affected passing plan. `--record-known-failures` permits only the
registered dependency revision and diagnostic, records that bytecode and keys
were not compared, and returns a distinct status. Other failures remain errors.
The normal checker stays strict. A dependency revision change makes comparison
mandatory again; no source or manifest is patched during this check.
