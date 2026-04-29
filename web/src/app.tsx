import type { NetworkStats } from '@rumblefish/api-types';
import type { NavigationItem } from '@rumblefish/soroban-block-explorer-ui';

const placeholderNav: NavigationItem[] = [
  { href: '/', label: 'Home' },
  { href: '/transactions', label: 'Transactions' },
];

// Type-only smoke test that `@rumblefish/api-types` is wired into the
// workspace. Replace with a real `useNetworkStats()` call once the
// `/v1/network/stats` page lands.
export type _NetworkStatsImportSmokeTest = Pick<
  NetworkStats,
  'latest_ledger_sequence' | 'tps_60s'
>;

export function App() {
  return (
    <div>
      <h1>Soroban Block Explorer</h1>
      <p>Application scaffold ready.</p>
      <nav>
        {placeholderNav.map((item) => (
          <a key={item.href} href={item.href}>
            {item.label}
          </a>
        ))}
      </nav>
    </div>
  );
}
