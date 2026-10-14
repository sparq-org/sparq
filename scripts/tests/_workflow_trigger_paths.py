"""Read a workflow's `on.<event>.paths` list and the globs a path classifier implies.

Stdlib only: the zk path-classifier suites run without PyYAML.
"""

import re
from pathlib import Path


def trigger_paths(workflow: Path, event: str) -> list[str]:
    """The quoted `paths:` entries under the top-level `on.<event>` trigger."""
    lines = workflow.read_text().splitlines()
    start = lines.index(f"  {event}:") + 1
    globs, in_paths = [], False
    for line in lines[start:]:
        if re.match(r"^ {0,2}\S", line):  # next trigger or top-level key
            break
        if line == "    paths:":
            in_paths = True
        elif in_paths and (m := re.fullmatch(r"      - '([^']+)'", line)):
            globs.append(m.group(1))
        elif in_paths and not line.lstrip().startswith("#"):
            in_paths = False
    return globs


def classifier_globs(prefixes, files) -> list[str]:
    """Filter globs selecting the paths `startswith(prefixes) or in files` selects."""
    return [p + ("**" if p.endswith("/") else "*") for p in prefixes] + sorted(files)
