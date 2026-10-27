import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { NodeStatusCard } from '../NodeStatusCard';
import { NodeSettingsCard } from '../NodeSettingsCard';
import '../../../i18n';

describe('NodeStatusCard and NodeSettingsCard Components', () => {
  it('renders NodeStatusCard initial loading state', () => {
    const html = renderToString(<NodeStatusCard siteId={1} />);
    expect(html).toContain('Status Layanan Node.js');
  });

  it('renders NodeSettingsCard initial loading state', () => {
    const html = renderToString(<NodeSettingsCard siteId={1} />);
    expect(html).toContain('Memuat konfigurasi Node.js...');
  });
});
