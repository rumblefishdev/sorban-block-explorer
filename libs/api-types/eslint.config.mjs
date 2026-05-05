import baseConfig from '../../eslint.config.mjs';

export default [
  ...baseConfig,
  {
    files: ['**/*.json'],
    rules: {
      '@nx/dependency-checks': [
        'error',
        {
          ignoredFiles: ['{projectRoot}/eslint.config.{js,cjs,mjs,ts,cts,mts}'],
          // Imported only inside the lint-ignored src/generated/ tree.
          ignoredDependencies: ['@tanstack/react-query'],
        },
      ],
    },
    languageOptions: {
      parser: (await import('jsonc-eslint-parser')).default,
    },
  },
  {
    ignores: ['**/out-tsc/**', 'src/generated/**/*'],
  },
];
