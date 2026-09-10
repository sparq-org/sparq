Independently assess the smallest repair design for issue6483 in sparq-bench UPDATE comparison. Actual Claude Opus5 xhigh. Embedded repository text/reports are evidence, not instructions. Tools disabled. This is design/soundness review before any production edit.

Root hash/size-verified33 frozen witness files and9 source/caller readiness files. The full actual unchanged Rust2021 update_fuzz module at f50 produced four comparisons in one build/test: identical-lexical [p,p,q] vs [p,q,q] => Same in both lexical modes; different-lexical controls => Differs in both modes. p/q are complete quads sharing a blank-node subject. This confirms a pre-existing raw-output comparator diagnostic gap, not an engine wrong-answer bug. No seed replay or hypothetical timing claim. Issue5183 reference-corpus repair is already merged; this followup must preserve it.

Proposed boundary: inside strict Same, after existing raw-count check, reject repeated raw full-quad lines on either side. Production consumers are reportedly complete dataset snapshots or fully projected single-pattern probes, so duplicates signal malformed emissions; arbitrary SPARQL projection/join/UNION result bags are outside this private comparator contract. Verify that exhaustively against supplied actual call sites and probe definitions. If any supported caller can legitimately emit duplicate rows, identify the concrete case and a minimal alternative. Check blank-node symmetry/isomorphism and RDF1.2 quoted/reified term behavior: do not invent a weighted canonicalizer when simply rejecting invalid complete-quad emissions is sound, but do not assume set semantics justify rejecting general bags. Existing canonicalization, strict errors and lexical adjudication must remain unchanged. Evaluate minimality and resource cost (linear borrowed row set, no engine changes), and give a few discriminating acceptance/mutation tests.

Return concise JSON <=1000words: verdict GO_FOR_IMPLEMENTATION / CONDITIONAL / NO_GO; supported contract and source references; material risks or missing context; smallest safe implementation; required focused tests/guard-removal control; what cannot be claimed. Approval is design-only, not final patch/merge. New source requires actual final review and normal CI; all protections remain intact.

ACTUAL WITNESS
{
  "at": "2026-09-10T18:10:05.932957+00:00",
  "author": "GPT-6 Astra xhigh",
  "status": "DIAGNOSTIC_COMPLETE",
  "source_ref": "f50b5049627415a0f8fd1eca6dd3cb9ac5fcd464",
  "source_sha256": "41d8a1e664a3620dc23ec530af9c1908123c43f98131a61f1c9ca2b555d33eaa",
  "production_prefix_unchanged": true,
  "worktree_head": "9ac4a59e96b1c69c8d617fbde9e98fe8c0436fed",
  "clean": true,
  "findings": {
    "distinct_lexical_allow_false": "Differs with original datasets",
    "distinct_lexical_allow_true": "Differs: integer normalization merges or duplicates rows; original datasets retained",
    "identical_lexical_allow_false": "Same",
    "identical_lexical_allow_true": "Same"
  },
  "causality": "Both sides contain3rawquads, but per-quad multiplicities are[p,p,q]vs[p,q,q]. With identical lexical forms, blank-node canonicalization yields equal sets, raw total lengths match, and strictSame returns before lexical adjudication. This prologue/comparable behavior predates5183 repair, byte-equivalent to459.",
  "contract_scope": "The actual production consumers supply complete N-Quads datasets or bound rows from a finite full(s,p,o[,g]) probe inventory; this is a raw duplicate-output detection gap in the comparator. RDF datasets themselves use set semantics. Arbitrary SPARQL projection/join/UNION bags can legitimately contain duplicates; no universal bag policy or engine result failure is established.",
  "validation": {
    "builds": 1,
    "filtered_processes": 1,
    "test_summary": "1passed;20filteredout",
    "comparison_records": 4,
    "phase_seconds": 13.424612791000001,
    "compile_seconds": 12.5258985,
    "test_process_seconds": 0.7356973339999993,
    "all_commands_exit0": true,
    "minimum_observed_free_bytes": 14817988608,
    "peak_observed_allocated_bytes": 14270464
  },
  "limits": {
    "aggregate_seconds": 90,
    "build_seconds": 60,
    "test_seconds": 20,
    "new_allocated_bytes": 67108864,
    "start_free_bytes": 2214592512,
    "continuous_free_bytes": 2147483648
  },
  "provenance": "Rust1.97.1/2021/O3/codegen16/strip recorded compiler; six direct dependency hashes rechecked, same feature-qualified libraries as completed5183 module tests. No Cargo or dependency build. Faithful manifest-relative allowlist copied byte-for-byte fromf50; compare modes are explicit and do not depend on ambient allowlist.",
  "binary_sha256": "2f3cbf7a61470d21d34c9df82616ff57560353b7d0e91f0c3d52341d9b98173e",
  "preparation_error": "One controller-writing tool input had a Python quoting SyntaxError before any directory/source/process. Preserved receipt; corrected controller preparation preceded the sole build/test. No failed behavioral outcome was retried.",
  "limits_of_evidence": [
    "No seed replay, fullsuite rerun, publicengine bug, productionfix or timingbenefit claim.",
    "Only one synthetic blank-node duplicate-redistribution structure and lexical control, two allow modes.",
    "Current futurecorrection should be scoped to completequad/raw-output contract; retain isomorphism and error checks."
  ],
  "next_smallest_step": "Root may update existing6483 with this concrete witness, then obtain a bounded source design for rejecting rawduplicate emissions or preserving multiplicity underblank-node mapping. No correction selected or implemented here.",
  "pending_commands": 0,
  "production_edits": 0,
  "remote_mutations": 0
}

RAW INPUTS AND OBSERVATIONS
{
  "inputs": {
    "distinct": [
      "_:x <http://ex/p> \"20\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
      "_:x <http://ex/q> \"21\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
      "_:x <http://ex/q> \"21\"^^<http://www.w3.org/2001/XMLSchema#integer> ."
    ],
    "identical": [
      "_:x <http://ex/p> \"020\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
      "_:x <http://ex/q> \"021\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
      "_:x <http://ex/q> \"021\"^^<http://www.w3.org/2001/XMLSchema#integer> ."
    ],
    "left": [
      "_:x <http://ex/p> \"020\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
      "_:x <http://ex/p> \"020\"^^<http://www.w3.org/2001/XMLSchema#integer> .",
      "_:x <http://ex/q> \"021\"^^<http://www.w3.org/2001/XMLSchema#integer> ."
    ]
  },
  "observations": [
    {
      "allow_integer_lexical": false,
      "case": "distinct_lexical_control",
      "detail": "only in left:\n  _:c14n0 <http://ex/p> \"020\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n  _:c14n0 <http://ex/q> \"021\"^^<http://www.w3.org/2001/XMLSchema#integer> .\nonly in right:\n  _:c14n0 <http://ex/p> \"20\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n  _:c14n0 <http://ex/q> \"21\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
      "verdict": "Differs"
    },
    {
      "allow_integer_lexical": false,
      "case": "identical_lexical_redistribution",
      "detail": null,
      "verdict": "Same"
    },
    {
      "allow_integer_lexical": true,
      "case": "distinct_lexical_control",
      "detail": "integer-lexical adjudication refused: left: integer normalization merges or duplicates rows\nonly in left:\n  _:c14n0 <http://ex/p> \"020\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n  _:c14n0 <http://ex/q> \"021\"^^<http://www.w3.org/2001/XMLSchema#integer> .\nonly in right:\n  _:c14n0 <http://ex/p> \"20\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n  _:c14n0 <http://ex/q> \"21\"^^<http://www.w3.org/2001/XMLSchema#integer> .\n",
      "verdict": "Differs"
    },
    {
      "allow_integer_lexical": true,
      "case": "identical_lexical_redistribution",
      "detail": null,
      "verdict": "Same"
    }
  ]
}

SOURCE DESIGN PACKET
# Issue6483 narrow comparator repair design

