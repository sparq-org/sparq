#!/usr/bin/env python3
# sq-fvzi6: hermetic regression test for the sparq-server release image contract.
#
# The workflow itself is the executable publication surface, so this test parses its docker
# job and pins the load-bearing shape: QEMU before Buildx, a native load/smoke/Trivy gate,
# semver/minor/latest tags, and one amd64+arm64 push with SBOM + provenance. The mutation test
# proves the platform assertion is non-vacuous.
#
# sq-fqrv4: the shared deployment substrate also publishes sparq-lws-core. Keep its
# independent release workflow under the same multi-arch and pre-push assurance contract so
# either server image regressing to a single architecture or an unscanned push fails this gate.
#
# sq-w1dxx: also pins the trigger-dependent TAG contract. `docker/metadata-action`
# derives `type=semver` from the git ref, which on a `workflow_dispatch` is the branch — so an
# unguarded tag list makes a dispatch dev-build push only the moving `:latest` (the tag the
# deploy configs pin) with no clean semver. The assertions below require every tag to be
# version-threaded from the `setup` job and the canonical/dispatch tag sets to be disjoint;
# `test_ungated_latest_tag_fails_contract` proves that assertion is non-vacuous.
#
# the structural assertions above are necessary but NOT sufficient: `inputs.tag` is a
# free-form required string, so gating the dispatch tag on the trigger still let `tag: latest`
# (or `v1.2.3`, or a moving `1.2`) resolve onto the canonical tag it names and overwrite the
# released image. `resolve_pushed_tags` therefore MODELS metadata-action's resolution and
# `assert_dispatch_tags_never_canonical` drives it with adversarial inputs, proving the dispatch
# path cannot emit a deployment-followed tag whatever the developer types.
# `test_unnamespaced_dispatch_tag_fails_contract` proves THAT assertion is non-vacuous.
#
# Run: python3 scripts/tests/test_release_container_multiarch.py

from __future__ import annotations

import re
import unittest
from pathlib import Path

import sys

# #5820: local runs without PyYAML skip this module instead of erroring; CI still
# hard-fails on a missing PyYAML (see scripts/tests/_yaml_seam.py).
sys.path.insert(0, str(Path(__file__).resolve().parent))
from _yaml_seam import yaml_or_local_skip  # noqa: E402

yaml = yaml_or_local_skip()

REPO_ROOT = Path(__file__).resolve().parent.parent.parent
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "release.yml"
LWS_WORKFLOW = REPO_ROOT / ".github" / "workflows" / "lws-container.yml"

VERSION_EXPR = "${{ needs.setup.outputs.version }}"
METADATA_STEP = "Image metadata (tags + OCI labels from the resolved release version)"

# A tag a deployment can follow: the literal `latest`, or a semver-shaped tag. Every canonical
# tag the release path publishes has one of these shapes, and `deploy/paas`, `deploy/aws`,
# `deploy/gcp`, `deploy/azure` + `deploy/helm` all pin `:latest` or a pinned `X.Y.Z`.
CANONICAL_SHAPE = re.compile(r"(?:latest|\d+\.\d+(?:\.\d+)?(?:[-+].*)?)\Z")

# Inputs a developer could plausibly type into the `tag` dispatch input — including the ones that
# NAME a canonical tag outright. Each must resolve to a non-canonical image tag.
ADVERSARIAL_DISPATCH_TAGS = (
    "latest",  # names the moving tag every deploy config pins
    "v1.2.3",  # a stable semver release tag
    "1.2.3",  # ... without the `v` prefix metadata-action strips anyway
    "v1.2",  # a moving major.minor tag
    "1.2",
    "v0.1.0",  # the version `deploy/paas` documents as a pinnable release
    "stable",
    "v0.1.0-dev",  # the well-behaved input the dispatch input description recommends
)


def _step_named(steps: list[dict], name: str) -> tuple[int, dict]:
    """Return one named workflow step and fail if the contract is absent or ambiguous."""
    matches = [(index, step) for index, step in enumerate(steps) if step.get("name") == name]
    assert len(matches) == 1, f"expected exactly one {name!r} step, found {len(matches)}"
    return matches[0]


