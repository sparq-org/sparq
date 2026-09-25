"""End-to-end tests for the `sparq` Python package (load / query / update / reason).

Run with the extension module built into the active environment:

    python3 -m venv .venv-py && . .venv-py/bin/activate
    pip install maturin pytest
    (cd crates/sparq-py && maturin develop)
    pytest crates/sparq-py/tests
"""

import json

import pytest

import sparq

DATA = """
@prefix ex: <http://ex/> .
ex:alice ex:knows ex:bob ; ex:age 30 ; ex:name "Alice" .
ex:bob   ex:knows ex:carol ; ex:age 25 ; ex:name "Bob"@en .
ex:carol ex:age 35 .
"""


def g():
    return sparq.Graph.load(DATA)


# --- load -------------------------------------------------------------------


def test_load_from_string_and_len():
    graph = g()
    assert len(graph) == 7
    assert "7 triples" in repr(graph)


def test_load_from_path(tmp_path):
    p = tmp_path / "data.ttl"
    p.write_text(DATA)
    assert len(sparq.Graph.load(p)) == 7  # pathlib.Path
    assert len(sparq.Graph.load(str(p))) == 7  # str path
    nt = tmp_path / "data.nt"
    nt.write_text("<http://ex/a> <http://ex/p> <http://ex/b> .\n")
    assert len(sparq.Graph.load(nt)) == 1  # format inferred from .nt


def test_load_named_graphs_nquads():
    nq = (
        "<http://ex/a> <http://ex/p> <http://ex/b> .\n"
        "<http://ex/c> <http://ex/p> <http://ex/d> <http://ex/g1> .\n"
    )
    graph = sparq.Graph.load(nq, format="nquads")
    assert len(graph) == 1  # default graph only
    assert graph.ask("ASK { GRAPH <http://ex/g1> { ?s ?p ?o } }")


# JSON-LD is default-on in the Python wheel (sq-oy1f.20): a default-features build
# wires sparq-core/jsonld, so both the format alias and the .jsonld extension work
# with no flag. A --no-default-features wheel would drop these (fail-closed); the
# CI builds default-on, so this exercises the real default surface.
JSONLD = """
{
  "@context": {"ex": "http://ex/"},
  "@id": "ex:alice",
  "ex:knows": {"@id": "ex:bob"},
  "ex:age": 30
}
"""


def test_load_jsonld_format_alias():
    graph = sparq.Graph.load(JSONLD, format="jsonld")
    assert len(graph) == 2  # ex:knows + ex:age
    assert graph.ask("ASK { <http://ex/alice> <http://ex/knows> <http://ex/bob> }")


def test_load_jsonld_extension_inferred(tmp_path):
    p = tmp_path / "data.jsonld"
    p.write_text(JSONLD)
    graph = sparq.Graph.load(p)  # format inferred from .jsonld
    assert len(graph) == 2
    assert graph.ask(
        'ASK { <http://ex/alice> <http://ex/age> 30 }'
    )


def test_load_jsonld_named_graph():
    # @graph with an outer @id is a named graph, preserved + queryable via GRAPH.
    doc = """
    {
      "@context": {"ex": "http://ex/"},
      "@id": "ex:g1",
      "@graph": [{"@id": "ex:c", "ex:p": {"@id": "ex:d"}}]
    }
    """
    graph = sparq.Graph.load(doc, format="json-ld")  # hyphenated alias too
    assert len(graph) == 0  # default graph empty; the triple is in ex:g1
    assert graph.ask("ASK { GRAPH <http://ex/g1> { <http://ex/c> <http://ex/p> ?o } }")


def test_load_parse_error():
    with pytest.raises(ValueError):
        sparq.Graph.load("this is not turtle @@@")


# --- query ------------------------------------------------------------------


