// Tolerant reader for `npm pack --dry-run --json` stdout. [SONNET-4.6] issue #5328.
//
// WHY THIS EXISTS
// ---------------
// `npm pack --dry-run --json` prints its file manifest as JSON on stdout, but it also
// runs the package's `prepack`/`prepare` lifecycle scripts with THEIR stdout inherited
// into the same stream. So any lifecycle script that prints a progress line to stdout
// prepends that line to the JSON, and a whole-stream `JSON.parse` throws — which
// guardrails/check-package.mjs would then report as `the package is not packable`, a
// misleading diagnosis of what is really a logging bug one script away.
//
// The scripts in that chain are all expected to keep stdout clean (they log to stderr),
// but that is an invariant a future edit can break silently. Parsing only the TRAILING
// JSON value makes such a regression degrade to harmless noise instead of a false
// "not packable" verdict.
//
// npm emits the JSON value last and at column 0, while every nested bracket in its
// pretty-printed output is indented — so a line-start `[`/`{` is a reliable candidate
// for where the value begins. Candidates are tried newest-first, and the clean case
// (no noise at all) never gets that far.

/**
 * Parse the trailing JSON value out of a possibly stdout-polluted stream.
 *
 * @param {string} out raw stdout of `npm pack --dry-run --json`
 * @returns {unknown} the parsed JSON value
 * @throws {SyntaxError} if no suffix of `out` parses as JSON
 */
export function parsePackJson(out) {
  let firstError;
  try {
    return JSON.parse(out);
  } catch (err) {
    firstError = err;
  }

  const starts = [];
  for (let i = 0; i < out.length; i += 1) {
    const atLineStart = i === 0 || out[i - 1] === '\n';
    if (atLineStart && (out[i] === '[' || out[i] === '{')) starts.push(i);
  }

  for (let i = starts.length - 1; i >= 0; i -= 1) {
    try {
      return JSON.parse(out.slice(starts[i]));
    } catch {
      // Not the start of the value (or the value is followed by trailing noise) — keep
      // walking backwards; if none of the candidates parse, the original error stands.
    }
  }

  throw firstError;
}