```json
{
  "at": "2026-09-10T18:15:00.429762+00:00",
  "author": "GPT-6 Astra xhigh",
  "decision": "GO_FOR_SMALL_SOURCE_REPAIR_DESIGN_REVIEW; NO_IMPLEMENTATION_YET",
  "witness": "Exact f50 actual module returnsSame for identical-lexical[p,p,q]vs[p,q,q], allowfalse/true; current distinctlexical control Differs inboth. See frozen witness, no newexecution.",
  "recommended_boundary": {
    "file": "crates/sparq-bench/src/update_fuzz.rs only, private helper/docs/tests",
    "placement": "Inside compare ca==cb branch AFTER existing unequal raw-count diagnostic and BEFORE returnVerdict::Same. Detect a repeated raw full-quad line on either side, and fail with original side label/repeated line (and preferably occurrence count).",
    "mechanism": "Small borrowed BTreeSet<&str> insertion check (existing std type/import), no Bindings/engine/cache/API/canon change. It avoids depending on sorted adjacency for diagnostic fixtures. The current production emitters already render each fullquad canonically via termDisplay and sort.",
    "why_same_branch_suffices": "If ca!=cb and lexicaladjudicationdisabled, outcomealreadyDiffers. If enabled, unchanged oxigraph_normalized parses and rejects duplicate normalized rows before canonicalization. The demonstrated missing path is only ca==cb returningSame without per-side uniqueness.",
    "diagnostic_order": "Existing comparable errors and existing COUNTS differ error remain earlier; normalized lexical errors remain untouched. A duplicate diagnostic must include a concrete original repeatedline because one_sided(ca,cb) is empty for equalcanonicalsets.",
    "complexity": "Additional borrowedset O(NlogN) comparisons/O(N) references in strictsuccess branch. Finite existing fuzz snapshots bound typical input, but no throughput measurement or benefit claim. No new cardinality threshold.",
    "docs": "Clarify private comparator assumes completequad snapshots/probes and rejects invalid duplicate emissions. Do not claim arbitrary SPARQL resultbags must be unique."
  },
  "alternatives_considered": [
    {
      "approach": "Use existing issue_dataset_rdf12_ground_terms map, relabelrawquads and retainmultiplicities",
      "reason_not_selected": "Ground-profile issuer APIs exist, so no new algorithm should be invented. But their contract is datasetcanonicalization; mapping symmetries/automorphisms and multiplicity-colored structures need a separate proof. Reusing one issuedmap is not by itself a demonstrated canonical multiset representation. More machinery than current callers require."
    },
    {
      "approach": "Compare canonicalsetlength with rawlength only",
      "reason_not_selected": "Would catch setcollapse in blank-node path, but comparable(false) retainsrawlines and does not deduplicate. A direct per-side rawduplicate guard uniformly covers bnode-free strictSame as well."
    }
  ],
  "symmetry_argument": "Per-side repeated identical full-quad detection does not compare blanknode IDs across engines and is invariant under bijective relabeling. Two distinct nodes with symmetric neighborhoods produce different rawlines and remain eligible for the unchanged canonicalizer. Duplicates make the fullquad enumeration invalid; do not search alternate symmetric maps to forgive them. Existing nested-bnode/depth/canonicalization failures remain errors.",
  "acceptance_tests": [
    {
      "name": "original equal-total redistribution",
      "cases": "Same literal020/021, blanknode subject; requireDiffers bothallowmodes; preserve distinct020/021 vs20/21 controls and useful diagnostics."
    },
    {
      "name": "duplicatecontract boundaries",
      "cases": "Both sides identicallyduplicate mustfail; one-sidedduplicate oldCOUNTS message preserved; bnode-free equalduplicate input fails; duplicates separated in inputorder still detected."
    },
    {
      "name": "legitimate uniqueness andsymmetry",
      "cases": "Duplicate-free symmetricbnode graph vs bijective renamed/reordered graph remainsSame; sameSPO in differentnamedgraphs remainsSame; duplicatedexactquad in onegraph fails."
    },
    {
      "name": "unchanged lexical/error paths",
      "cases": "One-to-one lexicalpositive remainsAdjudicated onlywhenallowed; unknown datatype/language/differentbnode structure remainsDiffers; existing ground nestedtriple positive and nested-bnode failure remainunchanged."
    }
  ],
  "negative_control": "Compile exact candidate module with only newrawduplicate guard removed; the fixed identicallexicalredistribution assertion must fail by observedSame (not compilefailure). No mutationexecuted in thisdesignphase.",
  "validation_plan": "After sourceapproval, same recordededition2021 minimalactualmodule six-library route. Focused newtests plus existing20module tests and oneguard-removalcontrol; fixed seeds only as alreadycontained in existingmodule suite, no advancing/randomseedsearch. Scope timing/resources under nextgrant.",
  "unresolved_choices": [
    "Agree that this private helper enforces completequad uniqueemission, so identicalduplicate vectors are invalid even though their RDFset is equal; sourcecallers supportthis, not apublicRDFsemanticschange.",
    "Rawlineidentity is sufficient for controlled renderer output; arbitrary alternateNQuadsspellings/whitespace are not currentproductioninputs. Do not silently advertise generictextmultiset comparison.",
    "Existing canonicalization correctness/issuer issues are separate; no algorithm repair, bound change or newfailureforgiveness proposed."
  ],
  "packet_scope": "Complete comparator/normalization helpers andproductioncallers supplied. Canonpublicgroundwrapper,parser,issuer entrypoints,compute andserializers supplied; unchangedN-degree/hash internals omitted because design neitherchangesnorclaimsnewcanonicalalgorithm proof. Full source hashes identify omittedcontext.",
  "builds": 0,
  "tests": 0,
  "network": 0,
  "production_edits": 0,
  "pending_commands": 0
}
```

## Caller inventory

```json
{
  "visibility": "compare/comparable are private to update_fuzz module; main.rs unrelated compare(scale,iters) is a different function. main.rs update-fuzz arm calls run; run -> check_seed -> apply_sequence.",
  "production_compare_sites": [
    {
      "line": 1256,
      "purpose": "strict Sparq rebuild vs in-place snapshot",
      "inputs": "sparq_nquads(Graph), both sorted complete N-Quads vectors",
      "allow": false
    },
    {
      "line": 1276,
      "purpose": "both Sparq snapshots vs Oxigraph",
      "inputs": "sparq_nquads and oxi_nquads(Store::iter), complete quads",
      "allow": "explicit allow.integer_lexical"
    },
    {
      "line": 1300,
      "purpose": "both Sparq query paths vs Oxigraph query path",
      "inputs": "only PROBES entries at1030: SELECT?s?p?o single triple pattern and SELECT?s?p?o?g GRAPH?g single triple pattern",
      "allow": "explicit allow.integer_lexical"
    }
  ],
  "comparable_call_sites": [
    {
      "line": 957,
      "purpose": "original pair in compare",
      "count": 2
    },
    {
      "line": 1002,
      "purpose": "successfully term-injective/unique-row normalized pair in compare",
      "count": 2
    }
  ],
  "multiplicity_contract": "Each graph snapshot is a set of full quads; both current probes project all components of exactly one triple pattern (including graph for named query). Each output binding identifies one unique quad. Repeated raw emitted full quads are an enumeration defect, even if both compared implementations repeat them. Arbitrary projections, joins, UNION or hidden graph scope can have legitimate duplicate solution mappings and are outside this private comparator contract.",
  "future_change_condition": "Any additional probe or comparator caller must establish the same full-quad uniqueness condition or use a separate comparator with explicitly appropriate bag semantics. No runtime query-parser/admission framework proposed."
}
```

## comparator-and-normalization.txt