def _evaluate_enable(expression: str, event_name: str) -> bool:
    """Evaluate a tag's `enable=` guard for a given trigger.

    Deliberately narrow: only the two `github.event_name` comparisons the workflow uses are
    understood, and anything else raises. A gate this model cannot evaluate must go RED rather
    than be silently treated as enabled/disabled — that would make the tests below vacuous.
    """
    match = re.fullmatch(
        r"\$\{\{\s*github\.event_name\s*(==|!=)\s*'([^']*)'\s*\}\}", expression.strip()
    )
    assert match, f"unsupported enable expression {expression!r}"
    operator, literal = match.group(1), match.group(2)
    return event_name == literal if operator == "==" else event_name != literal


def _semver_tags(value: str, pattern: str) -> tuple[str, ...]:
    """Model `type=semver` resolution: strip a leading `v`, then apply the pattern.

    metadata-action warns and SKIPS a value it cannot parse as semver, so an unparseable value
    contributes no tag. For a pre-release the real action also suppresses `{{major}}.{{minor}}`;
    this model emits it anyway, which is conservative — it can only ever credit the canonical
    path with MORE tags, making the disjointness assertions stricter, never weaker.
    """
    parsed = re.fullmatch(r"v?(\d+)\.(\d+)\.(\d+)(?:[-+].*)?", value)
    if not parsed:
        return ()
    if pattern == "{{version}}":
        return (value[1:] if value.startswith("v") else value,)
    if pattern == "{{major}}.{{minor}}":
        return (f"{parsed.group(1)}.{parsed.group(2)}",)
    raise AssertionError(f"unsupported semver pattern {pattern!r}")


def resolve_pushed_tags(workflow_text: str, event_name: str, version: str) -> set[str]:
    """Resolve the concrete image tags the docker job would push for a trigger + version.

    A MODEL of `docker/metadata-action`, not the action itself — it exists so the tests can ask
    the question that matters ("what does a hostile `inputs.tag` actually publish?") instead of
    only pattern-matching the tag list's source text.
    """
    workflow = yaml.safe_load(workflow_text)
    _, metadata = _step_named(workflow["jobs"]["docker"]["steps"], METADATA_STEP)
    resolved: set[str] = set()
    for line in filter(None, (line.strip() for line in metadata["with"]["tags"].splitlines())):
        directives = dict(part.split("=", 1) for part in line.split(","))
        if "enable" in directives and not _evaluate_enable(directives["enable"], event_name):
            continue
        value = directives.get("value", "").replace(VERSION_EXPR, version)
        kind = directives["type"]
        if kind == "raw":
            resolved.add(value)
        elif kind == "semver":
            resolved.update(_semver_tags(value, directives["pattern"]))
        else:
            raise AssertionError(f"unsupported tag type {kind!r}")
    return resolved


def assert_dispatch_tags_never_canonical(workflow_text: str) -> None:
    """No `workflow_dispatch` input may publish a tag a deployment follows.

    The trigger gate alone cannot provide this: `inputs.tag` is unconstrained, so the property
    has to hold for adversarial input, not just for the `-dev` suffix the input description asks
    for. The canonical set is taken from a real `v1.2.3` release push so the disjointness claim
    in the job header is checked against the tags that path actually publishes.
    """
    canonical = resolve_pushed_tags(workflow_text, "push", "v1.2.3")
    assert canonical == {"1.2.3", "1.2", "latest"}, f"unexpected canonical tag set: {canonical}"

    for dispatch_tag in ADVERSARIAL_DISPATCH_TAGS:
        pushed = resolve_pushed_tags(workflow_text, "workflow_dispatch", dispatch_tag)
        # A dispatch must still publish something — a fix that silences the collision by pushing
        # nothing would break the developer/test path this trigger exists to serve.
        assert pushed, f"dispatch with tag={dispatch_tag!r} published no image tag at all"
        collisions = pushed & canonical
        assert not collisions, (
            f"dispatch with tag={dispatch_tag!r} overwrites canonical tag(s) {sorted(collisions)}"
        )
        followed = {tag for tag in pushed if CANONICAL_SHAPE.fullmatch(tag)}
        assert not followed, (
            f"dispatch with tag={dispatch_tag!r} published deployment-followed tag(s) "
            f"{sorted(followed)}; dispatch tags must be namespaced out of the canonical space"
        )


