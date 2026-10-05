// [OPUS-4.8] sq-17nw — a minimal ESM hook that transpiles the site's `.ts`
// modules on the fly with the already-installed `typescript` devDependency, so
// `node --test` can import and unit-test pure helpers WITHOUT pulling in a heavy
// test runner (vitest/jest) or a separate build step. Type-erasure only; no
// type-checking (that is the job of `next build`).
import { access, readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import ts from 'typescript';

// [OPUS-4.8] sq-x0kp — rewrite an explicit `.js` import specifier to the sibling `.ts`
// source when that `.ts` exists. The shared `@sparq/client` package uses the NodeNext
// `.js`-extension convention on its INTERNAL imports (e.g. `import … from "./index.js"`),
// which its own `tsc` build needs; but Node's default resolver — which `node --test` uses
// for these on-the-fly-transpiled modules — would look for a literal `index.js` that does
// not exist on disk. Mapping `.js` → `.ts` (only when the `.ts` is present, so real `.js`
// files still resolve normally) lets the unit tests import that package's source unchanged.
// Pure test-support; it changes nothing the bundler / `tsc` see.
export async function resolve(specifier, context, next) {
  // #5114 — the tsconfig `paths` / webpack alias for the shared client, mirrored for node --test.
  if (specifier === '@sparq/client') {
    return next(new URL('../../packages/sparq-client/src/index.ts', import.meta.url).href, context);
  }
  if (specifier.endsWith('.js') && (specifier.startsWith('./') || specifier.startsWith('../'))) {
    try {
      const tsUrl = new URL(specifier.slice(0, -'.js'.length) + '.ts', context.parentURL);
      await access(fileURLToPath(tsUrl));
      return next(tsUrl.href, context);
    } catch {
      // No sibling `.ts` — fall through to the default resolution of the real `.js`.
    }
  }
  // [FABLE-5] Extensionless relative value imports (e.g. `import { curie } from "./curie"`)
  // are what the site's real bundler resolution (tsconfig `moduleResolution: "bundler"`)
  // uses between `src/` modules — but Node's default resolver, which `node --test` uses,
  // requires an explicit extension. Resolve to the sibling `.ts` when it exists, so unit
  // tests can import transpiled modules whose own imports follow the bundler convention.
  if ((specifier.startsWith('./') || specifier.startsWith('../')) && !/\.[^/.]+$/.test(specifier)) {
    try {
      const tsUrl = new URL(specifier + '.ts', context.parentURL);
      await access(fileURLToPath(tsUrl));
      return next(tsUrl.href, context);
    } catch {
      // No sibling `.ts` — fall through to default resolution.
    }
  }
  return next(specifier, context);
}

export async function load(url, context, next) {
  if (url.endsWith('.ts') && !url.endsWith('.d.ts')) {
    const source = await readFile(fileURLToPath(url), 'utf8');
    const { outputText } = ts.transpileModule(source, {
      compilerOptions: {
        module: ts.ModuleKind.ESNext,
        target: ts.ScriptTarget.ES2022,
        isolatedModules: true,
        verbatimModuleSyntax: false,
      },
      fileName: fileURLToPath(url),
    });
    return { format: 'module', source: outputText, shortCircuit: true };
  }
  return next(url, context);
}
