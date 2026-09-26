"""[GPT-6] Independent finite relational oracle and retained source-case import."""
import hashlib
import itertools
import json
from pathlib import Path

BACKENDS = ("noir_unsigned", "noir_signed", "native_rdf", "exact_v1", "exact_v2", "exact_v3")
NODES = ("<urn:a>", "<urn:b>")
CANDIDATES = (*NODES, "<urn:missing>")
PREDICATE = "<urn:p>"
TEMPLATES = ("scan", "join", "projection_bag", "count", "ask_absence", "negation", "top_k")
NOIR_ATTACKS = ("selected_padding", "leaf_out_of_range", "credential_out_of_range",
                "empty_signed_graph", "length_mismatch", "activate_padding_row",
                "deactivate_real_row", "inactive_public_nonzero", "valid_looking_padding",
                "duplicate_support_preimage")
INTEGER = "http://www.w3.org/2001/XMLSchema#integer"


def encoded(value):
    return (json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False,
                       allow_nan=False) + "\n").encode()


def digest(value):
    return hashlib.sha256(encoded(value)).hexdigest()


def load(path):
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result:
                raise ValueError(f"duplicate JSON key: {key}")
            result[key] = value
        return result
    data = Path(path).read_bytes()
    if len(data) > 16 * 1024 * 1024:
        raise ValueError("corpus byte capacity exceeded")
    return json.loads(data, object_pairs_hook=unique,
                      parse_constant=lambda x: (_ for _ in ()).throw(ValueError(x)))


def select(variables, rows, ordered=False):
    return {"Select": {"variables": variables, "order": "Sequence" if ordered else "Bag",
                       "rows": rows}}


def oracle(triples, template):
    """Closed finite algebra; never calls Sparq, its planner, or its evaluator."""
    edges = sorted({(s, o) for s, p, o in triples if p == PREDICATE})
    if template == "scan":
        return "SELECT DISTINCT ?s ?o WHERE { ?s <urn:p> ?o }", select(["s", "o"], [list(e) for e in edges])
    if template == "join":
        rows = sorted({(s, middle, o) for s, middle in edges for other, o in edges if middle == other})
        return "SELECT DISTINCT ?s ?m ?o WHERE { ?s <urn:p> ?m . ?m <urn:p> ?o }", select(["s", "m", "o"], [list(r) for r in rows])
    if template == "projection_bag":
        return "SELECT ?s WHERE { ?s <urn:p> ?o }", select(["s"], [[s] for s, _ in edges])
    if template == "count":
        return "SELECT (COUNT(*) AS ?n) WHERE { ?s <urn:p> ?o }", select(["n"], [[f'"{len(edges)}"^^<{INTEGER}>']])
    if template == "ask_absence":
        return "ASK { <urn:missing> <urn:p> ?o }", {"Ask": False}
    if template == "negation":
        rows = [[s, o] for s, o in edges if not any(other == o for other, _ in edges)]
        return "SELECT ?s ?o WHERE { ?s <urn:p> ?o FILTER NOT EXISTS { ?o <urn:p> ?z } }", select(["s", "o"], rows)
    if template == "top_k":
        return "SELECT ?s ?o WHERE { ?s <urn:p> ?o } ORDER BY ?s ?o LIMIT 1", select(["s", "o"], [list(e) for e in edges[:1]], True)
    raise ValueError(f"unknown template: {template}")


def tiny_case(mask, template):
    universe = [(s, PREDICATE, o) for s in NODES for o in NODES]
    if type(mask) is not int or not 0 <= mask < 1 << len(universe):
        raise ValueError("graph mask outside declared universe")
    triples = [list(t) for i, t in enumerate(universe) if mask & (1 << i)]
    query, expected = oracle(triples, template)
    return {"id": f"tiny-v1/{mask:02x}/{template}", "query": query, "triples": triples,
            "dataset": {"ntriples": "".join(" ".join(t) + " .\n" for t in triples),
                        "nquads": "".join(" ".join(t) + " .\n" for t in triples), "named_graphs": []},
            "expected": expected, "template": template, "candidate_terms": list(CANDIDATES),
            "oracle": {"kind": "finite_relational_definition", "domain": "tiny-v1",
                       "graph_mask": mask, "source": "corpus.py:oracle"}}


def exhaustive():
    return [tiny_case(mask, template) for mask in range(16) for template in TEMPLATES]


def finite_proof_universe():
    """All valid scan/join bindings for the fixed two-edge cycle, plus attacks."""
    cases = [tiny_case(6, template) for template in ("scan", "join")]
    for attack in NOIR_ATTACKS:
        case = tiny_case(6, "join")
        case.update(id=f"noir-witness-v1/{attack}", template="noir_witness_attack",
                    attack=attack)
        cases.append(case)
    return cases


