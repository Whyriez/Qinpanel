import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { PanelUrlSettings, panelEndpointUrl, samePanelAddress } from '../PanelUrlSettings';
import type { Endpoint } from '../../../api/connectivity';
import '../../../i18n';
describe('Panel URL settings', () => {
  it('keeps activation unavailable before the policy has loaded and explains the transition', () => {
    const html = renderToStaticMarkup(<PanelUrlSettings endpoints={[]} />);
    expect(html).toContain('Memuat URL panel'); expect(html).toContain('tidak langsung mengaktifkan URL');
    expect(html).not.toContain('>Aktifkan URL<');
  });
  it('forms exact IPv6 and custom-port previews and treats default ports as one origin', () => {
    expect(panelEndpointUrl({ host: '2001:db8::7', port: 19443, tls_mode: 'tls' } as Endpoint)).toBe('https://[2001:db8::7]:19443');
    expect(samePanelAddress('https://panel.test:443', 'https://panel.test')).toBe(true);
    expect(samePanelAddress('https://panel.test:8443', 'https://panel.test')).toBe(false);
    expect(samePanelAddress('https://panel.test.evil.test', 'https://panel.test')).toBe(false);
  });
});
