#!/usr/bin/env python3
# [OPUS-4.8] Validate orchestration/routing.toml against its model catalog (review D2).
"""routing-validate.py — schema-check the routing table.

Fails if: a model in any chain (defaults or a route) is not in [models]; a chain is empty; a route
lacks an agent; a route has neither `role` nor `match_labels`; or a model catalog entry is missing a
required field. Run in CI to prevent a misspelled model/role or an empty chain from shipping.
"""
import importlib.util
import os
import sys

try:
    import tomllib
except ModuleNotFoundError:  # pragma: no cover
    import tomli as tomllib

CATALOG_REQUIRED = ("provider", "harness", "provider_model", "credential_format")
# [FABLE-5] STANDING RULE (maintainer 2026-07-17): CI/infrastructure work is authored by a
# FRONTIER-tier model only (since 2026-07-26: sol / opus5). Validation enforces the floor
# structurally: a role:ci chain containing a sub-frontier tier fails CI, so exhaustion can only
# ever DEFER, never degrade.
SUB_FRONTIER = ("sonnet", "haiku")


def _deprecation_register():
    """[OPUS-5] Load the deprecation register from route-resolve.py — ONE source of truth.

    The register lives next to the resolver because the resolver is the fail-closed enforcement
    point (it refuses to RESOLVE a table naming a retired model). This validator is the CI-time
    half of the same rule. Importing rather than re-declaring is deliberate: two hand-maintained
    copies of a deprecation list is how a model comes back in one of them.
    """
    path = os.path.join(os.path.dirname(os.path.abspath(__file__)), "route-resolve.py")
    spec = importlib.util.spec_from_file_location("route_resolve", path)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod.DEPRECATED_ALIASES, mod.DEPRECATED_PROVIDER_MODELS


DEPRECATED_ALIASES, DEPRECATED_PROVIDER_MODELS = _deprecation_register()


def validate(doc):
    errs = []
    models = doc.get("models", {})
    if not models:
        errs.append("no [models] catalog")
    for name, spec in models.items():
        for f in CATALOG_REQUIRED:
            if f not in spec:
                errs.append(f"model {name}: missing '{f}'")

    # [OPUS-5] DEPRECATION GUARD (maintainer 2026-07-26: fable + opus-4.8 retired in favour of
    # opus5). Catalog half — the alias AND the concrete provider id, so a retired model cannot be
    # smuggled back under a fresh alias name.
    for name, spec in models.items():
        if name in DEPRECATED_ALIASES:
            errs.append(f"model {name}: DEPRECATED alias (retired 2026-07-26 in favour of opus5)")
        if spec.get("provider_model") in DEPRECATED_PROVIDER_MODELS:
            errs.append(f"model {name}: DEPRECATED provider_model "
                        f"'{spec.get('provider_model')}' (retired 2026-07-26)")

    def check_chain(where, chain):
        if not chain:
            errs.append(f"{where}: empty model_chain")
        for m in chain:
            if m in DEPRECATED_ALIASES:
                errs.append(f"{where}: DEPRECATED model '{m}' in model_chain — use 'opus5'")
            elif m not in models:
                errs.append(f"{where}: model '{m}' not in [models] catalog")

    defaults = doc.get("defaults", {})
    check_chain("defaults", defaults.get("model_chain", []))
    if not defaults.get("agent"):
        errs.append("defaults: missing agent")
    for i, r in enumerate(doc.get("route", [])):
        where = f"route[{i}]"
        if "role" not in r and "match_labels" not in r:
            errs.append(f"{where}: needs `role` or `match_labels`")
        if not r.get("agent"):
            errs.append(f"{where}: missing agent")
        check_chain(where, r.get("model_chain", []))
        # [FABLE-5] frontier-tier floor for CI/infrastructure authorship (standing rule 2026-07-17)
        if r.get("role") == "ci":
            for m in r.get("model_chain", []):
                if m in SUB_FRONTIER:
                    errs.append(f"{where}: role:ci chain contains sub-frontier model '{m}' "
                                "(frontier-tier infra-authorship rule: sol/opus5 only)")
    return errs


