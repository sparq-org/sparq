# `.claude/skills/` — internal agent skills

This tree holds **internal** Agent Skills for agents working *on* the engine's
source (parsing perf, ZK circuits, MPC, HDT, etc.). It is distinct from the
top-level [`skills/`](../../skills/) tree, which documents how to *use* sparq's
public surfaces. See the note in [`AGENTS.md`](../../AGENTS.md) ("Skills — how to
USE sparq") — do not confuse the two.

## First-party design / engineering skills

- **`frontend-design/`** — the reusable methodology for sparq's **two** frontends:
  the explanatory marketing-docs website (`site/`) and the operational desktop GUI
  (`gui/`). Covers information architecture + content-reduction (fighting "too
  much text"), explanatory-site vs operational-GUI patterns, the shared visual
  system, and the a11y/perf budget. Grounded in
  [`research/website-redesign.md`](../../research/website-redesign.md) and
  [`research/gui-design.md`](../../research/gui-design.md). Use it whenever you
  restructure the site nav, build/extend the GUI, or decide whether content
  belongs on the site vs the GUI vs a `SKILL.md`.

- **`ast-grep/`** — read code **structure**, not whole files: outline a large
  file to its signatures, structural-grep every impl of a trait or every call
  site of a fn, and know **when** to reach for ast-grep vs Grep vs LSP vs a full
  `Read` (the query-type → tool map). Ships a verified Display-impl rule and the
  `sg`↔`newgrp` collision guard. Grounded in
  [`research/agent-effectiveness-program.md`](../../research/agent-effectiveness-program.md)
  §2.2; the adoption verdict is gated on the shared A/B (`SKILL.md` §5). Use it
  whenever a question is about code *shape* rather than an exact string, or before
  reading a file > ~200 lines.


## Vendored third-party skills

- **`logo-designer/`** — vendored from
  [neonwatty/logo-designer-skill](https://github.com/neonwatty/logo-designer-skill)
  (MIT, © Jeremy Watt; see `logo-designer/LICENSE`). Branding / SVG work:
  designs and iterates on logos as SVG, exports to PNG at standard sizes via
  `scripts/export.sh`. For SVG *generation* via an external service, the
  optional [Recraft](https://www.recraft.ai/) and
  [SVGMaker](https://svgmaker.io/) APIs (REST + MCP) can be wired in with an API
  key — neither is required to use the skill.