def test_query_vars_rows_and_terms():
    res = g().query(
        "PREFIX ex: <http://ex/> SELECT ?s ?a WHERE { ?s ex:age ?a } ORDER BY ?a"
    )
    assert res.vars == ["s", "a"]
    assert len(res) == 3
    rows = res.rows
    assert [str(r["a"]) for r in rows] == ["25", "30", "35"]
    s0 = rows[0]["s"]
    assert s0.kind == "uri" and s0.value == "http://ex/bob"
    a0 = rows[0]["a"]
    assert a0.kind == "literal"
    assert a0.datatype == "http://www.w3.org/2001/XMLSchema#integer"
    # sequence protocol: indexing (incl. negative) and iteration
    assert res[0] == rows[0]
    assert res[-1]["a"].value == "35"
    assert sum(1 for _ in res) == 3


def test_term_language_eq_and_repr():
    res = g().query('PREFIX ex: <http://ex/> SELECT ?n WHERE { ex:bob ex:name ?n }')
    (term,) = [r["n"] for r in res.rows]
    assert term.language == "en"
    assert repr(term) == 'Term("Bob"@en)'
    same = g().query('PREFIX ex: <http://ex/> SELECT ?n WHERE { ex:bob ex:name ?n }').rows[0]["n"]
    assert term == same  # value equality
    assert hash(term) == hash(same)
    assert term != res  # not equal to arbitrary objects


def test_optional_leaves_var_out_of_row():
    res = g().query(
        "PREFIX ex: <http://ex/> SELECT ?s ?k WHERE { ?s ex:age ?a OPTIONAL { ?s ex:knows ?k } }"
    )
    assert len(res) == 3
    unbound = [r for r in res.rows if "k" not in r]
    assert len(unbound) == 1  # carol knows nobody


def test_query_json():
    out = g().query_json("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age 30 }")
    doc = json.loads(out)
    assert doc["head"]["vars"] == ["s"]
    bindings = doc["results"]["bindings"]
    assert bindings == [{"s": {"type": "uri", "value": "http://ex/alice"}}]


def test_query_errors():
    with pytest.raises(ValueError):
        g().query("SELECT WHERE {")  # malformed
    with pytest.raises(ValueError):
        g().query("CONSTRUCT { ?s ?p ?o } WHERE { ?s ?p ?o }")  # unsupported form


# --- ask --------------------------------------------------------------------


def test_ask():
    graph = g()
    assert graph.ask("PREFIX ex: <http://ex/> ASK { ex:alice ex:knows ex:bob }")
    assert not graph.ask("PREFIX ex: <http://ex/> ASK { ex:bob ex:knows ex:alice }")
    # SELECT is accepted too: True iff any row
    assert graph.ask("SELECT * WHERE { ?s ?p ?o }")
    with pytest.raises(ValueError):
        graph.ask("DESCRIBE <http://ex/alice>")


def test_ask_and_select_count_retain_version_ebv():
    # [GPT-6] Exercise the real Python entry points, including SELECT-as-bool.
    graph = g()
    body = 'FILTER(!"z"^^<http://www.w3.org/2001/XMLSchema#boolean>)'
    for form in ("ASK", "SELECT * WHERE"):
        assert graph.ask(f"VERSION '1.1' {form} {{{body}}}")
        assert not graph.ask(f"VERSION '1.2' {form} {{{body}}}")
        for prologue in ("VERSION 'bogus'", "VERSION '1.1' VERSION '1.2'"):
            with pytest.raises(ValueError):
                graph.ask(f"{prologue} {form} {{{body}}}")


# --- construct / describe ----------------------------------------------------


def test_construct_triples_and_dedup():
    triples = g().construct(
        "PREFIX ex: <http://ex/> "
        "CONSTRUCT { ?s a ex:Person } WHERE { ?s ex:age ?a }"
    )
    assert len(triples) == 3  # one per subject (a set, deduplicated)
    s, p, o = triples[0]
    assert s.kind == "uri"
    assert p.value == "http://www.w3.org/1999/02/22-rdf-syntax-ns#type"
    assert o.value == "http://ex/Person"
    assert {str(s) for s, _, _ in triples} == {
        "http://ex/alice",
        "http://ex/bob",
        "http://ex/carol",
    }


