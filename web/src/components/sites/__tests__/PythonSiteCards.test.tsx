import { describe, it, expect, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { PythonStatusCard } from '../PythonStatusCard';
import { PythonSettingsCard } from '../PythonSettingsCard';
import '../../../i18n';
vi.mock('../../../context/AuthContext', () => ({ useAuth: () => ({ user: { role: 'admin' } }) }));

describe('PythonStatusCard and PythonSettingsCard Components', () => {
  it('renders PythonStatusCard initial loading state and service title', () => {
    const html = renderToString(<PythonStatusCard siteId={1} />);
    expect(html).toContain('Status Layanan Python (uv + venv)');
    expect(html).toContain('Soket Unix');
  });

  it('renders PythonSettingsCard initial loading state', () => {
    const html = renderToString(<PythonSettingsCard siteId={1} />);
    expect(html).toContain('Memuat konfigurasi Python...');
  });
});
