// @ts-check
/**
 * ESLint flat config for the Loom TypeScript client.
 *
 * Enforces the house rules that a type-checker cannot: no `any`, no `console.*`
 * in library code, type-only imports written as such, and a file-size guardrail
 * so modules stay small and single-purpose. Generated code and non-source trees
 * are excluded. CI is wired separately by the lead — this config is local.
 */

import js from "@eslint/js";
import globals from "globals";
import tseslint from "typescript-eslint";

export default tseslint.config(
  { ignores: ["dist/**", "node_modules/**", "src/generated.ts", "mock/**"] },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    // Library source: the strict, conformance-enforcing rule set.
    files: ["src/**/*.ts"],
    languageOptions: {
      globals: { ...globals.node, ...globals.browser },
    },
    rules: {
      "@typescript-eslint/no-explicit-any": "error",
      "no-console": "error",
      "@typescript-eslint/consistent-type-imports": "error",
      "max-lines": ["error", { max: 120, skipBlankLines: true, skipComments: true }],
    },
  },
  {
    // Tests and scripts are executable tooling, not library code: they may log,
    // and their length is not a design smell worth guarding.
    files: ["test/**/*.ts", "scripts/**/*.ts", "scripts/**/*.mts"],
    languageOptions: {
      globals: { ...globals.node, ...globals.browser },
    },
    rules: {
      "no-console": "off",
      "max-lines": "off",
    },
  },
);
