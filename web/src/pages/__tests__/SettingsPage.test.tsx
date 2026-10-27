import { beforeEach, describe, expect, it, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { SettingsPage } from '../SettingsPage';
import '../../i18n';

const route = vi.hoisted(() => ({ url: '/settings' }));
vi.mock('../../lib/router', () => ({
  navigate: vi.fn(),
  useBrowserRoute: () => {
    const url = new URL(route.url, 'http://localhost');
    return { pathname: url.pathname, search: url.searchParams, url: url.pathname + url.search };
  },
}));

describe('SettingsPage navigation', () => {
  beforeEach(() => { route.url = '/settings'; });

  it('provides all ten sections in three labelled groups and names the mobile picker', () => {
    const html = renderToString(<SettingsPage />);
    expect(html).toContain('for="server-settings-section"');
    expect(html).toContain('Bagian pengaturan');
    expect(html.match(/<option /g)).toHaveLength(10);
    expect(html.match(/<optgroup /g)).toHaveLength(3);
    expect(html).toContain('label="Keamanan dan jaringan"');
    expect(html).toContain('label="Pemeliharaan"');
    expect(html).toContain('aria-controls="server-settings-content"');
  });

  it('reflects the existing section deep link in the picker and content', () => {
    route.url = '/settings?section=updates&retain=fixture';
    const html = renderToString(<SettingsPage />);
    expect(html).toContain('<option value="updates" selected="">');
    expect(html).toContain('aria-label="Pembaruan"');
    expect(html).toContain('Pembaruan panel');
  });

  it('uses the server optimizer for an unknown section', () => {
    route.url = '/settings?section=unknown';
    const html = renderToString(<SettingsPage />);
    expect(html).toContain('<option value="optimizer" selected="">');
    expect(html).toContain('data-testid="server-optimizer-card"');
  });
});
