import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { DatabaseServersTab } from '../DatabaseServersTab';
import '../../../i18n';

describe('DatabaseServersTab Component', () => {
  it('renders database server engine tab layout with notice and refresh button', () => {
    const html = renderToString(<DatabaseServersTab />);

    // Architecture notice
    expect(html).toContain('Arsitektur Mesin Basis Data Multi-Tenant');

    // Section title
    expect(html).toContain('Server Mesin Basis Data');
    expect(html).toContain('Kelola siklus hidup service database dan instalasi paket on-demand');

    // Refresh control
    expect(html).toContain('Muat Ulang');

    // Initial loading container
    expect(html).toContain('Memuat status service database...');
  });
});