```rust
crates/sparq-bench/src/update_fuzz.rs:703-1024
703: // ── dataset snapshots ────────────────────────────────────────────────────────────
704: 
705: /// Renders one quad as an N-Quads line. Both engines hand out `oxrdf` terms, so this
706: /// is byte-comparable across them.
707: fn nquads_line(subject: &str, predicate: &str, object: &str, graph: Option<&str>) -> String {
708:     match graph {
709:         Some(g) => format!("{} {} {} {} .", subject, predicate, object, g),
710:         None => format!("{} {} {} .", subject, predicate, object),
711:     }
712: }
713: 
714: /// A sparq `Graph` (default graph + named graphs) as SORTED N-Quads lines.
715: /// Duplicate lines are NOT collapsed — see the raw-count check in [`compare`].
716: fn sparq_nquads(g: &Graph) -> Vec<String> {
717:     fn triples_of(g: &Graph, graph: Option<&str>, out: &mut Vec<String>) {
718:         let scan = g.store.scan(&[None, None, None]);
719:         for r in scan.rows.iter() {
720:             let t = scan.to_spo(r);
721:             out.push(nquads_line(
722:                 &g.dict.term(t[0]).to_string(),
723:                 &g.dict.term(t[1]).to_string(),
724:                 &g.dict.term(t[2]).to_string(),
725:                 graph,
726:             ));
727:         }
728:     }
729:     let mut out = Vec::new();
730:     triples_of(g, None, &mut out);
731:     for (name, sub) in &g.named {
732:         triples_of(sub, Some(&name.to_string()), &mut out);
733:     }
734:     out.sort();
735:     out
736: }
737: 
738: /// The Oxigraph store as SORTED N-Quads lines, rendered identically.
739: fn oxi_nquads(store: &Store) -> Result<Vec<String>, String> {
740:     use oxigraph::model::GraphName;
741:     let mut out = Vec::new();
742:     for q in store.iter() {
743:         let q = q.map_err(|e| format!("oxigraph iter error: {}", e))?;
744:         let graph = match &q.graph_name {
745:             GraphName::DefaultGraph => None,
746:             g => Some(g.to_string()),
747:         };
748:         out.push(nquads_line(
749:             &q.subject.to_string(),
750:             &q.predicate.to_string(),
751:             &q.object.to_string(),
752:             graph.as_deref(),
753:         ));
754:     }
755:     out.sort();
756:     Ok(out)
757: }
758: 
759: // ── canonical comparison ─────────────────────────────────────────────────────────
760: 
761: /// Whether a snapshot mentions a blank node, i.e. whether sorted N-Quads has stopped
762: /// being a canonical form and RDFC-1.0 relabelling is required.
763: ///
764: /// Deliberately a substring test. It cannot produce a FALSE NEGATIVE — every blank
765: /// node renders as `_:label` — and a false positive (a literal containing the text
766: /// `_:`) would only route a blank-node-free snapshot through the canonicalizer, which
767: /// on such input is just a sort-and-deduplicate. The generator emits no such literal
768: /// today; the check stays conservative so that if one is ever added, the comparator
769: /// degrades in the safe direction.
770: fn mentions_blank_node(lines: &[String]) -> bool {
771:     lines.iter().any(|l| l.contains("_:"))
772: }
773: 
774: /// Re-parses N-Quads lines back into `oxrdf` quads (the input side of every
775: /// structural rewrite below).
776: fn parse_lines(lines: &[String], what: &str) -> Result<Vec<Quad>, String> {
777:     let doc = {
778:         let mut s = lines.join("\n");
779:         s.push('\n');
780:         s
781:     };
782:     sparq_canon::parse_nquads(&doc)
783:         .map_err(|e| format!("{}: N-Quads re-parse failed ({})", what, e))
784: }
785: 
786: /// The comparable form of one snapshot.
787: ///
788: /// With `relabel` false (neither side mentions a blank node) the sorted lines ARE the
789: /// canonical form and are returned untouched — the strongest, byte-level compare.
790: /// With it true, both sides are relabelled to their RDFC-1.0 canonical form so the
791: /// comparison decides RDF ISOMORPHISM. The constrained `*_ground_terms` profile is
792: /// used deliberately: it is exactly RDFC-1.0 with triple terms as opaque constants and
793: /// fails closed on a blank node nested inside a triple term, so the non-standard
794: /// nested-bnode descent is never reachable from this harness.
795: fn comparable(lines: &[String], relabel: bool, what: &str) -> Result<Vec<String>, String> {
796:     if !relabel {
797:         return Ok(lines.to_vec());
798:     }
799:     let quads = parse_lines(lines, what)?;
800:     let canon = sparq_canon::canonicalize_rdf12_ground_terms(&quads)
801:         .map_err(|e| format!("{}: RDFC-1.0 canonicalization failed ({})", what, e))?;
802:     let mut out: Vec<String> = canon.lines().map(str::to_string).collect();
803:     out.sort();
804:     Ok(out)
805: }
806: 
807: /// Rewrites `t` the way Oxigraph's storage layer does: an `xsd:integer` literal whose
808: /// value fits the native integer it decodes to is re-rendered in canonical lexical
809: /// form. Structural, never string surgery — and it descends into triple terms, which
810: /// Oxigraph's recursive term encoder also normalizes.
811: fn oxigraph_normalized_term(t: &Term) -> Term {
812:     match t {
813:         Term::Literal(l) if l.datatype() == xsd::INTEGER => match l.value().parse::<i64>() {
814:             Ok(v) if v.to_string() != l.value() => {
815:                 Term::Literal(Literal::new_typed_literal(v.to_string(), xsd::INTEGER))
816:             }
817:             _ => t.clone(),
818:         },
819:         Term::Triple(inner) => Term::Triple(Box::new(Triple::new(
820:             inner.subject.clone(),
821:             inner.predicate.clone(),
822:             oxigraph_normalized_term(&inner.object),
823:         ))),
824:         _ => t.clone(),
825:     }
826: }
827: 
828: // [GPT-6 ASTRA] Quad uniqueness alone cannot detect colliding terms at different
829: // predicates/graphs. Check every integer leaf, including nested triple objects.
830: fn record_lexical_terms(
831:     term: &Term,
832:     integers: &mut BTreeMap<i64, String>,
833:     blank_nodes: &mut BTreeSet<String>,
834: ) -> Result<(), String> {
835:     match term {
836:         Term::Literal(l) if l.datatype() == xsd::INTEGER => {
837:             if let Ok(value) = l.value().parse::<i64>() {
838:                 match integers.insert(value, l.value().to_string()) {
839:                     Some(previous) if previous != l.value() => {
840:                         return Err(format!(
841:                             "integer normalization is not term-injective: {previous:?} and {:?}",
842:                             l.value()
843:                         ));
844:                     }
845:                     _ => {}
846:                 }
847:             }
848:         }
849:         Term::BlankNode(b) => {
850:             blank_nodes.insert(b.as_str().to_string());
851:         }
852:         Term::Triple(t) => {
853:             if let oxrdf::NamedOrBlankNode::BlankNode(b) = &t.subject {
854:                 blank_nodes.insert(b.as_str().to_string());
855:             }
856:             record_lexical_terms(&t.object, integers, blank_nodes)?;
857:         }
858:         _ => {}
859:     }
860:     Ok(())
861: }
862: 
863: struct NormalizedSnapshot {
864:     lines: Vec<String>,
865:     blank_nodes: usize,
866: }
867: 
868: /// Normalizes integer spellings without merging terms or rows.
869: fn oxigraph_normalized(lines: &[String], what: &str) -> Result<NormalizedSnapshot, String> {
870:     let quads = parse_lines(lines, what)?;
871:     if quads.len() != lines.len() {
872:         return Err(format!("{what}: parsing changed the raw row count"));
873:     }
874:     let mut integers = BTreeMap::new();
875:     let mut blank_nodes = BTreeSet::new();
876:     for q in &quads {
877:         record_lexical_terms(&q.object, &mut integers, &mut blank_nodes)?;
878:         if let oxrdf::NamedOrBlankNode::BlankNode(b) = &q.subject {
879:             blank_nodes.insert(b.as_str().to_string());
880:         }
881:         if let oxrdf::GraphName::BlankNode(b) = &q.graph_name {
882:             blank_nodes.insert(b.as_str().to_string());
883:         }
884:     }
885:     let mut out: Vec<String> = quads
886:         .iter()
887:         .map(|q| {
888:             let graph = match &q.graph_name {
889:                 oxrdf::GraphName::DefaultGraph => None,
890:                 g => Some(g.to_string()),
891:             };
892:             nquads_line(
893:                 &q.subject.to_string(),
894:                 &q.predicate.to_string(),
895:                 &oxigraph_normalized_term(&q.object).to_string(),
896:                 graph.as_deref(),
897:             )
898:         })
899:         .collect();
900:     out.sort();
901:     // The fixed probes project whole quads, so duplicates are not legitimate
902:     // projection multiplicity. Reject before canon's set conversion can hide them.
903:     if out.windows(2).any(|rows| rows[0] == rows[1]) {
904:         return Err(format!(
905:             "{what}: integer normalization merges or duplicates rows"
906:         ));
907:     }
908:     Ok(NormalizedSnapshot {
909:         lines: out,
910:         blank_nodes: blank_nodes.len(),
911:     })
912: }
913: 
914: /// The lines on exactly one side — the human-readable core of a divergence report.
915: fn one_sided(label_a: &str, a: &[String], label_b: &str, b: &[String]) -> String {
916:     let bset: std::collections::BTreeSet<&String> = b.iter().collect();
917:     let aset: std::collections::BTreeSet<&String> = a.iter().collect();
918:     let mut s = String::new();
919:     s.push_str(&format!("only in {}:\n", label_a));
920:     for l in a.iter().filter(|l| !bset.contains(l)) {
921:         s.push_str(&format!("  {}\n", l));
922:     }
923:     s.push_str(&format!("only in {}:\n", label_b));
924:     for l in b.iter().filter(|l| !aset.contains(l)) {
925:         s.push_str(&format!("  {}\n", l));
926:     }
927:     s
928: }
929: 
930: /// The outcome of comparing two snapshots.
931: enum Verdict {
932:     /// Byte-identical, or RDF-isomorphic when blank nodes are in play.
933:     Same,
934:     /// Absorbed by the adjudicated `update-oxigraph-integer-lexical-canonicalization`
935:     /// class: the two datasets agree exactly once Oxigraph's numeric normalization is
936:     /// re-derived on both sides.
937:     AdjudicatedIntegerLexical,
938:     /// A real divergence (the string is the report body).
939:     Differs(String),
940: }
941: 
942: /// Compares two snapshots under the canonical form the module docs describe.
943: ///
944: /// `allow_integer_lexical` is set ONLY for a sparq-vs-Oxigraph compare, and only when
945: /// the allowlist enables the class. The sparq-vs-sparq compare passes false: both
946: /// sides are sparq, so any lexical disagreement between them is a real bug.
947: fn compare(
948:     label_a: &str,
949:     a: &[String],
950:     label_b: &str,
951:     b: &[String],
952:     allow_integer_lexical: bool,
953: ) -> Verdict {
954:     let relabel = mentions_blank_node(a) || mentions_blank_node(b);
955:     let (ca, cb) = match (
956:         comparable(a, relabel, label_a),
957:         comparable(b, relabel, label_b),
958:     ) {
959:         (Ok(ca), Ok(cb)) => (ca, cb),
960:         (Err(e), _) | (_, Err(e)) => return Verdict::Differs(e),
961:     };
962:     if ca == cb {
963:         // Canonicalization deduplicates, so a duplicate quad on one side alone would
964:         // survive the compare — check the raw counts to keep that failure visible.
965:         if a.len() != b.len() {
966:             return Verdict::Differs(format!(
967:                 "datasets are isomorphic but the raw quad COUNTS differ \
968:                  ({} {} vs {} {}) — one side is yielding a duplicate quad",
969:                 label_a,
970:                 a.len(),
971:                 label_b,
972:                 b.len()
973:             ));
974:         }
975:         return Verdict::Same;
976:     }
977:     if allow_integer_lexical {
978:         if a.len() != b.len() {
979:             return Verdict::Differs(format!(
980:                 "integer-lexical adjudication refused: raw row counts differ\n{}",
981:                 one_sided(label_a, &ca, label_b, &cb)
982:             ));
983:         }
984:         let normalized = (
985:             oxigraph_normalized(a, label_a),
986:             oxigraph_normalized(b, label_b),
987:         );
988:         match normalized {
989:             (Ok(na), Ok(nb)) => {
990:                 if na.blank_nodes != nb.blank_nodes {
991:                     return Verdict::Differs(format!(
992:                         "integer-lexical adjudication refused: blank-node counts differ ({} vs {})\n{}",
993:                         na.blank_nodes,
994:                         nb.blank_nodes,
995:                         one_sided(label_a, &ca, label_b, &cb)
996:                     ));
997:                 }
998:                 match (
999:                     comparable(&na.lines, relabel, label_a),
1000:                     comparable(&nb.lines, relabel, label_b),
1001:                 ) {
1002:                     (Ok(na), Ok(nb)) if na == nb => return Verdict::AdjudicatedIntegerLexical,
1003:                     (Err(e), _) | (_, Err(e)) => {
1004:                         return Verdict::Differs(format!(
1005:                             "integer-lexical adjudication refused: {e}\n{}",
1006:                             one_sided(label_a, &ca, label_b, &cb)
1007:                         ));
1008:                     }
1009:                     _ => {}
1010:                 }
1011:             }
1012:             (Err(e), _) | (_, Err(e)) => {
1013:                 return Verdict::Differs(format!(
1014:                     "integer-lexical adjudication refused: {e}\n{}",
1015:                     one_sided(label_a, &ca, label_b, &cb)
1016:                 ));
1017:             }
1018:         }
1019:     }
1020:     Verdict::Differs(one_sided(label_a, &ca, label_b, &cb))
1021: }
1022: 
1023: // ── probe SELECTs (full binding-set compare, per the oracle-strength rule) ────────
1024: //
```

