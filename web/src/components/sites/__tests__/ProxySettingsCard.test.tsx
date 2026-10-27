import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { ProxySettingsCard } from '../ProxySettingsCard';
import '../../../i18n';

describe('ProxySettingsCard Component', () => {
  it('renders ProxySettingsCard initial loading state', () => {
    const html = renderToString(<ProxySettingsCard siteId={1} />);
    expect(html).toContain('Memuat konfigurasi reverse proxy...');
  });
});
