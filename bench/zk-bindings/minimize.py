"""[GPT-6] Bounded data delta reduction with a fixed semantic failure predicate."""
from copy import deepcopy
from corpus import oracle


def minimize(case, fails, max_attempts=128):
    """Keep original query/template; regenerate only this independent synthetic oracle."""
    if case["oracle"]["kind"] != "finite_relational_definition":
        raise ValueError("never rewrite an imported normative golden while minimizing")
    if not 1 <= max_attempts <= 1024:
        raise ValueError("minimization work budget")
    original = deepcopy(case)
    if not fails(original):
        raise ValueError("original case does not reproduce the semantic failure")
    current, attempts = original, 1
    granularity = 2
    trace = []
    while current["triples"] and attempts < max_attempts:
        size = len(current["triples"])
        width = max(1, (size + granularity - 1) // granularity)
        reduced = False
        for start in range(0, size, width):
            if attempts == max_attempts:
                break
            candidate = deepcopy(current)
            candidate["triples"] = current["triples"][:start] + current["triples"][start + width:]
            query, candidate["expected"] = oracle(candidate["triples"], candidate["template"])
            if query != original["query"]:
                raise ValueError("minimization changed the original query bytes")
            text = "".join(" ".join(t) + " .\n" for t in candidate["triples"])
            candidate["dataset"] = {"ntriples":text,"nquads":text,"named_graphs":[]}
            attempts += 1
            retained = fails(candidate)
            trace.append({"attempt":attempts,"triples":len(candidate["triples"]),"same_failure":retained})
            if retained:
                current, granularity, reduced = candidate, max(2, granularity - 1), True
                break
        if not reduced:
            if granularity >= size:
                break
            granularity = min(size, granularity * 2)
    return {"original":original,"minimized":current,"attempts":attempts,"trace":trace,
            "budget_exhausted":attempts == max_attempts,
            "claim":"Data reduction under the supplied exact failure predicate; no globally minimal query claim."}
