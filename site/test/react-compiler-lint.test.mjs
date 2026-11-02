import { test } from "node:test";
import assert from "node:assert/strict";
import { fileURLToPath } from "node:url";
import { ESLint } from "eslint";

// Exercise the real site config: clean application code alone cannot detect a rule
// being disabled again. These virtual TSX files need no generated assets or browser.
const eslint = new ESLint({ cwd: fileURLToPath(new URL("..", import.meta.url)) });
const filePath = "src/components/compiler-lint-fixture.tsx";
const cases = [
  ["set-state-in-effect", `
    export function Example() {
      const [mounted, setMounted] = React.useState(false);
      React.useEffect(() => setMounted(true), []);
      return <p>{String(mounted)}</p>;
    }
  `],
  ["refs", `
    export function Example() {
      const ref = React.useRef(0);
      return <p>{ref.current}</p>;
    }
  `],
  ["preserve-manual-memoization", `
    export function useExample(store: React.RefObject<{ value: number }>) {
      return React.useCallback(() => store.current.value, [store]);
    }
  `],
  ["static-components", `
    export function Example({ value }: { value: number }) {
      function Inner() { return <p>{value}</p>; }
      return <Inner />;
    }
  `],
];

for (const [rule, source] of cases) {
  test(`site enforces react-hooks/${rule} as an error`, async () => {
    const [result] = await eslint.lintText(`import * as React from "react";\n${source}`, { filePath });
    assert.equal(result.fatalErrorCount, 0, "fixture must parse successfully");
    assert.ok(
      result.messages.some((message) => message.ruleId === `react-hooks/${rule}` && message.severity === 2),
      `expected react-hooks/${rule} error from the site config`,
    );
  });
}

test("site accepts a component with lazy state and complete memo dependencies", async () => {
  const [result] = await eslint.lintText(`
    import * as React from "react";
    export function Example({ value }: { value: number }) {
      const [initial] = React.useState(() => value);
      const doubled = React.useMemo(() => initial * 2, [initial]);
      return <p>{doubled}</p>;
    }
  `, { filePath });
  assert.equal(result.errorCount, 0);
  assert.equal(result.warningCount, 0);
});