def splitmix(seed):
    """The same deterministic PRNG family as sparq-bench and prior ZK fuzzers."""
    state = seed & ((1 << 64) - 1)
    while True:
        state = (state + 0x9E3779B97F4A7C15) & ((1 << 64) - 1)
        z = state
        z = ((z ^ (z >> 30)) * 0xBF58476D1CE4E5B9) & ((1 << 64) - 1)
        z = ((z ^ (z >> 27)) * 0x94D049BB133111EB) & ((1 << 64) - 1)
        yield z ^ (z >> 31)


def sampled(seed, count):
    if not 1 <= count <= 100_000:
        raise ValueError("sample count outside bounded profile")
    rng = splitmix(seed)
    result = []
    for ordinal in range(count):
        case = tiny_case(next(rng) % 16, TEMPLATES[next(rng) % len(TEMPLATES)])
        case["oracle"]["sample"] = {"seed": seed, "ordinal": ordinal}
        case["id"] = f"sample-v1/{seed}/{ordinal}/{case['id']}"
        result.append(case)
    return result


def capacity_expectation(original):
    """Exact cause from a reviewed original, never inferred from an actual error."""
    registry = load(Path(__file__).with_name("capacity-expectations.json"))
    matches = [entry for source in registry["sources"] for entry in source["cases"]
               if entry["id"] == original["id"] and entry["original_fixture_sha256"] == digest(original)]
    if len(matches) > 1:
        raise ValueError("duplicate capacity expectation")
    return matches[0]["expected_rejection"] if matches else None


ROOT = Path(__file__).resolve().parents[2]
PROJECTIONS = Path(__file__).with_name("projection-expectations.json")
PROJECTION_KEYS = {"source", "source_sha256", "id", "original_fixture", "variables"}


def valid_variables(variables):
    return (type(variables) is list
            and all(type(v) is str and v and v == v.strip() and v[0] not in "?$" for v in variables)
            and len(set(variables)) == len(variables))


def check_select(result, case_id):
    """[OPUS-5.5] Structural SELECT golden: unique names, list rows of projection width."""
    if type(result) is not dict or "Select" not in result:
        return
    body = result["Select"]
    if type(body) is not dict or not valid_variables(body.get("variables")):
        raise ValueError(f"fixture {case_id} has invalid SELECT projection variables")
    rows = body.get("rows")
    if type(rows) is not list or any(type(row) is not list for row in rows):
        raise ValueError(f"fixture {case_id} has malformed SELECT rows")
    if any(len(row) != len(body["variables"]) for row in rows):
        raise ValueError(f"fixture {case_id} row width differs from its projection width")


def projection_overrides(path, document):
    """[OPUS-5.5] Reviewed per-fixture projections; exact source path, hash and object."""
    registry = load(PROJECTIONS)
    if (set(registry) != {"schema", "authored_by", "scope", "overrides"}
            or registry["schema"] != "sparq.projection-expectations.v1"
            or type(registry["overrides"]) is not list):
        raise ValueError("unknown projection expectation schema")
    registry_sha256 = hashlib.sha256(PROJECTIONS.read_bytes()).hexdigest()
    source_sha256 = hashlib.sha256(path.read_bytes()).hexdigest()
    originals = {c.get("id"):c for c in document["cases"]}
    seen, used = set(), {}
    for entry in registry["overrides"]:
        if (type(entry) is not dict or set(entry) != PROJECTION_KEYS
                or type(entry["source"]) is not str or type(entry["id"]) is not str):
            raise ValueError("unknown projection override record")
        relative = Path(entry["source"])
        if relative.is_absolute() or ".." in relative.parts or not entry["variables"] or not valid_variables(entry["variables"]):
            raise ValueError(f"invalid projection override: {entry['id']}")
        if (entry["source"], entry["id"]) in seen:
            raise ValueError(f"duplicate projection override: {entry['id']}")
        seen.add((entry["source"], entry["id"]))
        if (ROOT / relative).resolve() != path:
            continue  # Same ID in an unrelated file never adopts this override.
        original = originals.get(entry["id"])
        if (entry["source_sha256"] != source_sha256 or original != entry["original_fixture"]
                or "expected_rows" not in original or "expected" in original or "expected_result" in original):
            raise ValueError(f"stale or unknown projection override: {entry['id']}")
        used[entry["id"]] = {"kind":"reviewed_projection_override", "registry":PROJECTIONS.name,
                             "registry_sha256":registry_sha256, "source":entry["source"],
                             "source_sha256":source_sha256, "original_fixture_sha256":digest(original),
                             "variables":entry["variables"]}
    return used


