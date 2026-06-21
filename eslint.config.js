import js from "@eslint/js";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import globals from "globals";
import tseslint from "typescript-eslint";

export default tseslint.config(
  {
    ignores: [
      "src/i18n/compare_locales.js",
      "dist/",
      "src-tauri/target/",
      "website/",
      ".cargo_home/",
      ".npm-cache/",
      "scripts/",
      "extension/",
      "public/",
      "test_format/",
    ],
  },
  // Node.js 脚本（构建、工具等）使用 node globals
  {
    files: ["scripts/**/*.mjs", "scripts/**/*.js", "src-tauri/scripts/**/*.mjs", "src-tauri/scripts/**/*.js"],
    languageOptions: {
      globals: globals.node,
    },
    rules: {
      "no-undef": "off", // node globals 已覆盖
      "@typescript-eslint/no-require-imports": "off",
    },
  },
  js.configs.recommended,
  ...tseslint.configs.recommended,
  {
    languageOptions: {
      ecmaVersion: 2020,
      globals: globals.browser,
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
      "no-unused-vars": "off",
      "@typescript-eslint/no-unused-vars": ["warn", {
        argsIgnorePattern: "^_",
        varsIgnorePattern: "^_",
        caughtErrorsIgnorePattern: "^_",
      }],
      "react-hooks/exhaustive-deps": "warn",
      "@typescript-eslint/ban-ts-comment": "warn",
    },
  },
  {
    files: ["**/__tests__/**", "**/test/**", "**/*.test.ts", "**/*.test.tsx", "**/*.spec.ts", "**/*.spec.tsx"],
    rules: {
      "@typescript-eslint/no-explicit-any": "off",
      "@typescript-eslint/no-unused-vars": "off",
      "@typescript-eslint/ban-ts-comment": "off",
    },
  },
  {
    files: ["**/*.d.ts"],
    rules: {
      "@typescript-eslint/no-unused-vars": "off",
      "@typescript-eslint/ban-ts-comment": "off",
    },
  },
  {
    files: ["src/components/workflow/Nodes/*.tsx"],
    rules: {
      "@typescript-eslint/no-unused-vars": "off",
    },
  },
);
