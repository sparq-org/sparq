#!/usr/bin/env python3
# [SONNET-4.6] sq-gg0qq.7 — hermetic consistency check for the Solid-CTH harness config.
#
# The harness itself cannot run without Docker + a Keycloak realm + an ath-patched image, so the
# per-change protection this lane can actually offer is: prove the COMMITTED pieces still agree with
# each other and with the crate. Every check here corresponds to a way this harness has a realistic
# chance of rotting silently while `cargo test` stays green:
#
#   C1  run.sh's default SERVER_BIN names a binary this crate actually produces.
#   C2  run.sh's `--target` subject exists as a node in config/test-subjects.ttl.
#   C3  run.sh's default ENV_FILE exists.
#   C4  application.yaml's `subjects:` mount path resolves to the committed test-subjects.ttl.
#   C5  baseline.json's per-suite counts sum to expected_total, and the bounds are self-consistent.
#   C6  baseline.json's skip_tags are exactly the `solid-test:skip` tags the TestSubject declares —
#       the suite SIZE is a function of those tags, so a tag added on one side and not the other
#       silently invalidates the pinned floor.
#   C7  run.sh starts the socat forwarder ONLY behind the `--network host` reachability probe. The
#       two networking branches need opposite behaviour (VM-backed engine: the sidecar is the
#       load-bearing hop; native Linux: the server already holds :3000, so the sidecar cannot bind
#       and dies silently), and `docker run -d` exits 0 either way — so the gate cannot be observed
#       failing, only asserted here.
#
# Exit 0 = clean. Any offence is printed with its C-number and exits 1.
# Self-test: `python3 check-conformance-config.py --self-test`.

from __future__ import annotations

import argparse
import json
import re
import sys
from pathlib import Path

HERE = Path(__file__).resolve().parent
CRATE = HERE.parent


def _shell_default(text: str, var: str) -> str | None:
    """Extract `VAR="${OVERRIDE:-<default>}"` or `VAR="<literal>"` from a shell script."""
    m = re.search(rf'^{re.escape(var)}="\$\{{[A-Z_]+:-(.*?)\}}"\s*$', text, re.M)
    if m:
        return m.group(1)
    m = re.search(rf'^{re.escape(var)}="(.*?)"\s*$', text, re.M)
    return m.group(1) if m else None


