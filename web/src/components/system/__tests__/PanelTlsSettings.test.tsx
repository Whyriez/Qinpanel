import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { PanelTlsSettings, certificateHours, panelTlsCandidates, parsePanelTlsInventory } from '../PanelTlsSettings';
import type { Endpoint } from '../../../api/connectivity';
import { canCancelJob } from '../../../lib/jobProgress';
import type { JobItem } from '../../../types/api';
import '../../../i18n';
describe('Panel IP certificate settings', () => {
  it('separates the panel from site certificates and explains port 80 without exposing premature actions', () => {
    const html = renderToStaticMarkup(<PanelTlsSettings endpoints={[]} />);
    expect(html).toContain('terpisah dari SSL situs'); expect(html).toContain('port 80'); expect(html).toContain('160 jam'); expect(html).toContain('Memuat sertifikat panel');
    expect(html).not.toContain('Antrekan penerbitan');
  });
  it('only offers registered direct IP endpoints with dedicated TLS ports', () => {
    const base = { service: 'panel', scope: { kind: 'host' }, enabled: true, transport: 'direct', tls_mode: 'tls', host: '1.1.1.1', port: 19443 } as Endpoint;
    const cases = [base, { ...base, host: '2606:4700:4700::1111' }, { ...base, host: 'panel.test' }, { ...base, port: 7700 }, { ...base, port: 17890 }, { ...base, enabled: false }, { ...base, transport: 'cloudflare_http' }, { ...base, service: 'ssh' }];
    expect(panelTlsCandidates(cases as Endpoint[])).toEqual(cases.slice(0, 2));
  });
  it('shows remaining hours without negative time and rejects malformed or site-scoped inventory', () => {
    expect(certificateHours(160 * 3600, 0)).toBe(160); expect(certificateHours(20, 21)).toBe(0);
    const cert = { id: 'fixture', scope: 'panel', ip: '1.1.1.1', port: 19443, staging: false, state: 'active', not_before: 0, not_after: 160 * 3600, renew_at: 100 * 3600, error: null, job_id: null };
    expect(parsePanelTlsInventory({ certificates: [cert], profile: 'shortlived', challenge_port: 80 }).certificates).toHaveLength(1);
    for (const row of [{ ...cert, scope: 'site' }, { ...cert, state: 'ready' }, { ...cert, not_after: Number.MAX_SAFE_INTEGER }]) expect(() => parsePanelTlsInventory({ certificates: [row] })).toThrow();
    expect(() => parsePanelTlsInventory({})).toThrow();
  });
  it('permits queued cancellation and blocks cancellation while TLS may be changing', () => {
    expect(canCancelJob({ name: 'panel.tls.issue', status: 'queued' } as JobItem)).toBe(true);
    expect(canCancelJob({ name: 'panel.tls.issue', status: 'running' } as JobItem)).toBe(false);
  });
});
