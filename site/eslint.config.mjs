import { defineConfig, globalIgnores } from "eslint/config";
import nextVitals from "eslint-config-next/core-web-vitals";
import nextTs from "eslint-config-next/typescript";

// Next 16 removed `next lint`; eslint-config-next now ships native flat configs, so they are
// spread directly instead of going through the @eslint/eslintrc FlatCompat shim.
const eslintConfig = defineConfig([
  ...nextVitals,
  ...nextTs,
  globalIgnores(["out/**", ".next/**", "public/wasm/**", "next-env.d.ts"]),
]);

export default eslintConfig;