def check(here: Path, crate: Path) -> list[str]:
    """Consistency offences across the committed harness config. Returns [] when clean."""
    offences: list[str] = []
    run_sh = (here / "run.sh").read_text(encoding="utf-8")
    subjects_ttl = (here / "config" / "test-subjects.ttl").read_text(encoding="utf-8")
    app_yaml = (here / "config" / "application.yaml").read_text(encoding="utf-8")
    baseline = json.loads((here / "baseline.json").read_text(encoding="utf-8"))
    cargo_toml = (crate / "Cargo.toml").read_text(encoding="utf-8")

    # --- C1: the default SERVER_BIN names a binary this crate produces. -------------------------
    server_bin = _shell_default(run_sh, "SERVER_BIN")
    if server_bin is None:
        offences.append("C1: run.sh has no parseable SERVER_BIN default")
    else:
        bin_name = server_bin.rsplit("/", 1)[-1]
        # Explicit [[bin]] stanzas, else the implicit bin named after the package.
        bins = set(re.findall(r'^\s*name\s*=\s*"([^"]+)"', cargo_toml, re.M)[:1])
        bins |= set(re.findall(r'\[\[bin\]\][^\[]*?name\s*=\s*"([^"]+)"', cargo_toml, re.S))
        if bin_name not in bins:
            offences.append(
                f"C1: run.sh SERVER_BIN default ends in {bin_name!r}, which is not a binary this "
                f"crate produces (known: {sorted(bins)}). Did the crate or its bin get renamed?"
            )

    # --- C2: the `--target` subject exists in test-subjects.ttl. --------------------------------
    target = _shell_default(run_sh, "TARGET_SUBJECT")
    if target is None:
        offences.append("C2: run.sh has no parseable TARGET_SUBJECT")
    elif not re.search(rf"^<{re.escape(target)}>\s*$", subjects_ttl, re.M):
        offences.append(
            f"C2: run.sh passes --target {target!r} but config/test-subjects.ttl declares no "
            f"<{target}> node — the harness would resolve no TestSubject and run nothing."
        )

    # --- C3: the default ENV_FILE exists. -------------------------------------------------------
    env_file = _shell_default(run_sh, "ENV_FILE")
    if env_file is None:
        offences.append("C3: run.sh has no parseable ENV_FILE default")
    else:
        rel = env_file.replace("$HERE/", "")
        if not (here / rel).is_file():
            offences.append(f"C3: run.sh ENV_FILE default {env_file!r} does not exist ({rel})")

    # --- C4: application.yaml's subjects mount resolves to the committed file. ------------------
    m = re.search(r"^subjects:\s*(\S+)\s*$", app_yaml, re.M)
    if not m:
        offences.append("C4: application.yaml has no `subjects:` key")
    else:
        # run.sh bind-mounts `config/` at /app/config, so /app/config/X must exist as config/X.
        declared = m.group(1)
        if not declared.startswith("/app/config/"):
            offences.append(
                f"C4: application.yaml subjects={declared!r} is not under /app/config/, which is "
                "where run.sh bind-mounts the config dir"
            )
        elif not (here / "config" / declared[len("/app/config/"):]).is_file():
            offences.append(f"C4: application.yaml subjects={declared!r} has no committed file behind it")

    # --- C5: baseline arithmetic is self-consistent. --------------------------------------------
    suites_sum = sum(baseline["suites"].values())
    if suites_sum != baseline["expected_total"]:
        offences.append(
            f"C5: baseline.json suites sum to {suites_sum} but expected_total={baseline['expected_total']}"
        )
    if baseline["min_passed"] > baseline["expected_total"]:
        offences.append(
            f"C5: baseline.json min_passed={baseline['min_passed']} exceeds "
            f"expected_total={baseline['expected_total']} — an unreachable floor"
        )
    if baseline["min_passed"] + baseline["max_failed"] > baseline["expected_total"]:
        offences.append(
            f"C5: baseline.json min_passed + max_failed ({baseline['min_passed']}+"
            f"{baseline['max_failed']}) exceeds expected_total={baseline['expected_total']}"
        )

    # --- C6: skip tags agree between the floor and the TestSubject. -----------------------------
    declared_skips = set(re.findall(r'solid-test:skip\s+"([^"]+)"', subjects_ttl))
    pinned_skips = set(baseline["skip_tags"])
    if declared_skips != pinned_skips:
        offences.append(
            f"C6: skip tags disagree — test-subjects.ttl declares {sorted(declared_skips)} but "
            f"baseline.json pins {sorted(pinned_skips)}. The suite SIZE depends on these, so the "
            f"pinned expected_total/min_passed are no longer trustworthy."
        )

    # --- C7: the socat forwarder is started only behind the reachability probe. -----------------
    probe = "harness_netns_reaches_server"
    if not re.search(rf"^{probe}\(\)\s*\{{", run_sh, re.M):
        offences.append(
            f"C7: run.sh no longer defines the {probe}() probe. Without it the forwarder branch is "
            f"chosen by assumption, and the assumption is wrong on one of the two engines."
        )
    lines = run_sh.splitlines()
    starts = [i for i, ln in enumerate(lines) if re.search(r'docker run -d .*--name "\$FWD_NAME"', ln)]
    if not starts:
        offences.append('C7: run.sh has no `docker run -d --name "$FWD_NAME"` forwarder start')
    for i in starts:
        guard = next((lines[j] for j in range(i - 1, -1, -1) if re.match(r"\s*(if|elif)\s", lines[j])), None)
        if guard is None or probe not in guard:
            offences.append(
                f"C7: the forwarder start at run.sh:{i + 1} is not gated on {probe} (nearest "
                f"conditional: {guard.strip() if guard else 'none — top level'}). On a native-Linux "
                f"engine the server already holds 0.0.0.0:3000, so an unconditional sidecar fails to "
                f"bind and dies silently behind a successful `docker run -d`."
            )

    return offences


