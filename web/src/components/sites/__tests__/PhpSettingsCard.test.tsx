import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { PhpSettingsCard } from '../PhpSettingsCard';
import { PhpStatusCard } from '../PhpStatusCard';

describe('PHP Settings and FPM Status Components', () => {
  it('renders PhpSettingsCard initial loading state', () => {
    const html = renderToString(
      <PhpSettingsCard siteId={1} phpVersion="8.3" />
    );
    expect(html).toContain('Memuat konfigurasi PHP...');
  });

  it('renders PhpStatusCard initial loading state', () => {
    const html = renderToString(
      <PhpStatusCard siteId={1} />
    );
    expect(html).toContain('Memuat status runtime PHP-FPM...');
  });
});
