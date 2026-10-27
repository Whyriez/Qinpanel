import { describe, it, expect, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { NotificationBell } from '../NotificationBell';
import { NotificationSettingsModal } from '../NotificationSettingsModal';
import '../../../i18n';

describe('NotificationBell Component', () => {
  it('renders notification bell button without crash', () => {
    const html = renderToString(
      <NotificationBell userRole="admin" />
    );

    expect(html).toContain('aria-label="Pusat notifikasi"');
    expect(html).toContain('Pusat notifikasi');
  });

  it('renders settings modal when isOpen is true', () => {
    const onClose = vi.fn();
    const html = renderToString(
      <NotificationSettingsModal isOpen={true} onClose={onClose} />
    );

    expect(html).toContain('Pengaturan Notifikasi Sistem');
    expect(html).toContain('Koneksi SMTP');
    expect(html).toContain('Ambang Batas &amp; Peringatan');
  });

  it('renders nothing when modal is closed', () => {
    const onClose = vi.fn();
    const html = renderToString(
      <NotificationSettingsModal isOpen={false} onClose={onClose} />
    );

    expect(html).toBe('');
  });
});
