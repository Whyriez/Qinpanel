import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { Dns01Settings, dns01State, parseDns01Inventory } from '../Dns01Settings';
import { canCancelJob } from '../../../lib/jobProgress';
import type { JobItem } from '../../../types/api';
const empty = { provider: { configured: false, pending: [] }, projects: [], orders: [], bindings: [] };
describe('DNS-01 origin settings', () => {
  it('explains zone scope, origin validation and separate panel certificates before any mutation', () => {
    const html = renderToStaticMarkup(<Dns01Settings />);
    for (const copy of ['Zone DNS Edit', 'Zone Read', 'verifikasi TLS origin tetap aktif', 'Global API Key', 'Sertifikat panel dan IP']) expect(html).toContain(copy);
    expect(html).toContain('Memuat DNS-01');
  });
  it('requires a coherent provider and bounded inventories', () => {
    expect(parseDns01Inventory(empty)).toEqual(empty);
    for (const bad of [{}, { ...empty, provider: { configured: true, pending: [] } }, { ...empty, orders: Array(21).fill({}) }, { ...empty, projects: [{ site_id: -1, hostname: 'a' }] }]) expect(() => parseDns01Inventory(bad)).toThrow();
  });
  it('rejects unknown states and invalid expiry before exposing actions', () => {
    const order = { id: 'fixture', site_id: 1, state: 'candidate', staging: false, error: null, job_id: 'job' };
    expect(parseDns01Inventory({ ...empty, orders: [order] }).orders).toHaveLength(1);
    expect(() => parseDns01Inventory({ ...empty, orders: [{ ...order, state: 'ready' }] })).toThrow();
    expect(() => parseDns01Inventory({ ...empty, bindings: [{ site_id: 1, certificate_id: 1, renew_at: 1, not_after: Number.MAX_SAFE_INTEGER, error: null }] })).toThrow();
  });
  it('distinguishes staging and uncertain candidates from installed TLS', () => {
    expect(dns01State('staging')).toContain('tidak dipasang');
    expect(dns01State('candidate')).toContain('dipulihkan');
    expect(dns01State('active')).toBe('Terpasang');
  });
  it('prevents running cancellation during TXT cleanup or origin activation', () => {
    expect(canCancelJob({ name: 'ssl.dns01.issue', status: 'running' } as JobItem)).toBe(false);
    expect(canCancelJob({ name: 'ssl.dns01.issue', status: 'queued' } as JobItem)).toBe(true);
  });
});
