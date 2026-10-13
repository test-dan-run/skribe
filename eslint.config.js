import js from "@eslint/js";
import { defineConfig, globalIgnores } from "eslint/config";
import reactHooks from "eslint-plugin-react-hooks";
import globals from "globals";
import tseslint from "typescript-eslint";

const tauriOnlyInIpc = {
  group: ["@tauri-apps/*"],
  message: "Only src/ipc may talk to Tauri. Call commands through the typed wrapper in src/ipc.",
};

export default defineConfig(
  // src/ipc/generated is produced from Rust by `npm run ipc:generate`.
  globalIgnores(["dist/", "target/", "src-tauri/", "src/ipc/generated/"]),

  js.configs.recommended,

  {
    files: ["**/*.{ts,tsx}"],
    extends: [tseslint.configs.strictTypeChecked, tseslint.configs.stylisticTypeChecked],
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
  },

  {
    files: ["src/**/*.{ts,tsx}"],
    extends: [reactHooks.configs.flat.recommended],
    languageOptions: { globals: globals.browser },
    rules: {
      "@typescript-eslint/consistent-type-imports": "error",
      "@typescript-eslint/switch-exhaustiveness-check": "error",
    },
  },

  // Module boundaries (IMPLEMENTATION_PLAN.md section 3.1).
  {
    files: ["src/**/*.{ts,tsx}"],
    ignores: ["src/ipc/**"],
    rules: {
      "@typescript-eslint/no-restricted-imports": ["error", { patterns: [tauriOnlyInIpc] }],
    },
  },
  {
    // Components render state and raise intents; only controller hooks (.ts) call commands.
    files: ["src/**/*.tsx"],
    ignores: ["src/**/*.test.tsx"],
    rules: {
      "@typescript-eslint/no-restricted-imports": [
        "error",
        {
          patterns: [
            tauriOnlyInIpc,
            {
              regex: "/ipc(/.*)?$",
              allowTypeImports: true,
              message: "Components may import IPC types only. Call commands from a controller hook.",
            },
          ],
        },
      ],
    },
  },

  {
    files: ["*.{js,ts}", "scripts/**/*.mjs"],
    languageOptions: { globals: globals.node },
  },
  {
    files: ["**/*.{js,mjs}"],
    extends: [tseslint.configs.disableTypeChecked],
  },
);
