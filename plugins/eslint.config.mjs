import eslint from "@eslint/js";
import tseslint from "typescript-eslint";

export default tseslint.config(
  eslint.configs.recommended,
  ...tseslint.configs.strictTypeChecked,
  {
    files: ["**/*.ts"],
    languageOptions: {
      parserOptions: {
        projectService: true,
        tsconfigRootDir: import.meta.dirname,
      },
    },
    rules: {
      complexity: ["error", 10],
      "max-depth": ["error", 3],
      "max-lines": ["error", { "max": 400, "skipBlankLines": true, "skipComments": true }],
      "max-lines-per-function": [
        "error",
        { "max": 60, "skipBlankLines": true, "skipComments": true }
      ],
      "max-params": ["error", 5],
      "no-console": "off",
    },
  },
);
