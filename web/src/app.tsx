import type { NetworkStats } from '@rumblefish/api-types';
import type { NavigationItem } from '@rumblefish/soroban-block-explorer-ui';

const placeholderNav: NavigationItem[] = [
  { href: '/', label: 'Home' },
  { href: '/transactions', label: 'Transactions' },
];

const placeholderStats: Pick<
  NetworkStats,
  'latest_ledger_sequence' | 'tps_60s'
> = {
  latest_ledger_sequence: 0,
  tps_60s: 0,
};

export function App() {
  return (
    <div>
      <h1>Soroban Block Explorer</h1>
      <p>Application scaffold ready.</p>
      <p>
        Latest ledger: {placeholderStats.latest_ledger_sequence}, TPS (60s):{' '}
        {placeholderStats.tps_60s}
      </p>
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
