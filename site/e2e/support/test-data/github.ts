// [OPUS-4.8] sq-ymr2e.1 — GitHub releases/latest test-data: the checked-in JSON fixtures the
// hermetic-network layer routes `api.github.com/.../releases/latest` to, plus the matcher that
// recognises that endpoint. The site makes exactly ONE external call in production — the
// unauthenticated releases fetch on /download — so this is the one external surface the
// determinism doctrine (research/web-gui-test-program.md §1.2) fixture-routes. Everything else
// is blocked. Owner-agnostic (`/repos/*/sparq/`) so it survives the org migration
// (sparq-org/sparq → sparq-org/sparq) without a fixture edit.
// Import attribute (`with { type: "json" }`) is required by Node's ESM loader (Playwright runs the
// specs under native ESM); tsc's resolveJsonModule + esbuild both honour it too.
import releaseFixture from "./fixtures/github-release.json" with { type: "json" };
import notFoundFixture from "./fixtures/github-not-found.json" with { type: "json" };
import errorFixture from "./fixtures/github-error.json" with { type: "json" };

/** Which releases/latest response a spec wants the hermetic fixture to serve. `off` = treat the
 *  endpoint as any other external host and BLOCK it (asserts the page degrades with no data). */
export type GithubFixtureName = "release" | "notFound" | "error" | "off";

/** Owner-agnostic regexp for `api.github.com/repos/<owner>/sparq/releases/latest`. */
export const GITHUB_RELEASE_RE =
  /(^|\/\/|\.)api\.github\.com\/repos\/[^/]+\/sparq\/releases\/latest\b/i;

/** Playwright glob form of the same endpoint (for specs that want their own `page.route`). */
export const GITHUB_RELEASE_GLOB = "**/api.github.com/repos/*/sparq/releases/latest";

export type GithubFixtureResponse = { status: number; body: string };

/** Resolve a fixture name to the `{ status, body }` the hermetic route fulfils with. `off` is
 *  handled by the caller (blocked before it reaches here) but mapped defensively to the 404. */
export function githubFixtureResponse(name: GithubFixtureName): GithubFixtureResponse {
  switch (name) {
    case "release":
      return { status: 200, body: JSON.stringify(releaseFixture) };
    case "error":
      return { status: 500, body: JSON.stringify(errorFixture) };
    case "notFound":
    case "off":
      return { status: 404, body: JSON.stringify(notFoundFixture) };
  }
}

/** Does this URL address the releases/latest endpoint the fixture routes? */
export function isGithubReleaseUrl(url: string): boolean {
  return GITHUB_RELEASE_RE.test(url);
}

export { releaseFixture, notFoundFixture, errorFixture };
