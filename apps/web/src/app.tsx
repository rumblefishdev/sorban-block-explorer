import type { NavigationItem } from '@rumblefish/soroban-block-explorer-ui';

const placeholderNav: NavigationItem[] = [
  { href: '/', label: 'Home' },
  { href: '/transactions', label: 'Transactions' },
];

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
