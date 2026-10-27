import { describe, it, expect, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { TenantDetailPage } from '../TenantDetailPage';
import { AuthProvider } from '../../context/AuthContext';
import '../../i18n';

describe('TenantDetailPage Component', () => {
  it('renders tenant detail initial loading state properly', () => {
    const html = renderToString(
      <AuthProvider>
        <TenantDetailPage tenantId={1} onBack={vi.fn()} />
      </AuthProvider>
    );

    expect(html).toContain('Memuat data tenant...');
  });
});
