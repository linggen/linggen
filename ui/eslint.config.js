const js = require('@eslint/js');
const globals = require('globals');
const reactHooks = require('eslint-plugin-react-hooks');
const reactRefresh = require('eslint-plugin-react-refresh');
const tseslint = require('typescript-eslint');
const reactRefreshPlugin =
  reactRefresh.default || reactRefresh.reactRefresh || reactRefresh;

module.exports = tseslint.config(
  // Generated from the Rust wire types (scripts/gen-types.sh).
  { ignores: ['dist', 'src/types/generated'] },
  {
    extends: [js.configs.recommended, ...tseslint.configs.recommended],
    files: ['**/*.{ts,tsx}'],
    languageOptions: {
      ecmaVersion: 2020,
      globals: globals.browser,
    },
    plugins: {
      'react-hooks': reactHooks,
      'react-refresh': reactRefreshPlugin,
    },
    rules: {
      ...reactHooks.configs.recommended.rules,
      // Wire shapes come from src/types/generated; `unknown` + a narrow type
      // where a payload really is open.
      '@typescript-eslint/no-explicit-any': 'error',
      '@typescript-eslint/no-unused-vars': [
        'error',
        {
          argsIgnorePattern: '^_',
          varsIgnorePattern: '^_',
          caughtErrorsIgnorePattern: '^_',
        },
      ],
      'react-hooks/set-state-in-effect': 'off',
      'react-refresh/only-export-components': [
        'warn',
        { allowConstantExport: true },
      ],
    },
  },
  // Entry points mount the app; they export nothing by design.
  {
    files: ['src/entries/**/*.tsx'],
    rules: { 'react-refresh/only-export-components': 'off' },
  }
);
