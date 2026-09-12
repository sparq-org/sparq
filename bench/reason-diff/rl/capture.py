#!/usr/bin/env python3
# [FABLE-5] sq-xg6uu — OFFLINE golden-vector capture for the sparq-reason-diff
# OWL 2 RL differential harness. NEVER RUN IN CI: CI only re-diffs against the
# committed `.expected` vectors (crates/sparq-reason-diff). Re-run this script
# by hand when the corpus changes, in a venv:
#
#   python3 -m venv /tmp/owlrl-venv && /tmp/owlrl-venv/bin/pip install owlrl
#   /tmp/owlrl-venv/bin/python bench/reason-diff/rl/capture.py
#
# Protocol (mirrored by the Rust harness's docs in crates/sparq-reason-diff/src/rl.rs):
#   * Each `<case>.nt` here is a hand-picked, BLANK-NODE-FREE TBox/ABox document
#     with no user data at W3C-vocabulary subjects and only canonical-lexical
#     literals (the harness's cross-tool canonical-multiset comparison needs all
#     three constraints; it fails loud on bnodes rather than guessing labels).
#     The W3C-vocabulary-subject constraint is ENFORCED by the Rust harness, on
#     the document AND on sparq's closure of it (`rl::lint_corpus_document` /
#     `rl::sparq_rl_closure_lines`) — a case that asserts OR entails a triple at
#     such a subject fails, because normalization drops those from both sides and
#     would mask a real divergence. Re-run the Rust test after editing a case.
#   * The golden vector is the RAW owlrl closure — `OWLRL_Semantics` with
#     `axiomatic_triples=False, datatype_axioms=False` — serialized one canonical
#     N-Triples statement per line (rdflib `.n3()` per term) and byte-sorted.
#     Normalization (dropping eq-ref reflexivity + the W3C vocabulary
#     self-description block) happens in the RUST harness, applied identically to
#     both sides, so the committed vector stays maximally raw and re-derivable.
#   * The header pins the exact tool versions the vector was captured with; the
#     Rust loader REJECTS a vector without the `# tool:` pin.

import platform
import sys
from pathlib import Path

import owlrl
import rdflib

HERE = Path(__file__).resolve().parent


def capture(nt_path: Path) -> None:
    g = rdflib.Graph()
    g.parse(nt_path, format="nt")
    owlrl.DeductiveClosure(
        owlrl.OWLRL_Semantics, axiomatic_triples=False, datatype_axioms=False
    ).expand(g)
    lines = sorted(f"{s.n3()} {p.n3()} {o.n3()} ." for s, p, o in g)
    for line in lines:
        if line.startswith("_:") or " _:" in line:
            sys.exit(f"{nt_path.name}: blank node in closure ({line!r}) — "
                     "the corpus contract is bnode-free; use named nodes")
    out = nt_path.with_suffix(".expected")
    header = [
        "# sq-xg6uu golden vector — RAW owlrl OWL 2 RL closure (normalization is",
        "# applied by the Rust harness, identically to both sides; see",
        "# crates/sparq-reason-diff/src/rl.rs).",
        f"# tool: owlrl=={owlrl.__version__} rdflib=={rdflib.__version__} "
        f"python=={platform.python_version()}",
        "# semantics: OWLRL_Semantics axiomatic_triples=False datatype_axioms=False",
        f"# case: {nt_path.name}",
        "# capture: bench/reason-diff/rl/capture.py (OFFLINE ONLY — never run in CI)",
    ]
    out.write_text("\n".join(header + lines) + "\n")
    print(f"{out.name}: {len(lines)} triples")


def main() -> None:
    cases = sorted(HERE.glob("*.nt"))
    if not cases:
        sys.exit("no .nt cases found")
    for nt in cases:
        capture(nt)


if __name__ == "__main__":
    main()
