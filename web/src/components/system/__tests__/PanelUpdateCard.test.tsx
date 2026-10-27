import { describe, expect, it } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { PanelUpdateSummary } from '../PanelUpdateCard';
import { canUpdate, parseUpdateStatus, type UpdateStatus } from '../../../api/panelUpdate';
import '../../../i18n';
const now = 2_000_000_000_000;
const data: UpdateStatus = { current_version: 'v0.1.0', installed_digest: null, supported: true, checked_at: now / 1000, available: { version: 'v0.2.0', digest: 'a'.repeat(64), notes: '<script>untrusted()</script>' }, check_error: null, update: null };
describe('Panel update state and component', () => {
  it('permits a fresh stable new version and a rebuilt development package', () => { expect(canUpdate(data, now)).toBe(true); expect(canUpdate({ ...data, available: { ...data.available!, version: 'v0.1.0' } }, now)).toBe(true); });
  it('blocks old versions, stale/future checks, installed digests and missing updater', () => {
    expect(canUpdate({ ...data, current_version: 'v1.0.0' }, now)).toBe(false);
    expect(canUpdate({ ...data, checked_at: now / 1000 - 601 }, now)).toBe(false);
    expect(canUpdate({ ...data, checked_at: now / 1000 + 1 }, now)).toBe(false);
    expect(canUpdate({ ...data, installed_digest: 'a'.repeat(64) }, now)).toBe(false);
    expect(canUpdate({ ...data, supported: false }, now)).toBe(false);
    expect(canUpdate({ ...data, current_version: 'v0.2.0' }, now)).toBe(false);
  });
  it('escapes release notes and shows reconnecting without declaring a failed update', () => {
    const html = renderToStaticMarkup(<PanelUpdateSummary data={data} reconnecting />);
    expect(html).toContain('&lt;script&gt;'); expect(html).not.toContain('<script>'); expect(html).toContain('Menunggu konfirmasi server'); expect(html).not.toContain('Pembaruan gagal');
  });
  it('reports failed/interrupted runs without claiming a successful rollback', () => {
    const run = { id: 'fixture', version: 'v0.2.0', digest: 'a'.repeat(64), phase: 'failed' as const, started_at: 1, finished_at: 2 };
    const html = renderToStaticMarkup(<PanelUpdateSummary data={{ ...data, update: run }} />);
    expect(html).toContain('Installer mencoba memulihkan snapshot'); expect(html).not.toContain('berhasil dipulihkan');
    expect(canUpdate({ ...data, update: { ...run, phase: 'running' } }, now)).toBe(false);
  });
  it('rejects malformed response instead of offering a bogus update', () => {
    expect(parseUpdateStatus(data)).toEqual(data);
    for (const value of [null, {}, { ...data, available: { version: 'v0.2.0-rc.1', digest: 'a'.repeat(64), notes: '' } }, { ...data, supported: 'true' }, { ...data, checked_at: NaN }, { ...data, diagnostics: {} }]) expect(() => parseUpdateStatus(value)).toThrow();
  });
});