def _self_test() -> int:
    """Each check must actually FIRE on a broken input — a silent validator is worse than none."""
    import copy
    import tempfile

    failures = 0

    def expect(name: str, cond: bool) -> None:
        nonlocal failures
        if not cond:
            failures += 1
            print(f"  SELF-TEST FAIL: {name}")

    expect("the committed config is clean", check(HERE, CRATE) == [])

    baseline = json.loads((HERE / "baseline.json").read_text(encoding="utf-8"))
    subjects = (HERE / "config" / "test-subjects.ttl").read_text(encoding="utf-8")
    run_sh = (HERE / "run.sh").read_text(encoding="utf-8")

    def mutated(baseline_patch=None, subjects_text=None, run_text=None) -> list[str]:
        with tempfile.TemporaryDirectory() as td:
            root = Path(td)
            crate = root / "crate"
            here = crate / "conformance"
            (here / "config").mkdir(parents=True)
            (here / "tls").mkdir()
            (crate / "Cargo.toml").write_text(
                (CRATE / "Cargo.toml").read_text(encoding="utf-8"), encoding="utf-8"
            )
            (here / "run.sh").write_text(run_text or run_sh, encoding="utf-8")
            (here / "config" / "test-subjects.ttl").write_text(
                subjects_text if subjects_text is not None else subjects, encoding="utf-8"
            )
            (here / "config" / "application.yaml").write_text(
                (HERE / "config" / "application.yaml").read_text(encoding="utf-8"), encoding="utf-8"
            )
            (here / "config" / "sparq-lws-core.env").write_text("x=1\n", encoding="utf-8")
            b = copy.deepcopy(baseline)
            if baseline_patch:
                b.update(baseline_patch)
            (here / "baseline.json").write_text(json.dumps(b), encoding="utf-8")
            return check(here, crate)

    expect("the temp mirror of the real config is also clean", mutated() == [])
    expect(
        "C1 fires on a renamed binary",
        any(o.startswith("C1") for o in mutated(
            run_text=run_sh.replace("/target/release/sparq-lws-core", "/target/release/renamed-bin"))),
    )
    expect(
        "C2 fires on a --target with no TestSubject node",
        any(o.startswith("C2") for o in mutated(
            run_text=run_sh.replace('TARGET_SUBJECT="sparq-lws-core"', 'TARGET_SUBJECT="ghost"'))),
    )
    expect(
        "C5 fires when the per-suite counts stop summing to expected_total",
        any(o.startswith("C5") for o in mutated(baseline_patch={"suites": {"protocol": 25, "wac": 15}})),
    )
    expect(
        "C5 fires on an unreachable floor",
        any(o.startswith("C5") for o in mutated(baseline_patch={"min_passed": 999})),
    )
    expect(
        "C6 fires when a skip tag is added to the TestSubject only",
        any(o.startswith("C6") for o in mutated(
            subjects_text=subjects.replace(
                'solid-test:skip "acp" ;', 'solid-test:skip "acp" ;\n    solid-test:skip "extra" ;'))),
    )

    expect(
        "C7 fires when the forwarder start stops being gated on the reachability probe",
        any(o.startswith("C7") for o in mutated(
            run_text=run_sh.replace(
                "if harness_netns_reaches_server; then", "if true; then"))),
    )
    expect(
        "C7 fires when the reachability probe is removed entirely",
        any(o.startswith("C7") for o in mutated(
            run_text=run_sh.replace("harness_netns_reaches_server()", "unused_probe()"))),
    )

    print("self-test: OK" if not failures else f"self-test: {failures} failure(s)")
    return 1 if failures else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--self-test", action="store_true", help="verify each check fires on a broken input")
    args = ap.parse_args()
    if args.self_test:
        return _self_test()

    offences = check(HERE, CRATE)
    for offence in offences:
        print(offence, file=sys.stderr)
    if offences:
        print(f"\nconformance-config: {len(offences)} offence(s).", file=sys.stderr)
        return 1
    print("conformance-config: all clear (C1-C7).")
    return 0


if __name__ == "__main__":
    sys.exit(main())