def import_regressions(path, variables=None):
    """Retain original case IDs, query bytes, golden objects and source digest."""
    path = Path(path).resolve()
    document = load(path)
    overrides = projection_overrides(path, document)
    dataset_path = path.parent / document["default_dataset"] if "default_dataset" in document else None
    dataset = dataset_path.read_text() if dataset_path else ""
    versioned = {entry["id"]: entry for entry in
                 load(Path(__file__).with_name("version-expectations.json"))["entries"]}
    output = []
    for original in document["cases"]:
        source_data = original.get("dataset_ntriples", original.get("dataset", dataset))
        version_entry = versioned.get(original["id"])
        if version_entry and (version_entry["original_fixture_sha256"] != digest(original)
                              or version_entry["dataset_sha256"] != hashlib.sha256(source_data.encode()).hexdigest()):
            raise ValueError(f"version-specific golden input changed: {original['id']}")
        expected = original.get("expected", {})
        result = expected.get("result", original.get("expected_result"))
        capacity = (original.get("expected_capacity_error", False)
                    or original.get("expectation_kind") == "implementation_capacity"
                    or document.get("expectation_kind") == "implementation_capacity")
        rejection = expected.get("kind") == "rejection" or original.get("admitted") is False or original.get("expected_admission_error", False) or capacity
        projection = None
        if result is None and "expected_rows" in original:
            # [OPUS-5.5] Reviewed exact-fixture override first, else the caller's default.
            projection = overrides.get(original["id"]) or (
                {"kind":"caller_variables", "variables":list(variables)} if variables else None)
            if projection is None:
                raise ValueError("row-only goldens require their existing runner's explicit projection variables")
            result = select(projection["variables"], original["expected_rows"])
        if result is None and not rejection:
            raise ValueError(f"fixture {original['id']} has no independent expected result or declared rejection")
        if not rejection:
            check_select(result, original["id"])
        required, unclassified = None, None
        if rejection:
            if capacity:
                required = capacity_expectation(original)
                if required is None:
                    unclassified = "Original capacity fixture has no exact cause expectation."
            elif expected.get("kind") == "rejection":
                diagnostic = expected.get("error")
                known = load(Path(__file__).with_name("rejections.json"))["diagnostics"].get(diagnostic)
                if known and expected.get("stage") in ("admission", "dataset", "evaluation"):
                    required = {"category":known["category"], "phase":expected["stage"],
                                "diagnostic":diagnostic}
                else:
                    unclassified = "Original rejection class or stage has no explicit mapping."
            elif original.get("admitted") is False or original.get("expected_admission_error"):
                required = {"category":"profile", "phase":"admission"}
            else:
                unclassified = "Original rejection requires an explicit category."
        output.append({"id": original["id"], "query": original["query"], "triples": [],
                       "dataset": {"ntriples": source_data, "nquads": source_data, "named_graphs": []},
                       "expected": None if rejection else result, "rejection": rejection,
                       **({"backend_expectations":version_entry["backend_expectations"]} if version_entry else {}),
                       **({"expected_rejection":required} if required else {}),
                       **({"classification":{"status":"requires_rejection_classification", "reason":unclassified}} if unclassified else {}),
                       "policy_overrides": {"max_rows":original["max_rows"]} if "max_rows" in original else {},
                       "template": "existing_regression", "candidate_terms": [],
                       "features": original.get("features", []), "original_fixture": original,
                       "oracle": {"kind": "retained_golden", "source": str(path),
                                  "source_sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
                                  "dataset_sha256": hashlib.sha256(source_data.encode()).hexdigest(),
                                  "spec": original.get("spec"),
                                  **({"projection":projection} if projection and not rejection else {})}})
    if not output or len({c["id"] for c in output}) != len(output):
        raise ValueError("empty or duplicate-ID regression corpus")
    return output


