import { describe, expect, it } from 'vitest';
import { renderToString } from 'react-dom/server';
import { PythonRuntimesTab } from '../PythonRuntimesTab';
import { CatalogScheduleCard } from '../CatalogScheduleCard';
import { NodeInstallSettings, defaultNodeInstallOptions } from '../NodeInstallSettings';
import { isFullPythonVersion, PythonManualInstall } from '../PythonManualInstall';

describe('Runtime catalog controls', () => {
  it('shows Python installation surface and loading status', () => {
    const html = renderToString(<PythonRuntimesTab />);
    expect(html).toContain('Runtime Python');
    expect(html).toContain('Memuat runtime Python');
    expect(html).toContain('Muat ulang Python');
    expect(html).toContain('Perbarui katalog Python');
    expect(html).not.toContain('role="dialog"');
  });
  it('offers installation and manual version entry even before the catalog is populated', () => {
    const html = renderToString(<PythonManualInstall installedVersions={[]} busyVersions={[]} unavailableReason="" onInstall={async () => {}} />);
    expect(html).toContain('Pasang versi Python lainnya');
    expect(html).toContain('Nomor versi lengkap');
    expect(html).not.toContain('role="dialog"');
    expect(html).toMatch(/type="submit"[^>]*disabled=""/);
  });
  it('accepts exact CPython versions and rejects shorthand, commands, paths and overflow', () => {
    for (const version of ['3.13.9', '3.15.0']) expect(isFullPythonVersion(version)).toBe(true);
    for (const version of ['', '3.13', '3.15.0rc1', '../3.13.9', '3.13.9;id', '3.65536.0', '3.13.09', '4.0.0']) expect(isFullPythonVersion(version), version).toBe(false);
  });
  it('explains catalog-only scheduling and provides explicit refresh', () => {
    const html = renderToString(<CatalogScheduleCard onRefresh={() => {}} />);
    expect(html).toContain('Perbarui katalog sekarang');
    expect(html).toContain('Instalasi dan versi situs tetap dipilih manual');
  });
  it('keeps Node options inline without another version selection or installation dialog', () => {
    const html = renderToString(<NodeInstallSettings value={defaultNodeInstallOptions} disabled onChange={() => {}} />);
    expect(html).toContain('Pengaturan pemasangan Node.js');
    expect(html).toContain('Aktifkan Corepack');
    expect(html).not.toContain('role="dialog"');
    expect(html).not.toContain('<select');
    expect(html).toMatch(/type="checkbox"[^>]*disabled=""/);
  });
});
