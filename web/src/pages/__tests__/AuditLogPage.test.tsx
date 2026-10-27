import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { AuditLogPage } from '../AuditLogPage';
import { AuthProvider } from '../../context/AuthContext';
import '../../i18n';

describe('AuditLogPage Component', () => {
  it('renders audit categories, labeled filters and an announced initial loading state', () => {
    const html = renderToString(
      <AuthProvider>
        <AuditLogPage />
      </AuthProvider>
    );

    expect(html).toContain('Catatan audit');
    expect(html).toContain('Autentikasi');
    expect(html).toContain('Website dan domain');
    expect(html).toContain('SSH dan terminal');
    expect(html).toContain('Lainnya');
    expect(html.match(/aria-pressed="true"/g)).toHaveLength(1);
    expect(html.match(/aria-pressed="false"/g)).toHaveLength(12);
    expect(html).toContain('for="audit-search"');
    expect(html).toContain('for="audit-status"');
    expect(html).toContain('for="audit-category-mobile"');
    expect(html).toContain('aria-busy="true"');
    expect(html).not.toContain('Belum ada catatan audit yang cocok');
  });
});