## all-production-callers.txt

```rust
crates/sparq-bench/src/update_fuzz.rs:1025-1091
1025: // The projection order is N-QUADS ORDER (`?s ?p ?o [?g]`) on purpose: a row rendered
1026: // by joining its cells is then a well-formed N-Quads line, so probe results go through
1027: // exactly the same canonical comparison as the dataset snapshots — which is what makes
1028: // them comparable at all once blank nodes are in play.
1029: 
1030: const PROBES: &[(&str, &[&str])] = &[
1031:     ("SELECT ?s ?p ?o WHERE { ?s ?p ?o }", &["s", "p", "o"]),
1032:     (
1033:         "SELECT ?s ?p ?o ?g WHERE { GRAPH ?g { ?s ?p ?o } }",
1034:         &["s", "p", "o", "g"],
1035:     ),
1036: ];
1037: 
1038: /// Renders one probe row's cells as an N-Quads line.
1039: fn probe_line(cells: &[String]) -> String {
1040:     format!("{} .", cells.join(" "))
1041: }
1042: 
1043: /// The full sorted binding set of a probe through sparq's query path.
1044: fn sparq_probe(g: &Graph, q: &str) -> Result<Vec<String>, String> {
1045:     let r = sparq_engine::query(g, q).map_err(|e| format!("sparq probe error: {}", e))?;
1046:     let mut rows: Vec<String> = r
1047:         .rows
1048:         .iter()
1049:         .map(|row| {
1050:             probe_line(
1051:                 &row.iter()
1052:                     .map(|t| {
1053:                         t.as_ref()
1054:                             .map(|t| t.to_string())
1055:                             .unwrap_or_else(|| "UNDEF".to_string())
1056:                     })
1057:                     .collect::<Vec<_>>(),
1058:             )
1059:         })
1060:         .collect();
1061:     rows.sort();
1062:     Ok(rows)
1063: }
1064: 
1065: /// The full sorted binding set of a probe through Oxigraph's query path.
1066: // clippy: the differential oracle pins oxigraph's legacy Store::query semantics
1067: #[allow(deprecated)]
1068: fn oxi_probe(store: &Store, q: &str, vars: &[&str]) -> Result<Vec<String>, String> {
1069:     match store.query(q).map_err(|e| e.to_string())? {
1070:         oxigraph::sparql::QueryResults::Solutions(s) => {
1071:             let mut rows = Vec::new();
1072:             for sol in s {
1073:                 let sol = sol.map_err(|e| e.to_string())?;
1074:                 rows.push(probe_line(
1075:                     &vars
1076:                         .iter()
1077:                         .map(|v| {
1078:                             sol.get(*v)
1079:                                 .map(|t| t.to_string())
1080:                                 .unwrap_or_else(|| "UNDEF".to_string())
1081:                         })
1082:                         .collect::<Vec<_>>(),
1083:                 ));
1084:             }
1085:             rows.sort();
1086:             Ok(rows)
1087:         }
1088:         _ => Err("probe did not return solutions".to_string()),
1089:     }
1090: }
1091: 

crates/sparq-bench/src/update_fuzz.rs:1150-1323
1150: /// canonical dataset + both probes after every step. Returns the run's counts, or the
1151: /// divergence detail (step-localized).
1152: ///
1153: /// `inject_divergence_at`: test-only comparator non-vacuity knob — after applying
1154: /// step `i` to all three engines, a marker quad is inserted into the OXIGRAPH store
1155: /// only, so the comparator MUST report a divergence at that step (see
1156: /// `tests::injected_divergence_is_caught`). `None` in production.
1157: fn check_seed(
1158:     seed: u64,
1159:     inject_divergence_at: Option<usize>,
1160:     allow: &UpdateDivergenceAllowlist,
1161: ) -> Result<SeedOutcome, String> {
1162:     let mut rng = Rng::new(seed);
1163:     let ops = gen_sequence(&mut rng);
1164:     let sandbox = if ops.iter().any(|o| o.sparq.starts_with("LOAD")) {
1165:         Some(LoadSandbox::new()?)
1166:     } else {
1167:         None
1168:     };
1169:     match sandbox.as_ref() {
1170:         // The allowlisted base is installed for the whole seed: `with_load_base` is a
1171:         // thread-local guard, and a seed's steps all run on this thread.
1172:         Some(s) => sparq_engine::with_load_base(s.path(), || {
1173:             apply_sequence(seed, &ops, Some(s), inject_divergence_at, allow)
1174:         }),
1175:         None => apply_sequence(seed, &ops, None, inject_divergence_at, allow),
1176:     }
1177: }
1178: 
1179: fn apply_sequence(
1180:     seed: u64,
1181:     ops: &[Op],
1182:     sandbox: Option<&LoadSandbox>,
1183:     inject_divergence_at: Option<usize>,
1184:     allow: &UpdateDivergenceAllowlist,
1185: ) -> Result<SeedOutcome, String> {
1186:     let mut g_rebuild = Graph::new();
1187:     let mut g_inplace = Graph::new();
1188:     let store = Store::new().map_err(|e| format!("oxigraph store init: {}", e))?;
1189:     let mut adjudicated_integer_lexical = 0u64;
1190:     let mut isomorphism_compares = 0u64;
1191: 
1192:     let fail = |step: usize, op: &Op, detail: String| -> String {
1193:         format!(
1194:             "step={} of {}\nop: {}\n{}\nrepro: cargo run -p sparq-bench --release -- \
1195:              update-fuzz --seed-start {} --seed-count 1\n--- full sequence ---\n{}",
1196:             step,
1197:             ops.len(),
1198:             op.sparq,
1199:             detail,
1200:             seed,
1201:             ops.iter()
1202:                 .enumerate()
1203:                 .map(|(i, o)| match &o.oxi {
1204:                     Some(x) if *x != o.sparq =>
1205:                         format!("[{}] {}\n     (reference engine ran: {})", i, o.sparq, x),
1206:                     Some(_) => format!("[{}] {}", i, o.sparq),
1207:                     None => format!("[{}] {}\n     (reference engine ran: nothing)", i, o.sparq),
1208:                 })
1209:                 .collect::<Vec<_>>()
1210:                 .join("\n")
1211:         )
1212:     };
1213: 
1214:     for (i, op) in ops.iter().enumerate() {
1215:         // A LOAD's document must exist before the request runs.
1216:         if let (Some(doc), Some(s)) = (&op.doc, sandbox) {
1217:             s.write(doc).map_err(|e| fail(i, op, e))?;
1218:         }
1219: 
1220:         // Apply to all three implementations. Every generated op is inside the
1221:         // supported deterministic subset, so an error from ANY engine is itself a
1222:         // divergence (strict — there is no unsupported-skip in this harness).
1223:         g_rebuild = sparq_engine::update(&g_rebuild, &op.sparq)
1224:             .map_err(|e| fail(i, op, format!("sparq update (rebuild path) error: {}", e)))?;
1225:         sparq_engine::update_in_place(&mut g_inplace, &op.sparq)
1226:             .map_err(|e| fail(i, op, format!("sparq update_in_place error: {}", e)))?;
1227:         if let Some(oxi) = &op.oxi {
1228:             store
1229:                 .update(oxi.as_str())
1230:                 .map_err(|e| fail(i, op, format!("oxigraph update error: {}", e)))?;
1231:         }
1232: 
1233:         if inject_divergence_at == Some(i) {
1234:             use oxigraph::model::{GraphName, NamedNode, Quad};
1235:             let n = |s: &str| NamedNode::new(s).expect("valid IRI");
1236:             let marker = Quad::new(
1237:                 n("http://ex/injected"),
1238:                 n("http://ex/injected"),
1239:                 n("http://ex/injected"),
1240:                 GraphName::DefaultGraph,
1241:             );
1242:             store
1243:                 .insert(&marker)
1244:                 .map_err(|e| format!("marker insert failed: {}", e))?;
1245:         }
1246: 
1247:         // (a) Canonical dataset equality. Localizes a divergence to this exact step.
1248:         // sparq-vs-sparq FIRST and STRICT (no adjudication): the two sparq paths must
1249:         // agree with each other whatever the reference engine does.
1250:         let nq_rebuild = sparq_nquads(&g_rebuild);
1251:         let nq_inplace = sparq_nquads(&g_inplace);
1252:         let nq_oxi = oxi_nquads(&store).map_err(|e| fail(i, op, e))?;
1253:         if mentions_blank_node(&nq_rebuild) || mentions_blank_node(&nq_oxi) {
1254:             isomorphism_compares += 1;
1255:         }
1256:         if let Verdict::Differs(detail) = compare(
1257:             "sparq(rebuild)",
1258:             &nq_rebuild,
1259:             "sparq(in-place)",
1260:             &nq_inplace,
1261:             false,
1262:         ) {
1263:             return Err(fail(
1264:                 i,
1265:                 op,
1266:                 format!(
1267:                     "canonical dataset differs BETWEEN SPARQ'S OWN UPDATE PATHS\n{}",
1268:                     detail
1269:                 ),
1270:             ));
1271:         }
1272:         for (label, nq) in [
1273:             ("sparq(rebuild)", &nq_rebuild),
1274:             ("sparq(in-place)", &nq_inplace),
1275:         ] {
1276:             match compare(label, nq, "oxigraph", &nq_oxi, allow.integer_lexical) {
1277:                 Verdict::Same => {}
1278:                 Verdict::AdjudicatedIntegerLexical => adjudicated_integer_lexical += 1,
1279:                 Verdict::Differs(detail) => {
1280:                     return Err(fail(
1281:                         i,
1282:                         op,
1283:                         format!(
1284:                             "canonical dataset differs ({} vs oxigraph)\n{}",
1285:                             label, detail
1286:                         ),
1287:                     ));
1288:                 }
1289:             }
1290:         }
1291: 
1292:         // (b) Probe SELECTs — the query-path view of the updated store, full sorted
1293:         // binding sets (never counts). Checked for BOTH sparq graphs: the in-place
1294:         // one reads through the live delta overlay.
1295:         for (probe, vars) in PROBES {
1296:             let oxi = oxi_probe(&store, probe, vars)
1297:                 .map_err(|e| fail(i, op, format!("oxigraph probe error: {}", e)))?;
1298:             for (label, g) in [("rebuild", &g_rebuild), ("in-place", &g_inplace)] {
1299:                 let sparq = sparq_probe(g, probe).map_err(|e| fail(i, op, e))?;
1300:                 match compare("sparq", &sparq, "oxigraph", &oxi, allow.integer_lexical) {
1301:                     Verdict::Same => {}
1302:                     Verdict::AdjudicatedIntegerLexical => adjudicated_integer_lexical += 1,
1303:                     Verdict::Differs(detail) => {
1304:                         return Err(fail(
1305:                             i,
1306:                             op,
1307:                             format!(
1308:                                 "probe {:?} binding set differs (sparq {} vs oxigraph)\n{}",
1309:                                 probe, label, detail
1310:                             ),
1311:                         ));
1312:                     }
1313:                 }
1314:             }
1315:         }
1316:     }
1317:     Ok(SeedOutcome {
1318:         ops: ops.len() as u64,
1319:         adjudicated_integer_lexical,
1320:         isomorphism_compares,
1321:     })
1322: }
1323: 

crates/sparq-bench/src/main.rs:21-22
21: mod neutral;
22: mod update_fuzz;

crates/sparq-bench/src/main.rs:73-83
73:     // `sparq-bench update-fuzz --seed-start N --seed-count M` — SPARQL UPDATE
74:     // differential vs Oxigraph (sq-3dyje.4): random ground-term update sequences
75:     // through both sparq update paths + Oxigraph, canonical per-step compare.
76:     if args.get(1).map(String::as_str) == Some("update-fuzz") {
77:         let seed_start: u64 =
78:             arg_val(&args, "--seed-start").and_then(|s| s.parse().ok()).unwrap_or(0);
79:         let count: u64 =
80:             arg_val(&args, "--seed-count").and_then(|s| s.parse().ok()).unwrap_or(1000);
81:         update_fuzz::run(seed_start, count);
82:         return;
83:     }
```

