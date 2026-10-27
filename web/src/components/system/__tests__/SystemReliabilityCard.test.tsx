import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SystemReliabilityCard } from '../SystemReliabilityCard';

describe('SystemReliabilityCard Component', () => {
  it('renders reliability card container and titles correctly', () => {
    const html = renderToString(<SystemReliabilityCard />);
    expect(html).toBeDefined();
    expect(typeof html).toBe('string');
    expect(html).toContain('Keandalan Sistem &amp; Diagnostik');
  });

  it('renders tab buttons for doctor, reconcile, and backup', () => {
    const html = renderToString(<SystemReliabilityCard />);
    expect(html).toContain('Doctor (Diagnosis)');
    expect(html).toContain('Rekonsiliasi Drift');
    expect(html).toContain('Cadangan Panel');
  });
});
