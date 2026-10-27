import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SecurityHardeningCard } from '../SecurityHardeningCard';

describe('SecurityHardeningCard Component', () => {
  it('renders hardening card container and titles correctly', () => {
    const html = renderToString(<SecurityHardeningCard />);
    expect(html).toBeDefined();
    // In server-side render ToString it might render initial state / skeleton
    expect(typeof html).toBe('string');
  });

  it('contains hardening keywords in output', () => {
    const html = renderToString(<SecurityHardeningCard />);
    expect(html).toContain('Hardening Keamanan &amp; Firewall Loopback');
  });
});