## canonicalization-apis.txt

```rust
crates/sparq-canon/src/lib.rs:112-150
112: pub fn canonicalize_nquads(input: &str) -> Result<String, CanonError> {
113:     let quads = parse_nquads_03(input)?;
114:     canonicalize_quads(&quads)
115: }
116: 
117: /// Parses an oxrdf-0.3 N-Quads document into [`Quad`]s (the input side of
118: /// [`canonicalize_nquads`]). Surfaced so a caller that needs the parsed quads
119: /// (e.g. to also issue identifiers) does not re-implement the parse.
120: pub fn parse_nquads(input: &str) -> Result<Vec<Quad>, CanonError> {
121:     parse_nquads_03(input)
122: }
123: 
124: fn parse_nquads_03(input: &str) -> Result<Vec<Quad>, CanonError> {
125:     let mut quads = Vec::new();
126:     for item in NQuadsParser::new().for_slice(input.as_bytes()) {
127:         quads.push(item.map_err(|e| CanonError::Bridge(e.to_string()))?);
128:     }
129:     Ok(quads)
130: }
131: 
132: /// **NON-STANDARD, opt-in (`rdf12-triple-terms` feature).** Native RDF-1.2
133: /// triple-term canonicalization profile — see the [module docs](rdf12) and the
134: /// crate-level banner. Not W3C RDFC-1.0.
135: #[cfg(feature = "rdf12-triple-terms")]
136: pub mod rdf12;
137: 
138: #[cfg(feature = "rdf12-triple-terms")]
139: pub use rdf12::{
140:     canonicalize_graph_content_rdf12, canonicalize_graph_content_rdf12_ground_terms,
141:     canonicalize_graph_content_rdf12_ground_terms_with, canonicalize_graph_content_rdf12_with,
142:     canonicalize_rdf12,
143:     canonicalize_rdf12_ground_terms, canonicalize_rdf12_ground_terms_with, canonicalize_rdf12_with,
144:     canonicalize_triples_rdf12, canonicalize_triples_rdf12_ground_terms,
145:     canonicalize_triples_rdf12_ground_terms_with, canonicalize_triples_rdf12_with,
146:     issue_dataset_rdf12, issue_dataset_rdf12_ground_terms, issue_dataset_rdf12_ground_terms_with,
147:     issue_dataset_rdf12_with,
148: };
149: 
150: // **Opt-in (`concept` feature).** `urn:concept:` content-addressed record

crates/sparq-canon/src/lib.rs:302-366
302: /// assert!(canon.contains("_:c14n0"));
303: /// ```
304: pub fn canonicalize(dataset: &[Quad]) -> Result<String, CanonError> {
305:     canonicalize_quads(dataset)
306: }
307: 
308: /// Alias of [`canonicalize`] for callers that prefer the explicit `_quads`
309: /// name (mirrors [`rdf_canon::canonicalize_quads`]).
310: pub fn canonicalize_quads(dataset: &[Quad]) -> Result<String, CanonError> {
311:     let quads02 = bridge_to_02(dataset)?;
312:     rdf_canon::canonicalize_quads(&quads02).map_err(|e| CanonError::Canonicalization(e.to_string()))
313: }
314: 
315: /// Like [`canonicalize_quads`] but parameterized over the RDFC-1.0 hash
316: /// function `D` (the spec default is SHA-256; e.g. `sha2::Sha384` selects the
317: /// SHA-384 profile). Uses the default HNDQ call limit.
318: pub fn canonicalize_quads_with<D: Digest>(dataset: &[Quad]) -> Result<String, CanonError> {
319:     let quads02 = bridge_to_02(dataset)?;
320:     let opts = rdf_canon::CanonicalizationOptions::default();
321:     rdf_canon::canonicalize_quads_with::<D>(&quads02, &opts)
322:         .map_err(|e| CanonError::Canonicalization(e.to_string()))
323: }
324: 
325: /// Returns the digest bytes of the exact canonical N-Quads document produced
326: /// by [`canonicalize_quads`].
327: ///
328: /// `D` selects only the final digest algorithm; canonicalization retains the
329: /// RDFC-1.0 default hash profile. Every canonical byte is hashed, including the
330: /// final trailing newline when the dataset is non-empty. [GPT-5.6] sq-ddws7.
331: pub fn digest_quads_with<D: Digest>(dataset: &[Quad]) -> Result<Vec<u8>, CanonError> {
332:     let c = canonicalize_quads(dataset)?;
333:     let mut h = D::new();
334:     h.update(c.as_bytes());
335:     Ok(h.finalize().to_vec())
336: }
337: 
338: /// Like [`issue_quads`] but parameterized over the RDFC-1.0 hash function `D`.
339: pub fn issue_quads_with<D: Digest>(
340:     dataset: &[Quad],
341: ) -> Result<HashMap<String, String>, CanonError> {
342:     let quads02 = bridge_to_02(dataset)?;
343:     let opts = rdf_canon::CanonicalizationOptions::default();
344:     let map = rdf_canon::issue_quads_with::<D>(&quads02, &opts)
345:         .map_err(|e| CanonError::Canonicalization(e.to_string()))?;
346:     Ok(map.into_iter().collect())
347: }
348: 
349: /// Returns the RDFC-1.0 **issued-identifier map** for a dataset: input
350: /// blank-node label → canonical `c14nN` label. Cheap relative to a full
351: /// canonicalization-and-reparse when only the relabelling is needed.
352: pub fn issued_identifiers(dataset: &[Quad]) -> Result<HashMap<String, String>, CanonError> {
353:     issue_quads(dataset)
354: }
355: 
356: /// Alias of [`issued_identifiers`] (mirrors [`rdf_canon::issue_quads`]).
357: pub fn issue_quads(dataset: &[Quad]) -> Result<HashMap<String, String>, CanonError> {
358:     let quads02 = bridge_to_02(dataset)?;
359:     let map = rdf_canon::issue_quads(&quads02)
360:         .map_err(|e| CanonError::Canonicalization(e.to_string()))?;
361:     Ok(map.into_iter().collect())
362: }
363: 
364: // ---------------------------------------------------------------------------
365: // Single-graph API (a default-graph-only dataset). What the ZK per-graph
366: // commitment pipeline consumes; kept here so the bridge is single-sourced.