def assert_release_container_contract(workflow_text: str) -> None:
    """Assert the release job emits the documented multi-platform image without losing gates."""
    workflow = yaml.safe_load(workflow_text)
    docker_job = workflow["jobs"]["docker"]
    steps = docker_job["steps"]

    # sq-w1dxx: the tags are threaded from the `setup` job's resolved version, so the
    # dependency edge that makes `needs.setup.outputs.version` resolvable is load-bearing.
    needs = docker_job.get("needs")
    needs = [needs] if isinstance(needs, str) else (needs or [])
    assert "setup" in needs, "docker job must depend on `setup` to read the resolved version"

    qemu_index, qemu = _step_named(steps, "Set up QEMU for arm64")
    buildx_indices = [
        index
        for index, step in enumerate(steps)
        if str(step.get("uses", "")).startswith("docker/setup-buildx-action@")
    ]
    assert len(buildx_indices) == 1, "release docker job must set up Buildx exactly once"
    assert qemu_index < buildx_indices[0], "QEMU must be registered before Buildx"
    assert re.fullmatch(
        r"docker/setup-qemu-action@[0-9a-f]{40}", qemu.get("uses", "")
    ), "QEMU action must be pinned to a full commit SHA"
    assert qemu.get("with", {}).get("platforms") == "arm64"

    _, metadata = _step_named(steps, METADATA_STEP)
    tags = set(filter(None, metadata["with"]["tags"].splitlines()))
    # sq-w1dxx: every tag is threaded from the `setup` job's resolved version (`value=`)
    # and gated on the trigger (`enable=`), so the two paths emit disjoint tag sets: a `v*` tag
    # push publishes semver + major.minor + `latest`; a developer/test dispatch publishes ONLY its
    # own `dev-`-namespaced version tag — never `latest` (which `deploy/paas` + `deploy/aws` pin)
    # and never a moving major.minor. The `dev-` prefix is what holds that apart for an
    # unconstrained `inputs.tag`; `assert_dispatch_tags_never_canonical` checks the consequence.
    version = VERSION_EXPR
    not_dispatch = "enable=${{ github.event_name != 'workflow_dispatch' }}"
    is_dispatch = "enable=${{ github.event_name == 'workflow_dispatch' }}"
    assert tags == {
        f"type=semver,pattern={{{{version}}}},value={version},{not_dispatch}",
        f"type=semver,pattern={{{{major}}}}.{{{{minor}}}},value={version},{not_dispatch}",
        f"type=raw,value=latest,{not_dispatch}",
        f"type=raw,value=dev-{version},{is_dispatch}",
    }, "release image tags must stay version-threaded, with `latest` gated off dispatch"

    smoke_index, smoke = _step_named(steps, "Build image for smoke test (load locally)")
    smoke_with = smoke["with"]
    assert smoke_with.get("load") is True and smoke_with.get("push") is False
    assert smoke_with.get("platforms") == "linux/amd64"
    assert smoke_with.get("tags") == "sparq-server:smoke"

    smoke_run_index, smoke_run = _step_named(
        steps, "Smoke test the image (docker run + /health + ASK query)"
    )
    assert smoke_run.get("run") == "scripts/docker-smoke.sh sparq-server:smoke 3030"

    trivy_index, trivy = _step_named(
        steps, "Trivy scan released image (fail on fixable HIGH/CRITICAL)"
    )
    assert trivy["with"].get("image-ref") == "sparq-server:smoke"
    assert trivy["with"].get("exit-code") == "1", "Trivy release scan must remain gating"

    push_index, push = _step_named(steps, "Build and push")
    assert smoke_index < smoke_run_index < trivy_index < push_index, (
        "the native artifact must be smoked and scanned before any registry push"
    )
    push_with = push["with"]
    assert push_with.get("push") is True
    platforms = {value.strip() for value in push_with.get("platforms", "").split(",")}
    assert platforms == {"linux/amd64", "linux/arm64"}
    assert push_with.get("tags") == "${{ steps.meta.outputs.tags }}"
    assert push_with.get("labels") == "${{ steps.meta.outputs.labels }}"
    assert push_with.get("provenance") == "mode=max"
    assert push_with.get("sbom") is True