def _self_test():
    good = {
        "models": {"opus5": {"provider": "a", "harness": "claude",
                             "provider_model": "claude-opus-5", "credential_format": "y"}},
        "defaults": {"model_chain": ["opus5"], "agent": "sparq-rust-impl"},
        "route": [{"role": "impl", "model_chain": ["opus5"], "agent": "sparq-rust-impl"},
                  # a frontier-only ci chain is valid (frontier-tier infra-authorship rule)
                  {"role": "ci", "model_chain": ["opus5"], "agent": "sparq-ci-infra"}],
    }
    bad = {
        "models": {"opus5": {"provider": "a"},  # missing fields
                   "sonnet": {"provider": "a", "harness": "claude", "provider_model": "x",
                              "credential_format": "y"}},
        "defaults": {"model_chain": ["ghost"], "agent": ""},  # unknown model + no agent
        "route": [{"model_chain": [], "agent": "x"},  # empty chain + no role/labels
                  # sub-frontier model in a role:ci chain -> frontier-floor violation
                  {"role": "ci", "model_chain": ["sonnet"], "agent": "sparq-ci-infra"}],
    }
    # [OPUS-5] the deprecation fixtures: each isolates ONE half of the guard, so deleting either
    # half turns a NAMED assertion below red rather than merely shrinking an error count.
    dep_alias_catalog = {
        "models": {"opus5": {"provider": "a", "harness": "claude",
                             "provider_model": "claude-opus-5", "credential_format": "y"},
                   "fable": {"provider": "a", "harness": "claude",
                             "provider_model": "claude-fable-5", "credential_format": "y"}},
        "defaults": {"model_chain": ["opus5"], "agent": "sparq-rust-impl"},
        "route": [{"role": "impl", "model_chain": ["opus5"], "agent": "sparq-rust-impl"}],
    }
    dep_alias_chain = {
        "models": {"opus5": {"provider": "a", "harness": "claude",
                             "provider_model": "claude-opus-5", "credential_format": "y"}},
        "defaults": {"model_chain": ["opus5"], "agent": "sparq-rust-impl"},
        "route": [{"role": "impl", "model_chain": ["opus5", "opus"], "agent": "sparq-rust-impl"}],
    }
    dep_smuggled_id = {
        "models": {"legacy": {"provider": "a", "harness": "claude",
                              "provider_model": "claude-opus-4-8", "credential_format": "y"}},
        "defaults": {"model_chain": ["legacy"], "agent": "sparq-rust-impl"},
        "route": [{"role": "impl", "model_chain": ["legacy"], "agent": "sparq-rust-impl"}],
    }

    bad_errs = validate(bad)
    checks = []

    def chk(name, cond):
        checks.append((name, bool(cond)))
        print(f"  {'ok  ' if cond else 'FAIL'} {name}")

    chk("a clean opus5-only table validates", not validate(good))
    chk("the malformed table reports >=5 errors", len(bad_errs) >= 5)
    chk("the role:ci frontier floor still fires", any("sub-frontier" in e for e in bad_errs))
    chk("a re-added [models.fable] catalog entry is REJECTED",
        any("DEPRECATED alias" in e for e in validate(dep_alias_catalog)))
    chk("a `opus` rung reintroduced into a chain is REJECTED",
        any("DEPRECATED model 'opus'" in e for e in validate(dep_alias_chain)))
    chk("claude-opus-4-8 smuggled back under a fresh alias is REJECTED",
        any("DEPRECATED provider_model 'claude-opus-4-8'" in e
            for e in validate(dep_smuggled_id)))
    chk("the register is loaded from route-resolve.py, not re-declared here",
        DEPRECATED_ALIASES == {"fable", "opus"}
        and DEPRECATED_PROVIDER_MODELS == {"claude-fable-5", "claude-opus-4-8"})

    ok = all(c for _, c in checks)
    print("good doc errors:", validate(good))
    print("bad doc errors :", validate(bad))
    print("routing-validate self-test", "PASSED" if ok else "FAILED")
    return 0 if ok else 1


def main():
    if "--self-test" in sys.argv:
        return _self_test()
    path = sys.argv[1] if len(sys.argv) > 1 else "orchestration/routing.toml"
    errs = validate(tomllib.load(open(path, "rb")))
    if errs:
        print(f"routing.toml INVALID ({len(errs)}):")
        for e in errs:
            print("  -", e)
        return 1
    print("routing.toml OK")
    return 0


if __name__ == "__main__":
    sys.exit(main())