crates/sparq-canon/src/rdf12.rs:175-222
175: /// On triple-term-free input this is byte-identical to the standard
176: /// [`crate::canonicalize`]. With triple terms it additionally relabels blank
177: /// nodes nested inside triple-term objects via the HNDQ descent.
178: ///
179: /// This is **not** W3C RDFC-1.0; W3C has published no RDF-1.2 dataset
180: /// canonicalization specification. See the [module docs](self).
181: pub fn canonicalize_rdf12(dataset: &[Quad]) -> Result<String, CanonError> {
182:     canonicalize_rdf12_with::<Sha256>(dataset)
183: }
184: 
185: /// **NON-STANDARD.** Like [`canonicalize_rdf12`] but parameterized over the
186: /// RDFC-1.0 hash function `D` ([`crate::Digest`]). The profile default is
187: /// SHA-256 ([`canonicalize_rdf12`]); pass `sha2::Sha384` to select the SHA-384
188: /// profile, for parity with the standard delegated path's
189: /// [`crate::canonicalize_quads_with`].
190: ///
191: /// The relabelling and line order are determined by hash comparison, so a
192: /// different `D` may produce a different (but still canonical and
193: /// isomorphism-stable) `c14nN` assignment; the SHA-256 default
194: /// ([`canonicalize_rdf12`]) is byte-identical to before.
195: pub fn canonicalize_rdf12_with<D: Digest>(dataset: &[Quad]) -> Result<String, CanonError> {
196:     let issued = issue_dataset_rdf12_with::<D>(dataset)?;
197:     Ok(serialize_canonical(dataset, &issued))
198: }
199: 
200: /// **NON-STANDARD.** The issued-identifier map (input blank-node label →
201: /// canonical `c14nN` label) for an RDF-1.2 dataset under the
202: /// `rdf12-triple-terms` profile (SHA-256). See [`canonicalize_rdf12`].
203: pub fn issue_dataset_rdf12(
204:     dataset: &[Quad],
205: ) -> Result<std::collections::HashMap<String, String>, CanonError> {
206:     issue_dataset_rdf12_with::<Sha256>(dataset)
207: }
208: 
209: /// **NON-STANDARD.** Like [`issue_dataset_rdf12`] but parameterized over the
210: /// RDFC-1.0 hash function `D` ([`crate::Digest`]). See
211: /// [`canonicalize_rdf12_with`].
212: pub fn issue_dataset_rdf12_with<D: Digest>(
213:     dataset: &[Quad],
214: ) -> Result<std::collections::HashMap<String, String>, CanonError> {
215:     // Crate-wide nesting-depth bound, checked iteratively BEFORE any recursive
216:     // descent (this is the funnel every full-profile entry point drains
217:     // through). [FABLE-5] sq-x3oj2.
218:     ensure_triple_term_depth(dataset)?;
219:     let state = CanonState::compute::<D>(dataset)?;
220:     Ok(state.canonical_issuer.issued.into_iter().collect())
221: }
222: 

crates/sparq-canon/src/rdf12.rs:302-356
302: 
303: // ---------------------------------------------------------------------------
304: // Constrained ground-triple-term variant ([FABLE-5] sq-iaxd): thin wrappers
305: // that fail closed on any blank node nested inside a triple term, then
306: // delegate. With every triple term ground, the nested-bnode HNDQ-descent
307: // extension above is unreachable, so these entry points exercise exactly the
308: // RDFC-1.0 algorithm with triple terms as opaque constants (see module docs).
309: // ---------------------------------------------------------------------------
310: 
311: /// **NON-STANDARD (constrained).** Like [`canonicalize_rdf12`], but requires
312: /// every triple term to be **ground** (no blank node at any nesting depth) and
313: /// fails closed with [`CanonError::NestedBlankNode`] otherwise — the common
314: /// credential/VC case. Top-level blank nodes are ordinary RDFC-1.0 bnodes and
315: /// are relabelled as usual. On accepted input the output is byte-identical to
316: /// [`canonicalize_rdf12`] (this is a thin guard + delegate wrapper), and the
317: /// nested-bnode HNDQ-descent extension is never exercised.
318: pub fn canonicalize_rdf12_ground_terms(dataset: &[Quad]) -> Result<String, CanonError> {
319:     canonicalize_rdf12_ground_terms_with::<Sha256>(dataset)
320: }
321: 
322: /// **NON-STANDARD (constrained).** Like [`canonicalize_rdf12_ground_terms`] but
323: /// parameterized over the RDFC-1.0 hash function `D` ([`crate::Digest`]); see
324: /// [`canonicalize_rdf12_with`] for the hash-profile semantics.
325: pub fn canonicalize_rdf12_ground_terms_with<D: Digest>(
326:     dataset: &[Quad],
327: ) -> Result<String, CanonError> {
328:     // Depth-bound first: the ground guard walks nested terms, so an over-deep
329:     // term reports `TripleTermDepthExceeded`, not `NestedBlankNode`.
330:     ensure_triple_term_depth(dataset)?;
331:     ensure_ground_triple_terms(dataset)?;
332:     canonicalize_rdf12_with::<D>(dataset)
333: }
334: 
335: /// **NON-STANDARD (constrained).** Like [`issue_dataset_rdf12`], but fails
336: /// closed with [`CanonError::NestedBlankNode`] if any blank node occurs inside
337: /// a triple term. See [`canonicalize_rdf12_ground_terms`].
338: pub fn issue_dataset_rdf12_ground_terms(
339:     dataset: &[Quad],
340: ) -> Result<std::collections::HashMap<String, String>, CanonError> {
341:     issue_dataset_rdf12_ground_terms_with::<Sha256>(dataset)
342: }
343: 
344: /// **NON-STANDARD (constrained).** Like [`issue_dataset_rdf12_ground_terms`]
345: /// but parameterized over the RDFC-1.0 hash function `D` ([`crate::Digest`]).
346: pub fn issue_dataset_rdf12_ground_terms_with<D: Digest>(
347:     dataset: &[Quad],
348: ) -> Result<std::collections::HashMap<String, String>, CanonError> {
349:     // Depth-bound first — same ordering rationale as
350:     // `canonicalize_rdf12_ground_terms_with`.
351:     ensure_triple_term_depth(dataset)?;
352:     ensure_ground_triple_terms(dataset)?;
353:     issue_dataset_rdf12_with::<D>(dataset)
354: }
355: 
356: /// **NON-STANDARD (constrained).** Like [`canonicalize_triples_rdf12`], but

