export interface NavigationItem {
  href: string;
  label: string;
}

export {
  colorsLight,
  colorsDark,
  type ColorScheme,
  contentLinkSx,
  grid,
  createPalette,
  radius,
  shadows,
  createExplorerTheme,
  ExplorerThemeProvider,
  useColorMode,
  monoFontFamily,
} from './theme/index.js';

export { Chip, type ChipProps } from './components/Chip.js';
export {
  DebouncedField,
  type DebouncedFieldProps,
} from './components/DebouncedField.js';
export { Dash } from './components/Dash.js';
export { StatusChip } from './components/StatusChip.js';

export {
  AnimatedNumber,
  formatAmount,
  formatCompactAmount,
  formatCompactUsd,
  scaleByDecimals,
  formatFee,
  formatStroops,
  formatTokenAmount,
  formatInteger,
  formatTps,
  formatPercent,
} from './format/index.js';

export {
  useDebouncedDraft,
  useDebounced,
  DEFAULT_DEBOUNCE_MS,
  useCopyToClipboard,
} from './hooks/index.js';

export {
  TableSkeleton,
  CardSkeleton,
  DetailSkeleton,
  SearchSpinner,
  NotFoundState,
  type NotFoundEntity,
  TransientErrorState,
  RateLimitState,
  GenericErrorState,
  QueryErrorState,
  DetailErrorState,
  EmptyState,
  type EmptyStateVariant,
  SectionErrorBoundary,
  classifyError,
  isMissingResource,
  type ErrorKind,
} from './states/index.js';

export {
  RelativeTimestamp,
  PollingIndicator,
  formatRelative,
  useNow,
  LiveNowProvider,
} from './timestamps/index.js';

export {
  ExplorerTable,
  EXPLORER_TABLE_ROW_HEIGHT,
  EXPLORER_TABLE_ROW_HEIGHT_TALL,
  PaginationControls,
  TableSectionHeader,
  TableEmptyState,
  useTableUrlState,
  useCursorPagination,
  usePageHandlers,
} from './table/index.js';
export type {
  ExplorerTableColumn,
  ExplorerTableProps,
  SortDirection,
  PaginationControlsProps,
  TableSectionHeaderProps,
  TableEmptyStateProps,
  TableEmptyKind,
  TableUrlState,
  UseTableUrlStateOptions,
  UseTableUrlStateResult,
  UseCursorPaginationResult,
  PageInfoLike,
  UsePageHandlersResult,
} from './table/index.js';

export {
  NavButton,
  type NavButtonProps,
  type NavButtonSize,
  SearchInput,
  type SearchInputProps,
  type SearchInputSize,
  isMac,
  searchShortcutLabel,
  TopNav,
  type TopNavProps,
  type NetworkStats,
  SecondaryNav,
  type SecondaryNavProps,
  type NavItem,
  ThemeToggle,
  Footer,
  type FooterProps,
  type FooterNavItem,
  PageGridBackdrop,
} from './layout/index.js';

export {
  CopyButton,
  type CopyButtonProps,
  IdentifierDisplay,
  type IdentifierDisplayProps,
  IdentifierWithCopy,
  type IdentifierWithCopyProps,
  LinkComponentProvider,
  useLinkComponent,
  getIdentifierHref,
  routeSegments,
  DEFAULT_TRUNCATION,
  getDefaultTruncation,
  truncateMiddle,
  type EntityType,
  type TruncationConfig,
  isAccountId,
  isAssetId,
  isContractId,
  isLedgerSequence,
  isPoolId,
  isTransactionHash,
  NATIVE_ASSET_CODE,
  isNativeAssetString,
  DomainChip,
  type DomainChipProps,
} from './identifiers/index.js';

export {
  Tabs,
  useTabUrlState,
  TimeSeriesChart,
  LazySection,
} from './visualization/index.js';
export type {
  TabsProps,
  TabDefinition,
  UseTabUrlStateOptions,
  UseTabUrlStateResult,
  TimeSeriesChartProps,
  TimeSeriesCurve,
  TimeSeriesPoint,
  TimeSeriesInterval,
  LazySectionProps,
  UseIntersectionObserverOptions,
  UseIntersectionObserverResult,
} from './visualization/index.js';
