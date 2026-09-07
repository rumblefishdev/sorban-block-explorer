// Re-export (not a bare side-effect import) so the emitted `dist/index.d.ts`
// references `./types.d.ts` and its `declare module '@mui/...'` augmentations
// (bodyXsRegular, text.tertiary, …) reach consumers resolving the built package
// types. A side-effect `import './types.js'` is elided from the .d.ts emit.
export type { TypeSurface } from './types.js';

export { colorsLight, colorsDark, type ColorScheme } from './colors.js';
export { grid } from './grid.js';
export { contentLinkSx } from './linkAffordance.js';
export { createPalette } from './palette.js';
export { radius } from './radius.js';
export { shadows } from './shadows.js';
export { createExplorerTheme } from './theme.js';
export { ExplorerThemeProvider, useColorMode } from './ThemeProvider.js';
export { monoFontFamily } from './typography.js';