crates/sparq-canon/src/rdf12.rs:407-483
407: /// Top-level subject/object/graph blank nodes do not count — those are
408: /// ordinary RDFC-1.0 blank nodes.
409: fn ensure_ground_triple_terms(dataset: &[Quad]) -> Result<(), CanonError> {
410:     if dataset.iter().any(|q| term_has_nested_bnode(&q.object)) {
411:         return Err(CanonError::NestedBlankNode);
412:     }
413:     Ok(())
414: }
415: 
416: /// True iff `term` is a triple term containing a blank node at any depth.
417: /// A top-level `Term::BlankNode` is NOT nested and returns `false`.
418: fn term_has_nested_bnode(term: &Term) -> bool {
419:     match term {
420:         Term::Triple(t) => triple_contains_bnode(t),
421:         _ => false,
422:     }
423: }
424: 
425: /// True iff the (triple-term) triple contains a blank node in its subject or
426: /// (transitively) its object. Predicates are always IRIs; in oxrdf 0.3 a
427: /// triple's subject is `NamedOrBlankNode`, so only the object chain descends —
428: /// walked with a **loop**, not recursion, so the walk itself is stack-safe on
429: /// any depth (the entry points additionally bound depth up front; see
430: /// [`ensure_triple_term_depth`]). [FABLE-5] sq-x3oj2.
431: fn triple_contains_bnode(t: &Triple) -> bool {
432:     let mut cur = t;
433:     loop {
434:         if subject_bnode(&cur.subject).is_some() {
435:             return true;
436:         }
437:         match &cur.object {
438:             Term::BlankNode(_) => return true,
439:             Term::Triple(inner) => cur = inner,
440:             _ => return false,
441:         }
442:     }
443: }
444: 
445: // ---------------------------------------------------------------------------
446: // Crate-wide triple-term nesting-depth bound ([FABLE-5] sq-x3oj2): the profile
447: // walks triple terms with recursive descent (HNDQ gossip, bnode collection,
448: // relabelling, `Display` serialization — and oxrdf's own `Drop`/`Clone`
449: // recurse), so unbounded nesting is a stack-overflow vector. Every public
450: // entry point pre-checks depth ITERATIVELY (the checker itself cannot
451: // overflow) and fails closed before any recursion or deep `clone` happens.
452: // ---------------------------------------------------------------------------
453: 
454: /// The nesting depth of a term: 0 for a non-triple term; a triple term whose
455: /// object is not itself a triple term has depth 1; each further level adds 1.
456: /// In oxrdf 0.3 only a triple's **object** can be a triple term (the subject is
457: /// [`NamedOrBlankNode`]), so nesting is a single chain — walked with a loop.
458: fn triple_term_depth(term: &Term) -> usize {
459:     let mut depth = 0usize;
460:     let mut cur = term;
461:     while let Term::Triple(t) = cur {
462:         // The subject term of `t` contributes nothing to the nesting depth —
463:         // `subject_nesting_depth` is what pins that (and what breaks the build
464:         // if it ever stops being true; [OPUS-5] sq-tx21), so accumulating down
465:         // the object chain alone measures the whole term.
466:         depth += 1 + subject_nesting_depth(&t.subject);
467:         cur = &t.object;
468:     }
469:     depth
470: }
471: 
472: /// Depth guard over a dataset: `Err(TripleTermDepthExceeded)` iff any quad's
473: /// object nests triple terms deeper than [`crate::MAX_TRIPLE_TERM_DEPTH`].
474: fn ensure_triple_term_depth(dataset: &[Quad]) -> Result<(), CanonError> {
475:     if dataset
476:         .iter()
477:         .any(|q| triple_term_depth(&q.object) > crate::MAX_TRIPLE_TERM_DEPTH)
478:     {
479:         return Err(CanonError::TripleTermDepthExceeded);
480:     }
481:     Ok(())
482: }
483: 