def assert_lws_container_contract(workflow_text: str) -> None:
    """Assert the Solid/LWS release emits the same assured multi-platform image shape."""
    workflow = yaml.safe_load(workflow_text)
    publish_job = workflow["jobs"]["publish"]
    steps = publish_job["steps"]

    permissions = publish_job["permissions"]
    assert permissions.get("packages") == "write"

    qemu_index, qemu = _step_named(steps, "Set up QEMU for arm64")
    buildx_indices = [
        index
        for index, step in enumerate(steps)
        if str(step.get("uses", "")).startswith("docker/setup-buildx-action@")
    ]
    assert len(buildx_indices) == 1, "LWS release job must set up Buildx exactly once"
    assert re.fullmatch(
        r"docker/setup-qemu-action@[0-9a-f]{40}", qemu.get("uses", "")
    ), "LWS QEMU action must be pinned to a full commit SHA"
    assert qemu.get("with", {}).get("platforms") == "arm64"

    _, metadata = _step_named(steps, "Generate stable release tags and OCI labels")
    assert metadata["with"].get("images") == "ghcr.io/sparq-org/sparq-lws-core"

    smoke_index, smoke = _step_named(steps, "Build native amd64 image for smoke and scan")
    smoke_with = smoke["with"]
    assert smoke_with.get("file") == "crates/sparq-lws-core/Dockerfile"
    assert smoke_with.get("load") is True and smoke_with.get("push") is False
    assert smoke_with.get("platforms") == "linux/amd64"
    assert smoke_with.get("tags") == "sparq-lws-core:smoke"

    smoke_run_index, smoke_run = _step_named(
        steps, "Smoke native image (health, fail-closed mutation, non-root)"
    )
    assert (
        smoke_run.get("run")
        == "crates/sparq-lws-core/tests/container-smoke.sh sparq-lws-core:smoke 3000"
    )
    assert smoke_run.get("env", {}).get("LWS_CONTAINER_SKIP_BUILD") == "1", (
        "the smoke must reuse the Buildx-gated image, not rebuild its own"
    )

    trivy_index, trivy = _step_named(
        steps, "Scan native image (fail on fixable HIGH or CRITICAL)"
    )
    assert trivy["with"].get("image-ref") == "sparq-lws-core:smoke"
    assert trivy["with"].get("exit-code") == "1", "LWS Trivy release scan must remain gating"

    push_index, push = _step_named(steps, "Publish amd64 + arm64 image index")
    assert smoke_index < smoke_run_index < trivy_index < qemu_index < push_index, (
        "the native artifact must be smoked and scanned before emulation and any registry push"
    )
    push_with = push["with"]
    assert push_with.get("file") == "crates/sparq-lws-core/Dockerfile"
    assert push_with.get("push") is True
    platforms = {value.strip() for value in push_with.get("platforms", "").split(",")}
    assert platforms == {"linux/amd64", "linux/arm64"}
    assert push_with.get("tags") == "${{ steps.meta.outputs.tags }}"
    assert push_with.get("labels") == "${{ steps.meta.outputs.labels }}"
    assert push_with.get("provenance") == "mode=max"
    assert push_with.get("sbom") is True


