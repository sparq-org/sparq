scripts/check-install-action-tool.py
```python
def _indent(line: str) -> int:
    """Number of leading spaces (tabs are invalid YAML indent in these files)."""
    return len(line) - len(line.lstrip(" "))
```

scripts/check-install-action-tool.py
```python
def split_steps(text: str) -> list[list[str]]:
    """Split a workflow file into per-step line blocks.

    A step is a list item (`- ` after some indent). We collect, for each
    `- `-introduced block at a given indent, all subsequent lines indented deeper
    than the dash until the next sibling dash or a dedent — that contiguous run is
    one step's body. Blank/comment lines inside a block are retained.

    The result is a list of blocks; each block is the list of physical lines (no
    trailing newline) belonging to one `- ...` item. Non-step content (top-level
    keys, job headers) never starts a block and is ignored by callers, which only
    act on blocks that contain a `taiki-e/install-action` `uses:`.
    """
    lines = text.splitlines()
    blocks: list[list[str]] = []
    i = 0
    n = len(lines)
    while i < n:
        line = lines[i]
        stripped = line.lstrip(" ")
        if stripped.startswith("- "):
            dash_indent = _indent(line)
            block = [line]
            i += 1
            # Absorb deeper-indented continuation lines (and blanks/comments) until a
            # sibling list item at the same indent or a dedent to <= dash_indent on a
            # non-blank, non-comment line.
            while i < n:
                nxt = lines[i]
                if not nxt.strip() or nxt.lstrip(" ").startswith("#"):
                    block.append(nxt)
                    i += 1
                    continue
                ind = _indent(nxt)
                nstripped = nxt.lstrip(" ")
                if ind == dash_indent and nstripped.startswith("- "):
                    # Next sibling step.
                    break
                if ind <= dash_indent:
                    # Dedent below the dash → end of this step's body.
                    break
                block.append(nxt)
                i += 1
            blocks.append(block)
        else:
            i += 1
    return blocks
```

scripts/check-install-action-tool.py
```python
def _step_uses(block: list[str]) -> tuple[str, str] | None:
    """Return (action, ref) for the step's `uses:` line, else None. The first line of
    a `- uses: ...` block has the dash; strip a leading `- ` before matching."""
    for raw in block:
        s = raw.lstrip(" ")
        if s.startswith("- "):
            s = s[2:]
        m = _USES_RE.match(s)
        if m:
            return m.group(1), m.group(2)
    return None
```

scripts/check-advisory-registry.py
```python
def _strip_shell_comments(line: str) -> str:
    """Drop a trailing/leading shell comment. Crude but adequate: the classifier
    only cares whether a real command mentions a gate-ish script."""
    stripped = line.lstrip()
    if stripped.startswith("#"):
        return ""
    return re.split(r"\s#", line, maxsplit=1)[0].rstrip()
```

scripts/check-advisory-registry.py
```python
def extract_run_commands(block_text: str) -> list[str]:
    """Return the shell text of every `run:` step in a job block.

    [OPUS-5] #3773 — classification must read what the job RUNS, not its prose. The
    previous whole-block scan also matched script paths mentioned in YAML comments
    (gui.yml's tauri-e2e header comment names `support/no-sleep-gate.sh`, which
    attributed the gate to the WRONG job), and shell comments inside a run block are
    likewise not invocations. Handles inline `run: cmd` and block scalars
    (`run: |` / `run: >`), whose body is every following more-indented line.
    """
    commands: list[str] = []
    lines = block_text.splitlines()
    i = 0
    run_re = re.compile(r"^(\s*)(?:-\s+)?run:\s*(.*)$")
    while i < len(lines):
        match = run_re.match(lines[i])
        if not match:
            i += 1
            continue
        indent, inline = match.group(1), match.group(2).strip()
        i += 1
        if inline and inline not in ("|", ">", "|-", ">-", "|+", ">+"):
            commands.append(_strip_shell_comments(inline))
            continue
        body: list[str] = []
        while i < len(lines):
            line = lines[i]
            if line.strip() and (len(line) - len(line.lstrip())) <= len(indent):
                break
            body.append(_strip_shell_comments(line))
            i += 1
        commands.append("\n".join(body))
    return commands
```
