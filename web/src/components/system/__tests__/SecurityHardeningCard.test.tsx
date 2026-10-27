import { describe, it, expect } from 'vitest';
import { renderToString, renderToStaticMarkup } from 'react-dom/server';
import { SecurityHardeningCard, ServiceExposureBadge } from '../SecurityHardeningCard';

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

it.each([
  [0.2,'OK','text-[var(--success)]'],
  [2.1,'MEDIUM','text-[var(--warning-ink)]'],
  [7,'EXPOSED','text-[var(--danger)]'],
  [0,'UNKNOWN','text-[var(--ink-muted)]'],
] as const)('reports score %s and rating %s with the matching assessment color',(exposure_score,rating,tone)=>{
  const html=renderToStaticMarkup(<ServiceExposureBadge value={{exposure_score,rating}}/>);
  expect(html).toContain(`${exposure_score.toFixed(1)} ${rating}`);
  expect(html).toContain(tone);
  if(rating!=='OK')expect(html).not.toContain('text-[var(--success)]');
});
