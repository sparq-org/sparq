<!-- [SONNET-4.6] sq-ymr2e.6 — SHRUNK tauri-driver smoke; was sq-pnnl (multi-step harness) -->

# sparq GUI — tauri-driver (WebDriver) e2e smoke

A **max-3-assertion** real-IPC smoke test for the Tauri desktop shell (bead sq-ymr2e.6).
It launches the built shell binary through
[`tauri-driver`](https://v2.tauri.app/develop/tests/webdriver/) (WebDriver), drives the
DISTINCT operational GUI frontend (`gui/app`, NOT the marketing site) in the native WebKit
webview, and makes exactly three assertions over real IPC.

UI-logic coverage (EXPLAIN, SHACL, Import drawer, keyboard spine) lives in the deterministic
Playwright mock-IPC lane at `gui/e2e-playwright` (sq-ymr2e.5) — no real binary required.

## What it proves (`run-e2e.mjs`)

| # | Assertion | What it proves |
|---|-----------|----------------|
| 1/3 | **LAUNCH** | Real binary launched; React workbench frontend mounted in the native webview (`#repl-query` exists). |
| 2/3 | **IPC LIVE** | Native engine came up and signalled `"Engine ready"` over real IPC (top-bar.tsx `status.kind === "ready"`). |
| 3/3 | **ROUND-TRIP** | `SELECT ?name` executed through real IPC, returned `"Alice"` in `data-result-kind="select"`, and the status bar shows a numeric row count. |

Everything else is out of scope here — small surface = small flake budget.

## Determinism

Zero arbitrary sleeps (`await sleep` / `waitForTimeout`). All waits use webdriverio's
event-driven APIs (`waitForExist` / `waitUntil`). The tauri-driver startup port-poll uses a
callback-style `setTimeout` (not an awaited sleep). Validated by `support/no-sleep-gate.sh`.

## Run it locally (Linux only)

Requires the webview system libraries (`libwebkit2gtk-4.1-dev`, …), `webkit2gtk-driver`
(provides `WebKitWebDriver`), and `xvfb`. Then:

```bash
# 1. Build the frontend the webview embeds (root-relative).
cd js && npm install && npm run build:wasm
cd ../gui/app && npm install && npm run build:tauri

# 2. Build the shell binary (embeds gui/app/out at compile time).
cd ../src-tauri && cargo build

# 3. Install the pinned driver, install deps, run under a virtual display.
cargo install tauri-driver@2.0.6 --locked
cd ../e2e && npm ci
xvfb-run --auto-servernum npm run test:e2e
```

`APP_BINARY` (absolute path to the built shell binary), `TAURI_DRIVER_BIN`, and `ARTIFACTS_DIR`
can override the defaults. CI sets `APP_BINARY` and `ARTIFACTS_DIR` explicitly.

## Failure artifacts

On assertion failure, the harness writes to `artifacts/` (git-ignored):
- `artifacts/screenshot.png` — WebDriver screenshot of the webview at failure time.
- `artifacts/tauri-driver.log` — collected stdout/stderr from the tauri-driver process.

The CI `upload-artifact` step collects these for 7 days so failures are diagnosable.

## CI wiring

Wired as the `tauri-e2e` job in [`.github/workflows/gui.yml`](../../.github/workflows/gui.yml).

- `tauri-driver` is pinned to **2.0.6** (`cargo install tauri-driver@2.0.6 --locked`).
- `WebKitWebDriver` comes from the runner's `webkit2gtk-driver` apt package (matched to the
  runner image's webkit2gtk; the version is logged in CI for drift detection).
- `xvfb-run --auto-servernum` provides the headless display.
- Single process (workers=1 by construction).
- Shell-level retry 1 for xvfb startup races; failure artifacts uploaded either way.
- Job name carries `(Linux, advisory)` and is declared in `.github/advisory-registry.json`.
- `continue-on-error: true` ensures a driver/webview flake never turns the run red.

## Flake probe

A `workflow_dispatch` trigger on `gui.yml` runs the smoke N times (default 20) to measure
consecutive-green rate — the evidence needed for the acceptance criteria (20/20 green).
To run: GitHub Actions UI → "Run workflow" → set `probe_runs`. Link the run in the PR body.

Promotion to a hard gate (drop `advisory` + `continue-on-error`) is tracked by bead `sq-var9`.
