import { describe, expect, it, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { TenantTargetNotice } from '../TenantTarget';

describe('Tenant target feedback', () => {
  it('explains the prerequisite and gives an actual tenant management link', () => {
    const html = renderToString(<TenantTargetNotice state="empty" onRetry={vi.fn()} />);
    expect(html).toContain('Belum ada tenant yang dapat dikelola');
    expect(html).toContain('sesuai izin aksesnya');
    expect(html).toContain('href="/tenants"');
    expect(html).toContain('Kelola tenant');
    expect(html).not.toContain('Buat tenant terlebih dahulu.');
  });
  it('announces loading separately from a retryable failure', () => {
    const loading = renderToString(<TenantTargetNotice state="loading" onRetry={vi.fn()} />);
    expect(loading).toContain('role="status"');
    expect(loading).not.toContain('href="/tenants"');
    const error = renderToString(<TenantTargetNotice state="error" error="Layanan tidak tersedia" onRetry={vi.fn()} />);
    expect(error).toContain('role="alert"');
    expect(error).toContain('Layanan tidak tersedia');
    expect(error).toContain('Coba lagi');
    expect(error).not.toContain('Belum ada tenant yang dapat dikelola');
  });
  it('asks for an explicit selection without implying a missing tenant', () => {
    const html = renderToString(<TenantTargetNotice state="choose" onRetry={vi.fn()} />);
    expect(html).toContain('Pilih tenant untuk melanjutkan');
    expect(html).not.toContain('href="/tenants"');
  });
});