def test_construct_drops_unbound_template_triples():
    # carol knows nobody, so her ex:knows template triple is silently dropped
    triples = g().construct(
        "PREFIX ex: <http://ex/> "
        "CONSTRUCT { ?s ex:knows ?k } WHERE { ?s ex:age ?a OPTIONAL { ?s ex:knows ?k } }"
    )
    assert len(triples) == 2


def test_construct_literal_terms():
    triples = g().construct(
        "PREFIX ex: <http://ex/> CONSTRUCT { ?s ex:label ?n } WHERE { ex:bob ex:name ?n . BIND(ex:bob AS ?s) }"
    )
    ((_, _, o),) = triples
    assert o.kind == "literal" and o.value == "Bob" and o.language == "en"


def test_construct_requires_construct_query():
    with pytest.raises(ValueError):
        g().construct("SELECT * WHERE { ?s ?p ?o }")


def test_describe_iri_and_var():
    triples = g().describe("DESCRIBE <http://ex/alice>")
    assert {(str(s), str(p)) for s, p, _ in triples} == {
        ("http://ex/alice", "http://ex/knows"),
        ("http://ex/alice", "http://ex/age"),
        ("http://ex/alice", "http://ex/name"),
    }
    via_var = g().describe(
        "PREFIX ex: <http://ex/> DESCRIBE ?s WHERE { ?s ex:name \"Alice\" }"
    )
    assert {t for t in via_var} == {t for t in triples}


def test_describe_cbd_recurses_blank_nodes():
    data = """
    @prefix ex: <http://ex/> .
    ex:alice ex:addr [ ex:city "Berlin" ] .
    """
    triples = sparq.Graph.load(data).describe("DESCRIBE <http://ex/alice>")
    # the anonymous address node's own triples are part of alice's CBD
    assert len(triples) == 2
    assert any(o.value == "Berlin" for _, _, o in triples)


def test_describe_requires_describe_query():
    with pytest.raises(ValueError):
        g().describe("PREFIX ex: <http://ex/> ASK { ex:alice ?p ?o }")


# --- update -----------------------------------------------------------------


def test_update_insert_and_delete_round_trip():
    graph = g()
    graph.update('INSERT DATA { <http://ex/dave> <http://ex/age> 40 }')
    assert len(graph) == 8
    assert graph.ask("PREFIX ex: <http://ex/> ASK { ex:dave ex:age 40 }")
    graph.update('DELETE DATA { <http://ex/dave> <http://ex/age> 40 }')
    assert len(graph) == 7
    assert not graph.ask("PREFIX ex: <http://ex/> ASK { ex:dave ex:age ?a }")


def test_update_delete_insert_where():
    graph = g()
    graph.update(
        "PREFIX ex: <http://ex/> "
        "DELETE { ?s ex:age ?a } INSERT { ?s ex:age 99 } WHERE { ?s ex:age ?a . FILTER(?a > 28) }"
    )
    res = graph.query("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age 99 }")
    assert len(res) == 2  # alice (30) and carol (35) rewritten


def test_update_error_leaves_graph_unchanged():
    graph = g()
    with pytest.raises(ValueError):
        graph.update("NOT SPARQL")
    assert len(graph) == 7


