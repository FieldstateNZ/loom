// Flat ESLint config for the Loom Console SPA.
//
// Enforces the Fieldstate `typescript-codebase` house rules that a type-checker
// cannot: no `any`, no `console.*` in shipped code, type-only imports, the
// rules of hooks, and a hard per-file size ceiling so files stay small and
// single-concern. `npm run lint` runs this over `src/`.
import js from "@eslint/js";
import tseslint from "typescript-eslint";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";

export default tseslint.config(
  { ignores: ["dist/**", "node_modules/**", "**/*.tsbuildinfo"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      ecmaVersion: 2022,
      sourceType: "module",
      globals: { ...globals.browser },
    },
    plugins: { "react-hooks": reactHooks },
    rules: {
      ...reactHooks.configs.recommended.rules,

      // House rules the compiler cannot enforce.
      "no-console": "error",
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/consistent-type-imports": [
        "error",
        { prefer: "type-imports", fixStyle: "inline-type-imports" },
      ],

      // Small-files guardrail: components/modules stay well under this ceiling
      // (the house target is 80-100 lines of real code).
      "max-lines": ["error", { max: 150, skipBlankLines: true, skipComments: true }],

      // `tsc` already enforces unused locals/params via noUnused*; keep the lint
      // layer aligned (and let intentionally-unused `_args` through).
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_", varsIgnorePattern: "^_" }],

      // The console reads untrusted JSON, so empty `catch` (fall back to a
      // default) is a deliberate, documented pattern.
      "no-empty": ["error", { allowEmptyCatch: true }],
      "@typescript-eslint/no-unused-expressions": [
        "error",
        { allowShortCircuit: true, allowTernary: true },
      ],
    },
  },
);
