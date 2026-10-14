import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

// Next 16 removed `next lint`; eslint-config-next now ships native flat configs, so they are
// spread directly instead of going through the @eslint/eslintrc FlatCompat shim.
const eslintConfig = defineConfig([
  ...nextVitals,
  ...nextTs,
  // eslint-config-next 16 pulls in eslint-plugin-react-hooks 7, whose `recommended` set adds the
  // React Compiler diagnostics as errors. Existing code trips the rules below; fixing it means
  // restructuring effects/refs (a behaviour change), tracked in #6986. set-state-in-effect fires
  // across most interactive components, so it is off app-wide; the others are off only in the
  // files that trip them, and every other compiler rule stays on.
  {
    rules: {
      "react-hooks/set-state-in-effect": "off",
    },
  },
  {
    files: ["src/components/workbench/forms-tool.tsx", "src/components/workbench/query-workbench.tsx", "src/lib/engine-context.tsx", "src/lib/workspace-context.tsx"],
    rules: {
      "react-hooks/refs": "off",
    },
  },
  {
    files: ["src/components/workbench/plan-explorer.tsx"],
    rules: {
      "react-hooks/purity": "off",
    },
  },
  globalIgnores(["out/**", ".next/**", "public/wasm/**", "next-env.d.ts"]),
]);

export default eslintConfig;