def test_update_named_graphs():
    graph = g()
    graph.update("INSERT DATA { GRAPH <http://ex/g1> { <http://ex/a> <http://ex/p> <http://ex/b> } }")
    assert len(graph) == 7  # default graph untouched
    assert graph.ask("ASK { GRAPH <http://ex/g1> { <http://ex/a> <http://ex/p> <http://ex/b> } }")
    # GRAPH-scoped DELETE/INSERT ... WHERE
    graph.update(
        "DELETE { GRAPH <http://ex/g1> { ?s <http://ex/p> ?o } } "
        "INSERT { GRAPH <http://ex/g1> { ?s <http://ex/q> ?o } } "
        "WHERE { GRAPH <http://ex/g1> { ?s <http://ex/p> ?o } }"
    )
    assert graph.ask("ASK { GRAPH <http://ex/g1> { <http://ex/a> <http://ex/q> <http://ex/b> } }")
    graph.update("DROP GRAPH <http://ex/g1>")
    assert not graph.ask("ASK { GRAPH <http://ex/g1> { ?s ?p ?o } }")


def test_update_named_graphs_survive_default_graph_ops():
    nq = (
        "<http://ex/a> <http://ex/p> <http://ex/b> .\n"
        "<http://ex/c> <http://ex/p> <http://ex/d> <http://ex/g1> .\n"
    )
    graph = sparq.Graph.load(nq, format="nquads")
    graph.update("INSERT DATA { <http://ex/e> <http://ex/p> <http://ex/f> }")
    assert len(graph) == 2
    assert graph.ask("ASK { GRAPH <http://ex/g1> { <http://ex/c> <http://ex/p> <http://ex/d> } }")


# --- reasoning --------------------------------------------------------------

RDFS_DATA = """
@prefix ex: <http://ex/> .
@prefix rdf: <http://www.w3.org/1999/02/22-rdf-syntax-ns#> .
@prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
ex:Dog rdfs:subClassOf ex:Animal .
ex:rex rdf:type ex:Dog .
"""


def test_reason_rdfs():
    graph = sparq.Graph.load(RDFS_DATA)
    q = "PREFIX ex: <http://ex/> ASK { ex:rex a ex:Animal }"
    assert not graph.ask(q)
    added = graph.reason("rdfs")
    assert added >= 1
    assert graph.ask(q)
    assert graph.reason("rdfs") == 0  # idempotent


def test_reason_owl():
    data = """
    @prefix ex: <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    ex:knows a owl:SymmetricProperty .
    ex:alice ex:knows ex:bob .
    """
    graph = sparq.Graph.load(data)
    graph.reason("owl")
    assert graph.ask("PREFIX ex: <http://ex/> ASK { ex:bob ex:knows ex:alice }")


def test_reason_bad_profile():
    with pytest.raises(ValueError):
        g().reason("clearly-not-a-profile")
    with pytest.raises(ValueError, match="load_n3"):
        g().reason("n3")


def test_load_n3_forward_chaining():
    n3 = """
    @prefix ex: <http://ex/> .
    ex:socrates a ex:Man .
    { ?x a ex:Man } => { ?x a ex:Mortal } .
    """
    graph = sparq.Graph.load_n3(n3)
    assert graph.ask("PREFIX ex: <http://ex/> ASK { ex:socrates a ex:Mortal }")


def test_reason_preserves_named_graphs():
    nq = (
        "<http://ex/Dog> <http://www.w3.org/2000/01/rdf-schema#subClassOf> <http://ex/Animal> .\n"
        "<http://ex/rex> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Dog> .\n"
        "<http://ex/c> <http://ex/p> <http://ex/d> <http://ex/g1> .\n"
    )
    graph = sparq.Graph.load(nq, format="nquads")
    assert graph.reason("rdfs") >= 1
    assert graph.ask("PREFIX ex: <http://ex/> ASK { ex:rex a ex:Animal }")
    assert graph.ask("ASK { GRAPH <http://ex/g1> { <http://ex/c> <http://ex/p> <http://ex/d> } }")


RULES = """
@prefix ex: <http://ex/> .
{ ?x a ex:Man } => { ?x a ex:Mortal } .
"""


