import { afterEach, describe, expect, it, vi } from 'vitest';
import { changePassword, getCsrfToken, setCsrfToken } from './client';

afterEach(() => { vi.unstubAllGlobals(); setCsrfToken(null); });

describe('Credential API contract', () => {
  it('sends the current_password field and adopts the rotated CSRF token', async () => {
    const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({
      user: { id: 1, username: 'renamed_admin', role: 'admin' }, csrf_token: 'rotated', requires_2fa: false,
    }), { status: 200, headers: { 'content-type': 'application/json' } }));
    vi.stubGlobal('fetch', fetch);
    setCsrfToken('current');
    const result = await changePassword('old-password', 'new-password', 'renamed_admin');
    const options = fetch.mock.calls[0][1];
    expect(JSON.parse(options.body)).toEqual({ current_password: 'old-password', new_password: 'new-password', username: 'renamed_admin' });
    expect(new Headers(options.headers).get('X-CSRF-Token')).toBe('current');
    expect(getCsrfToken()).toBe('rotated');
    expect(result.user?.username).toBe('renamed_admin');
  });

  it('omits the new password when only changing username and preserves errors', async () => {
    const fetch = vi.fn().mockResolvedValue(new Response(JSON.stringify({ error: 'Username sudah digunakan.' }), { status: 409 }));
    vi.stubGlobal('fetch', fetch);
    setCsrfToken('current');
    await expect(changePassword('old-password', undefined, 'taken')).rejects.toThrow('Username sudah digunakan.');
    expect(JSON.parse(fetch.mock.calls[0][1].body)).toEqual({ current_password: 'old-password', username: 'taken' });
    expect(getCsrfToken()).toBe('current');
  });
});
