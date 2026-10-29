import { renderToString } from 'react-dom/server';
import { describe, expect, it } from 'vitest';
import { NetworkSettings } from '../NetworkSettings';

describe('Network settings destination', () => {
  it('opens DNS-01 credentials directly for the storage setup link', () => {
    const html = renderToString(<NetworkSettings initialTab="certificates" />);
    expect(html).toContain('Sertifikat hostname melalui DNS-01');
    expect(html).toContain('Zone DNS Edit dan Zone Read');
    expect(html).toContain('Storage Qinpanel');
    expect(html).not.toContain('Alamat publik server');
  });
  it('preserves the public IP tab as the default destination', () => {
    const html = renderToString(<NetworkSettings />);
    expect(html).toContain('Alamat publik server');
    expect(html).toContain('Memuat status Cloudflare');
    expect(html).not.toContain('Sertifikat hostname melalui DNS-01');
  });
});