def test_reason_n3_with_rules():
    graph = sparq.Graph.load("@prefix ex: <http://ex/> . ex:socrates a ex:Man .")
    q = "PREFIX ex: <http://ex/> ASK { ex:socrates a ex:Mortal }"
    assert not graph.ask(q)
    added = graph.reason_n3_with(RULES)
    assert added == 1
    assert graph.ask(q)
    assert graph.reason_n3_with(RULES) == 0  # idempotent


def test_reason_n3_with_builtins_and_literals():
    graph = sparq.Graph.load(DATA)
    added = graph.reason_n3_with(
        """
        @prefix ex: <http://ex/> .
        @prefix math: <http://www.w3.org/2000/10/swap/math#> .
        { ?x ex:age ?a . ?a math:notLessThan 30 } => { ?x a ex:Adult } .
        """
    )
    assert added == 2  # alice (30) and carol (35)
    res = graph.query("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s a ex:Adult } ORDER BY ?s")
    assert [str(r["s"]) for r in res] == ["http://ex/alice", "http://ex/carol"]
    # the original facts (incl. language-tagged literals) survive the round trip
    assert graph.ask('PREFIX ex: <http://ex/> ASK { ex:bob ex:name "Bob"@en }')


def test_reason_n3_with_facts_in_rules_document():
    graph = sparq.Graph.load("@prefix ex: <http://ex/> . ex:socrates a ex:Man .")
    graph.reason_n3_with("@prefix ex: <http://ex/> . ex:plato a ex:Man .")
    assert graph.ask("PREFIX ex: <http://ex/> ASK { ex:plato a ex:Man }")


def test_reason_n3_with_preserves_named_graphs():
    nq = (
        "<http://ex/socrates> <http://www.w3.org/1999/02/22-rdf-syntax-ns#type> <http://ex/Man> .\n"
        "<http://ex/c> <http://ex/p> <http://ex/d> <http://ex/g1> .\n"
    )
    graph = sparq.Graph.load(nq, format="nquads")
    assert graph.reason_n3_with(RULES) == 1
    assert graph.ask("ASK { GRAPH <http://ex/g1> { <http://ex/c> <http://ex/p> <http://ex/d> } }")


def test_reason_n3_with_error_leaves_graph_unchanged():
    graph = g()
    with pytest.raises(ValueError):
        graph.reason_n3_with("{ this is } not => valid n3 @@@")
    assert len(graph) == 7
    assert graph.ask("PREFIX ex: <http://ex/> ASK { ex:alice ex:knows ex:bob }")


def test_reason_n3_with_rules_bnode_cannot_alias_data_bnode():
    # roborev 1961: a blank-node label in the rules document must NOT alias an
    # existing data blank node. The data has a blank node labelled `_:x` with a
    # distinguishing property; the rules also mention `_:x`. If the labels
    # collided (old merge behaviour) the rule would fire against the data node
    # and add ex:Tainted to it. With graph bnodes renamed under the reserved
    # `sparqg` prefix, the rule's `_:x` is a separate node and the data node is
    # untouched.
    graph = sparq.Graph.load(
        "@prefix ex: <http://ex/> . _:x ex:realProp ex:realVal ."
    )
    added = graph.reason_n3_with(
        """
        @prefix ex: <http://ex/> .
        _:x ex:p ex:q .
        { _:x ex:p ex:q } => { _:x a ex:Tainted } .
        """
    )
    # The rule fires only on the rules-local _:x; the data node keeps only its
    # original property and gains nothing tying it to ex:Tainted.
    assert not graph.ask(
        "PREFIX ex: <http://ex/> ASK { ?b ex:realProp ex:realVal . ?b a ex:Tainted }"
    )
    # The original data triple survives the rename round-trip.
    assert graph.ask("PREFIX ex: <http://ex/> ASK { ?b ex:realProp ex:realVal }")
    assert added >= 0  # exact count depends on rules-side materialization, not asserted


# --- inconsistencies ----------------------------------------------------------


def test_inconsistencies_clean_graph():
    assert g().inconsistencies() == []


