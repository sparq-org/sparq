#!/usr/bin/env python3
"""Issuance hardening: inventory raw commitment signers in non-test crate Rust.

Run from any directory: python3 scripts/check-raw-commitment-signers.py
Stdlib only; no Rust or Noir toolchain is needed. Exceptions pin the path,
signer, argument tokens, occurrence count and reason. Graph issuance uses
SecretKey::attest.
"""

from __future__ import annotations

import argparse
import os
import re
import sys
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
RAW = {
    "sign_commitment", "sign_commitment_with_salt",
    "sign_commitment_with_status", "sign_commitment_with_holder",
}
RAW_QUOTE = re.compile(r'(?:[bc])?r(\#*)"')

# Path relative to crates/, signer, argument tokens, expected occurrences, reason.
ALLOWED = (
    ("sparq-trust/examples/trust_graph_closure.rs", "sign_commitment",
     "& certification_message ( & cert )", 1, "certification; not a graph commitment"),
    ("sparq-trust/examples/admission_cost.rs", "sign_commitment",
     "& certification_message ( & cert )", 2, "certification; not a graph commitment"),
)


@dataclass(frozen=True)
class Token:
    text: str
    line: int


def tokens(source: str) -> list[Token]:
    """Keep identifiers/punctuation, excluding comments and literal contents.

    Preserve line numbers. Delimiters inside literals cannot change test-item
    boundaries; nested block comments and raw/byte/C strings are handled here.
    """
    out = []
    i = 0
    line = 1
    while i < len(source):
        start, start_line = i, line
        if source[i].isspace():
            i += 1
        elif source.startswith("//", i):
            end = source.find("\n", i)
            i = len(source) if end < 0 else end
        elif source.startswith("/*", i):
            i += 2
            depth = 1
            while i < len(source) and depth:
                if source.startswith("/*", i):
                    depth += 1
                    i += 2
                elif source.startswith("*/", i):
                    depth -= 1
                    i += 2
                else:
                    i += 1
        elif raw := RAW_QUOTE.match(source, i):
            suffix = '"' + raw[1]
            end = source.find(suffix, raw.end())
            i = len(source) if end < 0 else end + len(suffix)
        elif source[i] == '"' or (source[i] == "'" and (
            source[i + 2:i + 3] == "'" or source[i + 1:i + 2] == "\\"
        )):
            quote = source[i]
            i += 1
            while i < len(source):
                if source[i] == "\\":
                    i = min(i + 2, len(source))
                elif source[i] == quote:
                    i += 1
                    break
                else:
                    i += 1
        else:
            i += 1
            if source[start].isascii() and (source[start].isalnum() or source[start] == "_"):
                while i < len(source) and source[i].isascii() and (
                    source[i].isalnum() or source[i] == "_"
                ):
                    i += 1
            out.append(Token(source[start:i], start_line))
        line += source.count("\n", start, i)
    return out


def close(t: list[Token], start: int, opening: str, ending: str) -> int:
    depth = 0
    for i in range(start, len(t)):
        if t[i].text == opening:
            depth += 1
        if t[i].text == ending:
            depth -= 1
            if depth == 0:
                return i
    raise ValueError(f"line {t[start].line}: unclosed {opening} in source inventory")


def test_only_attribute(attr: list[str]) -> bool:
    if attr in (["test"], ["cfg", "(", "test", ")"]):
        return True
    if attr[:4] == ["cfg", "(", "all", "("]:
        depth = 0
        for token in attr[4:]:
            if depth == 0 and token == "test":
                return True
            if token == "(":
                depth += 1
            if token == ")":
                depth -= 1
    return False


@dataclass
class Scan:
    calls: list[tuple[int, str, str]] = field(default_factory=list)
    test_modules: list[str] = field(default_factory=list)


def scan(source: str) -> Scan:
    t = tokens(source)
    out = Scan()
    i = 0
    while i < len(t):
        if t[i].text == "#" and i + 1 < len(t) and t[i + 1].text == "[":
            end = close(t, i + 1, "[", "]")
            attr = [token.text for token in t[i + 2:end]]
            if test_only_attribute(attr):
                i = end + 1
                if i + 2 < len(t) and t[i].text == "mod" and t[i + 2].text == ";":
                    out.test_modules.append(t[i + 1].text)
                while i < len(t) and t[i].text not in {"{", ";"}:
                    i += 1
                if i < len(t) and t[i].text == "{":
                    i = close(t, i, "{", "}")
                i += 1
                continue
        if t[i].text in RAW and i > 0 and t[i - 1].text in {".", ":"} and (
            i + 1 < len(t) and t[i + 1].text == "("
        ):
            end = close(t, i + 1, "(", ")")
            args = " ".join(token.text for token in t[i + 2:end])
            out.calls.append((t[i].line, t[i].text, args))
        i += 1
    return out


def rust_files(directory: Path):
    # scandir propagates unreadable-directory errors instead of certifying a partial scan.
    with os.scandir(directory) as entries:
        for entry in sorted(entries, key=lambda e: e.name):
            path = Path(entry.path)
            if entry.is_dir():
                if entry.name not in {"tests", "target"}:
                    yield from rust_files(path)
            elif path.suffix == ".rs":
                yield path


def audit(root: Path) -> list[str]:
    crates = root / "crates"
    scans = {}
    test_files = set()
    for path in rust_files(crates):
        try:
            scanned = scan(path.read_text(encoding="utf-8"))
        except ValueError as exc:
            raise ValueError(f"{path.relative_to(root)}: {exc}") from exc
        for module in scanned.test_modules:
            test_files.add(path.parent / f"{module}.rs")
            test_files.add(path.parent / module / "mod.rs")
        scans[path] = scanned

    counts = Counter()
    findings = []
    for path, scanned in scans.items():
        if path in test_files:
            continue
        relative = path.relative_to(crates).as_posix()
        for line, signer, args in scanned.calls:
            allowed = next((i for i, (p, s, a, _, reason) in enumerate(ALLOWED)
                            if (relative, signer, args) == (p, s, a) and reason), None)
            if allowed is None:
                findings.append(
                    f"crates/{relative}:{line}: {signer} requires an explicit allow-list entry; "
                    "use SecretKey::attest for graph issuance"
                )
            else:
                counts[allowed] += 1
    for i, (path, signer, _, expected, reason) in enumerate(ALLOWED):
        if counts[i] != expected:
            findings.append(
                f"crates/{path}: {signer} ({reason}) changed: expected {expected}, "
                f"found {counts[i]}; use SecretKey::attest for graph issuance"
            )
    return findings


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=REPO_ROOT, help="repository root")
    args = parser.parse_args(argv)
    try:
        findings = audit(args.root)
    except (OSError, UnicodeError, ValueError) as exc:
        findings = [f"{exc}; inventory incomplete — use SecretKey::attest for graph issuance"]
    for finding in findings:
        print(finding, file=sys.stderr)
    print(f"raw-commitment-signers: {'FAIL' if findings else 'PASS'}")
    return int(bool(findings))


if __name__ == "__main__":
    sys.exit(main())
