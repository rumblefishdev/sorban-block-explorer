import { defineConfig } from '@hey-api/openapi-ts';

export default defineConfig({
  input: './src/openapi.json',
  output: './src/generated',
  client: '@hey-api/client-fetch',
  plugins: ['@hey-api/typescript', '@hey-api/sdk', '@tanstack/react-query'],
});