def test_inconsistencies_disjoint_classes():
    data = """
    @prefix ex: <http://ex/> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    ex:Cat owl:disjointWith ex:Dog .
    ex:felix a ex:Cat , ex:Dog .
    """
    clashes = sparq.Graph.load(data).inconsistencies()
    assert len(clashes) >= 1
    assert any("felix" in c for c in clashes)


def test_inconsistencies_after_owl_reasoning():
    # the clash only follows by entailment (subclass membership): reason first
    data = """
    @prefix ex: <http://ex/> .
    @prefix rdfs: <http://www.w3.org/2000/01/rdf-schema#> .
    @prefix owl: <http://www.w3.org/2002/07/owl#> .
    ex:Cat owl:disjointWith ex:Dog .
    ex:Tabby rdfs:subClassOf ex:Cat .
    ex:felix a ex:Tabby , ex:Dog .
    """
    graph = sparq.Graph.load(data)
    assert graph.inconsistencies() == []  # asserted triples alone do not clash
    graph.reason("owl")
    assert len(graph.inconsistencies()) >= 1


# --- save / open ------------------------------------------------------------


def test_save_open_round_trip(tmp_path):
    d = tmp_path / "db"
    graph = g()
    graph.save(d)
    reopened = sparq.Graph.open(d)
    assert len(reopened) == len(graph)
    assert reopened.query_json(
        "PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a } ORDER BY ?a"
    ) == graph.query_json("PREFIX ex: <http://ex/> SELECT ?s WHERE { ?s ex:age ?a } ORDER BY ?a")


def test_open_missing_dir(tmp_path):
    with pytest.raises(IOError):
        sparq.Graph.open(tmp_path / "nope")


# --- copy -------------------------------------------------------------------


def test_copy_is_equal_but_independent():
    graph = g()
    c = graph.copy()
    assert len(c) == len(graph) == 7
    q = "PREFIX ex: <http://ex/> SELECT ?s ?a WHERE { ?s ex:age ?a } ORDER BY ?a"
    assert c.query_json(q) == graph.query_json(q)
    # Mutating the copy does NOT touch the original ...
    c.update("INSERT DATA { <http://ex/dave> <http://ex/age> 40 }")
    assert len(c) == 8 and len(graph) == 7
    assert not graph.ask("PREFIX ex: <http://ex/> ASK { ex:dave ex:age 40 }")
    # ... and mutating the original after copying does NOT touch the copy.
    graph.update("INSERT DATA { <http://ex/erin> <http://ex/age> 50 }")
    assert len(graph) == 8 and len(c) == 8
    assert not c.ask("PREFIX ex: <http://ex/> ASK { ex:erin ex:age 50 }")


def test_copy_reason_independent():
    graph = sparq.Graph.load(RDFS_DATA)
    c = graph.copy()
    c.reason("rdfs")
    q = "PREFIX ex: <http://ex/> ASK { ex:rex a ex:Animal }"
    assert c.ask(q)          # entailment materialised on the copy
    assert not graph.ask(q)  # original untouched by the copy's reasoning


def test_copy_preserves_named_graphs():
    nq = (
        "<http://ex/a> <http://ex/p> <http://ex/b> .\n"
        "<http://ex/c> <http://ex/p> <http://ex/d> <http://ex/g1> .\n"
    )
    graph = sparq.Graph.load(nq, format="nquads")
    c = graph.copy()
    assert len(c) == 1  # default graph
    assert c.ask("ASK { GRAPH <http://ex/g1> { <http://ex/c> <http://ex/p> <http://ex/d> } }")


def test_copy_has_fresh_text_index():
    graph = g()
    graph.build_text_index()           # force the original's index to exist
    c = graph.copy()
    # The copy lazily builds its own index; text search works on it independently.
    hits = c.text_search("alic*")
    assert any(h[0].value == "Alice" for h in hits)


def test_version():
    assert isinstance(sparq.__version__, str) and sparq.__version__
