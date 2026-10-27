import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { StatusPill } from '../StatusPill';

describe('StatusPill Component', () => {
  it('renders label and applies success variant class', () => {
    const html = renderToString(
      <StatusPill variant="success" label="Aktif" />
    );
    expect(html).toContain('Aktif');
    expect(html).toContain('var(--success)');
  });

  it('renders danger variant with error style', () => {
    const html = renderToString(
      <StatusPill variant="danger" label="Terputus" />
    );
    expect(html).toContain('Terputus');
    expect(html).toContain('var(--danger)');
  });

  it('renders pulse animation element when pulse is true', () => {
    const html = renderToString(
      <StatusPill variant="success" label="Running" pulse />
    );
    expect(html).toContain('animate-ping');
  });
});
