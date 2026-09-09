# Issue6475 deterministic replay

[GPT-6 Astra] This standalone diagnostic constructs the saved three-quad pair directly in oxrdf0.2.4 and invokes pinned rdf-canon0.15.3, with no Sparq bridge or engine. Exit1 is the reproduced failure; the calibration exits normally before the failing pair. It is not a passing test suite or a proposed patch.

Run `python3 run.py` from this evidence directory. It resolves only cached dependencies, formats the exact harness, builds offline/locked with the installed pinned compiler and two jobs, then executes the diagnostic. The runner stops at300seconds or below6GiB free disk. To inspect existing evidence without rebuilding, read commands.json/replay.log/replay.stderr.log. The exported binary can be run directly on the recorded native host architecture.

The final manifest excludes the regenerable private target directory; its path and size are recorded. It includes the exported binary, final formatted source, Cargo.lock, complete resolved dependency graph, actual output and upstream/source provenance. Existing cached registry sources are verified against their exact .crate archive checksums, not edited.

Production repair remains unproven. A change to the shared canonicalizer must preserve conformance, default outputs, hash profiles and poison-input budgets; do not modify the bridge or retry the random property until it passes. See report.json for the precise next bounded experiment and ownership limitations.