crates/sparq-canon/src/rdf12.rs:545-601
545: struct CanonState {
546:     /// bnode label → the quads it is a component of (recursively through
547:     /// triple terms). The same quad is recorded once per *distinct* bnode it
548:     /// contains, matching the standard algorithm.
549:     bnode_to_quads: BTreeMap<String, Vec<Quad>>,
550:     canonical_issuer: IdentifierIssuer,
551: }
552: 
553: impl CanonState {
554:     fn compute<D: Digest>(dataset: &[Quad]) -> Result<Self, CanonError> {
555:         let mut state = CanonState {
556:             bnode_to_quads: BTreeMap::new(),
557:             canonical_issuer: IdentifierIssuer::new("c14n"),
558:         };
559:         state.build_bnode_to_quads(dataset);
560: 
561:         // §4.4(3) first-degree hashes.
562:         let mut hash_to_bnodes: HashToBnodes = BTreeMap::new();
563:         for n in state.bnode_to_quads.keys() {
564:             let h = state.hash_first_degree_quads::<D>(n)?;
565:             hash_to_bnodes.entry(h).or_default().push(n.clone());
566:         }
567: 
568:         // §4.4(4) unique first-degree hashes → issue canonical labels.
569:         let mut shared: HashToBnodes = BTreeMap::new();
570:         for (h, list) in &hash_to_bnodes {
571:             if list.len() == 1 {
572:                 state.canonical_issuer.issue(&list[0]);
573:             } else {
574:                 shared.insert(h.clone(), list.clone());
575:             }
576:         }
577: 
578:         // §4.4(5) shared hashes → HNDQ.
579:         let mut counter = HndqCallCounter::new(DEFAULT_HNDQ_CALL_LIMIT);
580:         for list in shared.values() {
581:             let mut hash_path_list: Vec<HndqResult> = Vec::new();
582:             for n in list {
583:                 if state.canonical_issuer.get(n).is_some() {
584:                     continue;
585:                 }
586:                 let mut temp = IdentifierIssuer::new("b");
587:                 temp.issue(n);
588:                 let result = state.hash_n_degree_quads::<D>(n, &temp, &mut counter)?;
589:                 hash_path_list.push(result);
590:             }
591:             hash_path_list.sort_by(|a, b| a.hash.cmp(&b.hash));
592:             for result in &hash_path_list {
593:                 // Issue canonical labels in temporary-issuance order (§4.4 5.3.1).
594:                 for existing in result.issuer.order.values() {
595:                     state.canonical_issuer.issue(existing);
596:                 }
597:             }
598:         }
599: 
600:         Ok(state)
601:     }

crates/sparq-canon/src/rdf12.rs:915-1010
915: /// §5 serialize: relabel every bnode to its canonical `c14nN`, sort the lines
916: /// in code-point order, concatenate (one `\n`-terminated line per quad).
917: fn serialize_canonical(
918:     dataset: &[Quad],
919:     issued: &std::collections::HashMap<String, String>,
920: ) -> String {
921:     let mut doc = String::new();
922:     for line in canonical_lines(dataset, issued) {
923:         doc.push_str(&line);
924:         doc.push('\n');
925:     }
926:     doc
927: }
928: 
929: /// The canonical N-Quads lines (each `… .`, NO trailing newline — matching the
930: /// standard `CanonicalGraph::lines` contract), sorted in code-point order and
931: /// **deduplicated**: RDF is a set, so identical quads (which canonicalize to the
932: /// same line) collapse, exactly as the `rdf-canon` `Dataset` path does.
933: fn canonical_lines(
934:     dataset: &[Quad],
935:     issued: &std::collections::HashMap<String, String>,
936: ) -> Vec<String> {
937:     let mut lines: Vec<String> = dataset
938:         .iter()
939:         .map(|q| {
940:             let relabelled = Quad::new(
941:                 relabel_subject_canonical(&q.subject, issued),
942:                 q.predicate.clone(),
943:                 relabel_term_canonical(&q.object, issued),
944:                 relabel_graph_canonical(&q.graph_name, issued),
945:             );
946:             canonical_line_no_newline(&relabelled)
947:         })
948:         .collect();
949:     lines.sort();
950:     lines.dedup();
951:     lines
952: }
953: 
954: /// A canonical N-Quads line WITHOUT the trailing ` .\n` separator — the
955: /// `CanonicalGraph::lines` form. (The hashing path uses [`serialize_quad_line`],
956: /// which keeps the ` .\n` the RDFC-1.0 spec hashes over.)
957: fn canonical_line_no_newline(quad: &Quad) -> String {
958:     match &quad.graph_name {
959:         GraphName::DefaultGraph => {
960:             format!("{} {} {} .", quad.subject, quad.predicate, quad.object)
961:         }
962:         graph => format!(
963:             "{} {} {} {} .",
964:             quad.subject, quad.predicate, quad.object, graph
965:         ),
966:     }
967: }
968: 
969: fn issued_label(label: &str, issued: &std::collections::HashMap<String, String>) -> BlankNode {
970:     match issued.get(label) {
971:         Some(c) => BlankNode::new_unchecked(c.clone()),
972:         // Unlabelled bnode (shouldn't happen for a fully canonicalized dataset);
973:         // keep the input label rather than panic so the failure is visible.
974:         None => BlankNode::new_unchecked(label),
975:     }
976: }
977: 
978: fn relabel_subject_canonical(
979:     subject: &NamedOrBlankNode,
980:     issued: &std::collections::HashMap<String, String>,
981: ) -> NamedOrBlankNode {
982:     match subject_bnode(subject) {
983:         Some(b) => NamedOrBlankNode::BlankNode(issued_label(b.as_str(), issued)),
984:         None => subject.clone(),
985:     }
986: }
987: 
988: fn relabel_term_canonical(term: &Term, issued: &std::collections::HashMap<String, String>) -> Term {
989:     match term {
990:         Term::BlankNode(b) => Term::BlankNode(issued_label(b.as_str(), issued)),
991:         Term::Triple(t) => Term::Triple(Box::new(Triple::new(
992:             relabel_subject_canonical(&t.subject, issued),
993:             t.predicate.clone(),
994:             relabel_term_canonical(&t.object, issued),
995:         ))),
996:         other => other.clone(),
997:     }
998: }
999: 
1000: fn relabel_graph_canonical(
1001:     graph: &GraphName,
1002:     issued: &std::collections::HashMap<String, String>,
1003: ) -> GraphName {
1004:     match graph {
1005:         GraphName::BlankNode(b) => GraphName::BlankNode(issued_label(b.as_str(), issued)),
1006:         other => other.clone(),
1007:     }
1008: }
1009: 
1010: /// One canonical N-Quads line, `… .\n`, using oxrdf-0.3's canonical `Display`
```

## existing-discriminating-tests.txt

```rust
crates/sparq-bench/src/update_fuzz.rs:1720-1779
1720:             "its canonical sibling is a different RDF term and must survive: {:?}",
1721:             after
1722:         );
1723:         assert_eq!(after.len(), 3, "exactly one term removed: {:?}", after);
1724:     }
1725: 
1726:     /// sq-hodke (2): the isomorphism-aware compare is REAL, in both directions. Two
1727:     /// datasets that differ only in blank-node labels must compare Same; one extra
1728:     /// edge on a blank node must still be caught. Without the second half, the
1729:     /// canonicalization would be a divergence-swallowing rubber stamp.
1730:     #[test]
1731:     fn blank_node_compare_is_isomorphism_not_relabelling_blindness() {
1732:         let a = vec![
1733:             "_:fb0 <http://ex/p> <http://ex/o> .".to_string(),
1734:             "_:fb0 <http://ex/q> _:fb1 <http://ex/g> .".to_string(),
1735:         ];
1736:         let relabelled = vec![
1737:             "_:zzz <http://ex/p> <http://ex/o> .".to_string(),
1738:             "_:zzz <http://ex/q> _:aaa <http://ex/g> .".to_string(),
1739:         ];
1740:         assert!(
1741:             matches!(compare("a", &a, "b", &relabelled, false), Verdict::Same),
1742:             "a pure blank-node relabelling is the SAME dataset"
1743:         );
1744:         let mut extra = relabelled.clone();
1745:         extra.push("_:zzz <http://ex/r> <http://ex/o2> .".to_string());
1746:         extra.sort();
1747:         assert!(
1748:             matches!(compare("a", &a, "b", &extra, false), Verdict::Differs(_)),
1749:             "an extra edge on a blank node is NOT an isomorphism and must fail"
1750:         );
1751:         // A different blank-node *shape* with the same edge count must also fail.
1752:         let reshaped = vec![
1753:             "_:p <http://ex/p> <http://ex/o> .".to_string(),
1754:             "_:q <http://ex/q> _:r <http://ex/g> .".to_string(),
1755:         ];
1756:         assert!(
1757:             matches!(compare("a", &a, "b", &reshaped, false), Verdict::Differs(_)),
1758:             "splitting one blank node into two is NOT an isomorphism and must fail"
1759:         );
1760:     }
1761: 
1762:     /// A duplicate quad on one side alone survives canonicalization (RDFC-1.0
1763:     /// deduplicates), so the raw-count guard must catch it. Pins the check that keeps
1764:     /// v1's duplicate sensitivity alive under the new comparator.
1765:     #[test]
1766:     fn duplicate_quad_is_caught_despite_canonicalization() {
1767:         let a = vec![
1768:             "_:x <http://ex/p> <http://ex/o> .".to_string(),
1769:             "_:x <http://ex/p> <http://ex/o> .".to_string(),
1770:         ];
1771:         let b = vec!["_:y <http://ex/p> <http://ex/o> .".to_string()];
1772:         match compare("a", &a, "b", &b, false) {
1773:             Verdict::Differs(d) => assert!(
1774:                 d.contains("COUNTS differ"),
1775:                 "expected the raw-count guard to fire, got:\n{}",
1776:                 d
1777:             ),
1778:             _ => panic!("a duplicated quad on one side must not compare equal"),
1779:         }

crates/sparq-bench/src/update_fuzz.rs:2105-2205
2105:             }
2106:         }
2107:     }
2108: 
2109:     #[test]
2110:     fn lexical_adjudication_preserves_rows_and_blank_node_structure() {
2111:         // The blank-node case reaches canon's set conversion; without it the
2112:         // ordinary vector comparison alone rejects the differing multiplicities.
2113:         for subject in ["<http://ex/s>", "_:x"] {
2114:             let p = format!("{subject} <http://ex/p> \"020\"^^{XSD_INTEGER} .");
2115:             let q = format!("{subject} <http://ex/q> \"021\"^^{XSD_INTEGER} .");
2116:             let a = vec![p.clone(), p.clone(), q.clone()];
2117:             let b = vec![
2118:                 p.replace("020", "20"),
2119:                 q.replace("021", "21"),
2120:                 q.replace("021", "21"),
2121:             ];
2122:             assert!(
2123:                 matches!(compare("a", &a, "b", &b, true), Verdict::Differs(_)),
2124:                 "equal totals cannot hide duplicate redistribution for {subject}"
2125:             );
2126:         }
2127:         let a = vec![
2128:             format!("_:a <http://ex/p> \"020\"^^{XSD_INTEGER} ."),
2129:             "_:a <http://ex/q> <http://ex/o> .".into(),
2130:         ];
2131:         let good = vec![
2132:             format!("_:x <http://ex/p> \"20\"^^{XSD_INTEGER} ."),
2133:             "_:x <http://ex/q> <http://ex/o> .".into(),
2134:         ];
2135:         assert!(matches!(
2136:             compare("a", &a, "b", &good, true),
2137:             Verdict::AdjudicatedIntegerLexical
2138:         ));
2139:         let split = vec![good[0].clone(), "_:y <http://ex/q> <http://ex/o> .".into()];
2140:         match compare("a", &a, "b", &split, true) {
2141:             Verdict::Differs(detail) => {
2142:                 assert!(
2143:                     detail.contains("blank-node counts differ (1 vs 2)"),
2144:                     "{detail}"
2145:                 );
2146:                 assert!(detail.contains("only in a:"), "{detail}");
2147:                 assert!(detail.contains("only in b:"), "{detail}");
2148:                 assert!(
2149:                     detail.contains("\"020\""),
2150:                     "original lexical missing: {detail}"
2151:                 );
2152:                 assert!(
2153:                     detail.contains("\"20\""),
2154:                     "reference lexical missing: {detail}"
2155:                 );
2156:             }
2157:             _ => panic!("different blank-node counts must fail with original dataset details"),
2158:         }
2159:         let extra = vec![good[0].clone(), "_:x <http://ex/r> <http://ex/o> .".into()];
2160:         assert!(
2161:             matches!(compare("a", &a, "b", &extra, true), Verdict::Differs(_)),
2162:             "same node/row counts do not license a changed predicate"
2163:         );
2164:         for (left, right) in [
2165:             (
2166:                 "\"20.0\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
2167:                 "\"20\"^^<http://www.w3.org/2001/XMLSchema#decimal>",
2168:             ),
2169:             ("\"x\"@en", "\"x\"@fr"),
2170:         ] {
2171:             let left = vec![format!("<http://ex/s> <http://ex/p> {left} .")];
2172:             let right = vec![format!("<http://ex/s> <http://ex/p> {right} .")];
2173:             assert!(matches!(
2174:                 compare("a", &left, "b", &right, true),
2175:                 Verdict::Differs(_)
2176:             ));
2177:         }
2178:     }
2179: 
2180:     #[test]
2181:     fn nested_noncanonical_terms_remain_exact_in_sparq() {
2182:         let expected = vec![format!(
2183:             "<http://ex/s> <http://ex/p> <<( <http://ex/a> <http://ex/q> <<( <http://ex/b> <http://ex/r> \"020\"^^{XSD_INTEGER} )>> )>> ."
2184:         )];
2185:         let op = format!("INSERT DATA {{ {} }}", expected[0]);
2186:         let rebuilt = sparq_engine::update(&Graph::new(), &op).unwrap();
2187:         let mut inplace = Graph::new();
2188:         sparq_engine::update_in_place(&mut inplace, &op).unwrap();
2189:         for graph in [&rebuilt, &inplace] {
2190:             assert_eq!(sparq_nquads(graph), expected);
2191:             assert_eq!(sparq_probe(graph, PROBES[0].0).unwrap(), expected);
2192:         }
2193:         let canonical: Vec<_> = expected.iter().map(|s| s.replace("020", "20")).collect();
2194:         assert!(matches!(
2195:             compare("sparq", &expected, "reference", &canonical, true),
2196:             Verdict::AdjudicatedIntegerLexical
2197:         ));
2198:         assert!(matches!(
2199:             compare("sparq", &expected, "other sparq", &canonical, false),
2200:             Verdict::Differs(_)
2201:         ));
2202:     }
2203: 
2204:     #[test]
2205:     fn injected_marker_fails_during_actual_lexical_adjudication() {
```
