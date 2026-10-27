import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { TenantsPage } from '../TenantsPage';
import { AuthProvider } from '../../context/AuthContext';
import '../../i18n';

describe('TenantsPage Component', () => {
  it('renders tenant management header, buttons and search filter', () => {
    const html = renderToString(
      <AuthProvider>
        <TenantsPage />
      </AuthProvider>
    );

    expect(html).toContain('Manajemen Tenant');
    expect(html).toContain('Tambah Tenant');
    expect(html).toContain('Paket Hosting');
    expect(html).toContain('aria-label="Cari tenant berdasarkan slug atau email"');
    expect(html).toContain('role="group" aria-labelledby="tenant-status-label"');
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain('aria-busy="true"');
    expect(html).toContain('Memuat daftar tenant...');
  });
});
