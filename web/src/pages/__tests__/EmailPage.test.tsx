import { describe, it, expect, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { EmailPage } from '../EmailPage';
import '../../i18n';
const role = vi.hoisted(() => ({ value: 'admin' }));
vi.mock('../../context/AuthContext', () => ({ useAuth: () => ({ user: { role: role.value } }) }));
describe('email page access', () => {
  it('offers administrator SMTP and server settings', () => {
    role.value = 'admin'; const html = renderToString(<EmailPage />);
    expect(html).toContain('Notifikasi panel'); expect(html).toContain('Server email'); expect(html).toContain('Akun dan kotak masuk');
  });
  it('limits tenant navigation to their accounts and inbox', () => {
    role.value = 'user'; const html = renderToString(<EmailPage />);
    expect(html).toContain('Akun dan kotak masuk'); expect(html).not.toContain('>Notifikasi panel<'); expect(html).not.toContain('>Server email<');
  });
});