class ReleaseContainerMultiarch(unittest.TestCase):
    def test_live_release_workflow_has_multiarch_manifest_and_native_gates(self):
        assert_release_container_contract(WORKFLOW.read_text(encoding="utf-8"))

    def test_dropping_arm64_descriptor_fails_contract(self):
        original = WORKFLOW.read_text(encoding="utf-8")
        mutated = original.replace(
            "platforms: linux/amd64,linux/arm64", "platforms: linux/amd64", 1
        )
        self.assertNotEqual(mutated, original, "mutation fixture did not alter the workflow")
        with self.assertRaises(AssertionError):
            assert_release_container_contract(mutated)

    def test_live_lws_workflow_has_multiarch_manifest_and_native_gates(self):
        assert_lws_container_contract(LWS_WORKFLOW.read_text(encoding="utf-8"))

    def test_dropping_lws_arm64_descriptor_fails_contract(self):
        original = LWS_WORKFLOW.read_text(encoding="utf-8")
        mutated = original.replace(
            "platforms: linux/amd64,linux/arm64", "platforms: linux/amd64", 1
        )
        self.assertNotEqual(mutated, original, "mutation fixture did not alter the workflow")
        with self.assertRaises(AssertionError):
            assert_lws_container_contract(mutated)

    def test_moving_lws_scan_after_push_fails_contract(self):
        original = LWS_WORKFLOW.read_text(encoding="utf-8")
        workflow = yaml.safe_load(original)
        steps = workflow["jobs"]["publish"]["steps"]
        trivy_index, trivy = _step_named(
            steps, "Scan native image (fail on fixable HIGH or CRITICAL)"
        )
        push_index, _ = _step_named(steps, "Publish amd64 + arm64 image index")
        steps.pop(trivy_index)
        steps.insert(push_index, trivy)
        with self.assertRaises(AssertionError):
            assert_lws_container_contract(yaml.safe_dump(workflow, sort_keys=False))

    def test_live_dispatch_tags_never_collide_with_canonical_tags(self):
        """a hostile/careless `inputs.tag` must not reach a deployment-followed tag."""
        assert_dispatch_tags_never_canonical(WORKFLOW.read_text(encoding="utf-8"))

    def test_dispatch_still_publishes_a_pullable_namespaced_tag(self):
        """The dev path stays useful: the recommended input yields exactly one namespaced tag."""
        pushed = resolve_pushed_tags(
            WORKFLOW.read_text(encoding="utf-8"), "workflow_dispatch", "v0.1.0-dev"
        )
        self.assertEqual(pushed, {"dev-v0.1.0-dev"})

    def test_unnamespaced_dispatch_tag_fails_contract(self):
        """dropping the `dev-` namespace must go red — the pre-fix collision returns."""
        original = WORKFLOW.read_text(encoding="utf-8")
        mutated = original.replace(
            f"type=raw,value=dev-{VERSION_EXPR},", f"type=raw,value={VERSION_EXPR},", 1
        )
        self.assertNotEqual(mutated, original, "mutation fixture did not alter the workflow")
        with self.assertRaises(AssertionError):
            assert_dispatch_tags_never_canonical(mutated)

    def test_unevaluatable_enable_guard_fails_contract(self):
        """A tag gate this model cannot evaluate must go red, never silently pass."""
        original = WORKFLOW.read_text(encoding="utf-8")
        mutated = original.replace(
            "enable=${{ github.event_name != 'workflow_dispatch' }}",
            "enable=${{ github.actor != 'nobody' }}",
            1,
        )
        self.assertNotEqual(mutated, original, "mutation fixture did not alter the workflow")
        with self.assertRaises(AssertionError):
            assert_dispatch_tags_never_canonical(mutated)

    def test_ungated_latest_tag_fails_contract(self):
        """sq-w1dxx: reinstating the pre-fix tag list must go red."""
        original = WORKFLOW.read_text(encoding="utf-8")
        mutated = original.replace(
            "type=raw,value=latest,enable=${{ github.event_name != 'workflow_dispatch' }}",
            "type=raw,value=latest",
            1,
        )
        self.assertNotEqual(mutated, original, "mutation fixture did not alter the workflow")
        with self.assertRaises(AssertionError):
            assert_release_container_contract(mutated)

    def test_dropping_the_version_thread_fails_contract(self):
        """sq-w1dxx: reverting the tags to git-ref-derived semver must go red."""
        original = WORKFLOW.read_text(encoding="utf-8")
        mutated = original.replace(",value=${{ needs.setup.outputs.version }},enable=", ",enable=")
        self.assertNotEqual(mutated, original, "mutation fixture did not alter the workflow")
        with self.assertRaises(AssertionError):
            assert_release_container_contract(mutated)

    def test_dropping_the_setup_dependency_fails_contract(self):
        """sq-w1dxx: `needs.setup.outputs.version` is unresolvable without the edge."""
        original = WORKFLOW.read_text(encoding="utf-8")
        # Anchor on `packages: write`, which is unique to the docker job — four other jobs also
        # declare `needs: setup`, so a bare replace would mutate the wrong one.
        mutated = original.replace(
            "    needs: setup\n    permissions:\n      contents: read\n      packages: write",
            "    permissions:\n      contents: read\n      packages: write",
            1,
        )
        self.assertNotEqual(mutated, original, "mutation fixture did not alter the workflow")
        with self.assertRaises(AssertionError):
            assert_release_container_contract(mutated)


if __name__ == "__main__":
    unittest.main(verbosity=2)
