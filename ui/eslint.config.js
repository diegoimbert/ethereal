import js from "@eslint/js";
import jsxA11y from "eslint-plugin-jsx-a11y";
import reactHooks from "eslint-plugin-react-hooks";
import reactRefresh from "eslint-plugin-react-refresh";
import globals from "globals";
import tseslint from "typescript-eslint";

export default tseslint.config(
  // src/generated/** is written by the Rust → TS type exporter; never lint it.
  { ignores: ["dist", "src/generated/**", "playwright-report", "test-results"] },
  {
    files: ["**/*.{ts,tsx}"],
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    languageOptions: {
      ecmaVersion: 2022,
      globals: { ...globals.browser },
    },
    plugins: {
      "react-hooks": reactHooks,
      "react-refresh": reactRefresh,
      "jsx-a11y": jsxA11y,
    },
    rules: {
      ...jsxA11y.flatConfigs.recommended.rules,
      // Focus moves into what the user just opened (a dialog, a popover, an inline rename
      // field), as the ARIA dialog pattern asks; nothing autofocuses on page load.
      "jsx-a11y/no-autofocus": "off",
      // Key handlers on a container only see keys bubbling from its focusable descendants
      // (shortcut scopes, Escape-to-close, roving focus): not an interaction of the element
      // itself. A focusable container is still caught by no-noninteractive-tabindex.
      "jsx-a11y/no-static-element-interactions": ["error", { allowExpressionValues: true, handlers: ["onClick", "onMouseDown", "onMouseUp"] }],
      // A labelled group that takes focus is an editor's keyboard scope (the piano roll, an
      // automation lane, the warp view): its shortcuts act on what it shows.
      "jsx-a11y/no-noninteractive-tabindex": ["error", { tags: [], roles: ["tabpanel", "group"], allowExpressionValues: true }],
      // The kit's fields render an <input> (a label wrapping one labels it).
      "jsx-a11y/label-has-associated-control": ["error", { controlComponents: ["TextInput", "NumberField"], depth: 3 }],
      "jsx-a11y/no-noninteractive-element-interactions": [
        "error",
        {
          handlers: ["onClick", "onError", "onLoad", "onMouseDown", "onMouseUp"],
          body: ["onError", "onLoad"],
          iframe: ["onError", "onLoad"],
          img: ["onError", "onLoad"],
        },
      ],
      ...reactHooks.configs.recommended.rules,
      "react-refresh/only-export-components": ["warn", { allowConstantExport: true }],
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_", varsIgnorePattern: "^_" }],
    },
  },
  {
    files: ["*.config.{ts,js}", "vitest.setup.ts"],
    languageOptions: { globals: { ...globals.node } },
  },
);
