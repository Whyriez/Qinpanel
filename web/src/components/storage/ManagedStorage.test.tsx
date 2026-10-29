import { renderToString } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import { BucketDialog, BucketOwnerStatus, ConfigureDialog, ManagedKeyDialog, ManagedStorage, ManagedStorageActions, StorageDnsStatus } from './ManagedStorage';
import type { ManagedStorageStatus, StorageConnection } from '../../api/storage';
import { canCancelJob } from '../../lib/jobProgress';
import type { JobItem } from '../../types/api';
vi.mock('../ui/ModalOverlay', () => ({ ModalOverlay: ({ children }: { children: React.ReactNode }) => <div>{children}</div> }));
const connection: StorageConnection = { id: 'fixture', owner_id: 2, site_id: null, label: 'Media', provider: 'compatible', endpoint: 'https://storage.example.test', region: 'us-east-1', addressing: 'path', bucket: 'alice-media', revision: 1, verified_at: null, has_credentials: true, created_at: 1, updated_at: 1, managed: true, managed_state: 'ready', application_key_generation: 0 };
describe('Managed storage', () => {
  const status: ManagedStorageStatus = { configured: true, running: true, endpoint: 'https://storage.example.test', engine_version: '4.50', data_directory: '/var/lib/qonpanel-storage', capacity_bytes: 100, available_bytes: 60, bucket_count: 0, quota_enforcement: 'none' };
  const actions = (props: Partial<React.ComponentProps<typeof ManagedStorageActions>> = {}) => renderToString(<ManagedStorageActions english={false} status={status} loading={false} disabled={false} modalOpen={false} onConfigure={vi.fn()} onBucket={vi.fn()} onManage={vi.fn()} onRefresh={vi.fn()} {...props} />);
  const bucketButton = (html: string) => html.match(/<button\b[^>]*>[\s\S]*?<\/button>/g)?.find(button => button.includes('Buat bucket Qinpanel')) ?? '';
  it('opens the first bucket flow for an active service without silently requiring an owner list', () => {
    expect(bucketButton(actions())).toContain('button-primary');
    expect(bucketButton(actions())).not.toContain('disabled');
    expect(actions()).toContain('Kelola alokasi bucket');
  });
  it('still blocks allocation while the service is unavailable, loading, locked or another dialog is open', () => {
    for (const props of [{ status: null }, { status: { ...status, running: false } }, { loading: true }, { disabled: true }, { modalOpen: true }]) {
      expect(bucketButton(actions(props))).toContain('disabled');
    }
  });
  it('explains the missing owner and provides tenant setup and refresh actions', () => {
    const html = renderToString(<BucketOwnerStatus english={false} context={{ owners: [], sites: [], upload_limit_bytes: 33554432 }} onRefreshContext={vi.fn()} />);
    expect(html).toContain('Siapkan tenant aktif sebagai pemilik bucket');
    expect(html).toContain('href="/tenants"'); expect(html).toContain('target="_blank"');
    expect(html).toContain('Muat ulang tenant');
  });
  it('distinguishes a failed or loading owner lookup from a missing tenant', () => {
    const context = { owners: [], sites: [], upload_limit_bytes: 33554432 };
    const loading = renderToString(<BucketOwnerStatus english context={context} contextLoading />);
    expect(loading).toContain('Loading bucket owners'); expect(loading).not.toContain('Set up an active tenant');
    const failed = renderToString(<BucketOwnerStatus english context={context} contextError="Context unavailable" onRefreshContext={vi.fn()} />);
    expect(failed).toContain('Existing buckets can still be managed'); expect(failed).toContain('Refresh tenants');
    expect(failed).not.toContain('Set up an active tenant');
  });
  it('opens bucket setup without an owner but prevents submitting an unowned allocation', () => {
    const html = renderToString(<BucketDialog english={false} context={{ owners: [], sites: [], upload_limit_bytes: 33554432 }} onClose={vi.fn()} onSaved={vi.fn()} />);
    expect(html).toContain('Nama bucket'); expect(html).toContain('Kelola tenant');
    expect(html).toMatch(/<button[^>]*type="submit"[^>]*disabled/);
  });
  it('only permits allocation to an available owner and a project they own', () => {
    const context = { owners: [{ id: 2, username: 'alice' }, { id: 3, username: 'bob' }], sites: [{ id: 8, owner_id: 3, domain: 'bob.test' }], upload_limit_bytes: 33554432 };
    const render = (props: { ownerId?: number; siteId?: number; contextLoading?: boolean; contextError?: string }) => renderToString(<BucketDialog english context={context} onClose={vi.fn()} onSaved={vi.fn()} {...props} />);
    expect(render({ siteId: 8 })).not.toMatch(/<button[^>]*type="submit"[^>]*disabled/);
    expect(render({ siteId: 8 })).toContain('<option value="3" selected="">bob</option>');
    for (const props of [{ ownerId: 1 }, { ownerId: 2, siteId: 8 }, { ownerId: 2, contextLoading: true }, { ownerId: 2, contextError: 'unavailable' }]) {
      expect(render(props)).toMatch(/<button[^>]*type="submit"[^>]*disabled/);
    }
    expect(render({ ownerId: 2, siteId: 8 })).toContain('project is unavailable');
  });
  it('permits cancelling a queued certificate request but protects an in-flight installation', () => {
    expect(canCancelJob({name:'storage.tls.issue',status:'queued'} as JobItem)).toBe(true);
    expect(canCancelJob({name:'storage.tls.issue',status:'running'} as JobItem)).toBe(false);
  });
  it('defaults to DNS-01 generation with consent and preserves the manual certificate option', () => {
    const html=renderToString(<ConfigureDialog english={false} status={null} onClose={vi.fn()} onSaved={vi.fn()} />);
    expect(html).toContain('Generate otomatis'); expect(html).toContain('Generate &amp; aktifkan');
    expect(html).toContain('DNS-01 Cloudflare'); expect(html).toContain('perpanjangan otomatis');
    expect(html).toContain('Gunakan sertifikat sendiri'); expect(html).not.toContain('<textarea');
    expect(html).toContain('Memeriksa token DNS-01'); expect(html).toContain('type="email"');
    expect(html).not.toContain('Hubungkan token DNS-01 (tab baru)');
  });
  it('explains why a connected Tunnel does not make DNS-01 ready and links to the certificate tab', () => {
    const html = renderToString(<StorageDnsStatus english={false} dns={{ available: true, configured: false, zone_name: null }} checking={false} failed={false} />);
    expect(html).toContain('Tunnel dan token DNS-01 dihubungkan secara terpisah');
    expect(html).toContain('Zone DNS Edit dan Zone Read');
    expect(html).toContain('network_tab=certificates');
    expect(html).toContain('target="_blank"');
    expect(html).toContain('isian domain dan email tetap ada');
  });
  it('does not label a loading or unavailable credential status as disconnected', () => {
    for (const props of [
      { checking: true, failed: false },
      { checking: false, failed: true },
      { checking: false, failed: false, dns: { available: false, configured: false, zone_name: null } },
    ]) {
      const html = renderToString(<StorageDnsStatus english={false} {...props} />);
      expect(html).not.toContain('Hubungkan token DNS-01 (tab baru)');
      expect(html).toContain('role="status"');
    }
  });
  it('shows the connected DNS-01 zone with legacy and current API responses', () => {
    for (const dns of [{ configured: true, zone_name: 'example.test' }, { available: true, configured: true, zone_name: 'example.test' }]) {
      const html = renderToString(<StorageDnsStatus english dns={dns} checking={false} failed={false} />);
      expect(html).toContain('DNS-01 token connected for zone:');
      expect(html).toContain('example.test');
      expect(html).not.toContain('Connect DNS-01 token (new tab)');
    }
  });
  it('distinguishes local service activation from connecting an external provider', () => {
    const html = renderToString(<ManagedStorage english={false} context={{ owners: [], sites: [], upload_limit_bytes: 33554432 }} onCreated={vi.fn()} />);
    expect(html).toContain('Aktifkan storage Qinpanel'); expect(html).toContain('Buat bucket Qinpanel');
    expect(html).toContain('server Anda sendiri'); expect(html).toContain('batas keras per bucket belum diterapkan'); expect(html).not.toContain('0 GiB');
  });
  it('requires typed bucket confirmation before key issuance and explains revocation', () => {
    const html = renderToString(<ManagedKeyDialog english connection={connection} onClose={vi.fn()} onIssued={vi.fn()} />);
    expect(html).toContain('Type the bucket name'); expect(html).toContain('Rotation revokes the previous application key'); expect(html).toContain('Issue new key'); expect(html).toContain('disabled'); expect(html).not.toContain('secret_key');
  });
  it('offers reconciliation when a key operation has an unknown result', () => {
    const html = renderToString(<ManagedKeyDialog english connection={{ ...connection, application_key_state: 'unknown' }} onClose={vi.fn()} onIssued={vi.fn()} />);
    expect(html).toContain('Reconcile key'); expect(html).not.toContain('Issue new key');
  });
});
