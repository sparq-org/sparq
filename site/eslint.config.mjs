import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

// Next 16 removed `next lint`; eslint-config-next now ships native flat configs, so they are
// spread directly instead of going through the @eslint/eslintrc FlatCompat shim.
const eslintConfig = defineConfig([
  ...nextVitals,
  ...nextTs,
  {
    // eslint-config-next 16 pulls in eslint-plugin-react-hooks 7, whose `recommended` set adds
    // the React Compiler diagnostics as errors. The Next 15 config enforced only rules-of-hooks
    // + exhaustive-deps; the compiler rules below are the ones existing components trip, and
    // satisfying them means restructuring effects/refs (a behaviour change, not a dependency
    // bump). They stay off here so the Next 16 move is lint-neutral; the remaining compiler
    // rules are already clean and stay on as a ratchet.
    rules: {
      "react-hooks/set-state-in-effect": "off",
      "react-hooks/refs": "off",
      "react-hooks/static-components": "off",
      "react-hooks/preserve-manual-memoization": "off",
      "react-hooks/purity": "off",
    },
  },
  globalIgnores(["out/**", ".next/**", "public/wasm/**", "next-env.d.ts"]),
]);

export default eslintConfig;