def jobs_for(case, backend, tier):
    if backend not in BACKENDS or tier not in ("native", "constraint", "real"):
        raise ValueError("unknown backend or tier")
    if case.get("classification"):
        return [], {"case_id":case["id"], "backend":backend, **case["classification"]}
    case_hash = digest(case)
    # [GPT-6] Profile promotion changes only the version's expected outcome.
    # Original query, dataset, fixture and V1 rejection stay bound in case_hash.
    versioned = case.get("backend_expectations", {}).get(backend)
    rejected = case.get("rejection", False) if versioned is None else False
    expected_result = case["expected"] if versioned is None else versioned["expected"]
    base = {"schema": "sparq.proof-binding-job.v1", "case_id": case["id"],
            "case_sha256": case_hash, "backend": backend, "tier": tier,
            "query": case["query"], "triples": case["triples"], "dataset": case["dataset"],
            "policy_overrides":case.get("policy_overrides", {}),
            "variables": [], "rows": [], "expected_accept": not rejected}
    candidates = []
    if case["template"] == "noir_witness_attack":
        if backend not in ("noir_unsigned", "noir_signed") or tier == "native":
            return [], {"case_id":case["id"], "backend":backend, "status":"requires_adapter",
                        "reason":"This concrete private-witness mutation is implemented only by the Noir constraint adapter."}
        expected = case["expected"]["Select"]
        candidates.append(base | {"operation":"attack", "variables":expected["variables"],
                                  "rows":expected["rows"][:1], "expected_accept":False,
                                  "attack":{"kind":case["attack"]}})
    elif backend.startswith("exact_"):
        for authority in ("verifier_agreed", "holder_declared"):
            candidates.append(base | {"operation": "admission" if rejected else "result",
                                      "expected_result": expected_result, "authority": authority,
                                      **({"expectation_oracle":versioned["oracle"]} if versioned else {}),
                                      **({"expected_rejection":case["expected_rejection"]} if rejected and case.get("expected_rejection") else {})})
    elif case["template"] in ("scan", "join"):
        expected = case["expected"]["Select"]
        rows = {tuple(row) for row in expected["rows"]}
        # Every candidate in this explicit domain, including every valid binding.
        for row in itertools.product(case["candidate_terms"], repeat=len(expected["variables"])):
            candidates.append(base | {"operation": "binding", "variables": expected["variables"],
                                      "rows": [list(row)], "expected_accept": row in rows,
                                      "attack": {"anchor_rows": expected["rows"][:1],
                                                 "kind": "fabricated_binding"}})
    else:
        return [], {"case_id": case["id"], "backend": backend, "status": "requires_profile_classification"
                    if case["template"].startswith("existing_") else "excluded",
                    "reason": "Existing query needs actual backend admission; no syntax rewriting or guessed support."
                    if case["template"].startswith("existing_") else
                    "This fixture requires bag/completeness/negation/aggregation/order semantics outside selected DISTINCT BGP support."}
    for ordinal, job in enumerate(candidates):
        job["nonce_id"] = ordinal + 1
        job["id"] = digest(job)
    return candidates, None


def plan(cases, backends, tier, shard=0, shards=1, coverage="retained_corpus"):
    if not cases or not backends or len(set(backends)) != len(backends) or not 0 <= shard < shards <= 1024:
        raise ValueError("empty, duplicate or invalid profile")
    if len({case["id"] for case in cases}) != len(cases):
        raise ValueError("duplicate case IDs")
    jobs, exclusions, totals = [], [], {}
    for backend in backends:
        all_jobs = []
        for case in cases:
            generated, exclusion = jobs_for(case, backend, tier)
            all_jobs.extend(generated)
            if exclusion:
                exclusions.append(exclusion)
        selected = [j for i, j in enumerate(all_jobs) if i % shards == shard]
        totals[backend] = {"configured_jobs": len(all_jobs), "shard_jobs": len(selected),
                           "input_cases":len(cases), "classified_cases":sum(e["backend"] == backend for e in exclusions),
                           "operation_counts":{operation:sum(j["operation"] == operation for j in all_jobs)
                                               for operation in ("binding", "result", "admission", "attack")},
                           "positive_bindings_or_results": sum(j["expected_accept"] for j in all_jobs),
                           "negative_bindings_or_admissions": sum(not j["expected_accept"] for j in all_jobs),
                           "executed_jobs": 0, "genuine_proofs": 0}
        jobs.extend(selected)
    if not jobs:
        raise ValueError("empty replay shard")
    return {"schema": "sparq.proof-bindings.plan.v1", "coverage": coverage, "tier": tier,
            "backends":list(backends),
            "domain": {"nodes": list(NODES), "candidate_terms": list(CANDIDATES),
                       "graph_count": 16, "query_templates": list(TEMPLATES)} if coverage == "exhaustive_tiny_v1" else None,
            "case_count": len(cases), "corpus_sha256": digest(cases), "shard": shard, "shards": shards,
            "cases":cases,
            "totals": totals, "classifications": exclusions, "jobs": jobs,
            "claim": "Configured coverage only. No execution or proof is established by this plan."}
