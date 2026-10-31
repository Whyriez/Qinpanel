import { renderToString } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import { AllocationEditor, BucketPolicy, RulesEditor, storageBytes } from './BucketPolicy';
import { ObjectDetail } from './ObjectExplorer';
import { AccessEditor, OwnerBucketWorkspace } from './BucketAccess';
import { StorageConnectionPanel } from './StorageConnectionPanel';
import type { StorageConnection, StoragePolicy } from '../../api/storage';

const policy: StoragePolicy = { revision: 1, quota_bytes: 1073741824, used_bytes: null, object_count: null, measured_at: null, visibility: 'owner', enforcement: 'panel', rules: { read: true, write: true, delete: true, max_file_bytes: 33554432, extensions: [] } };
const connection = { id: 'fixture', bucket: 'media', managed: true } as StorageConnection;
describe('Bucket workspace', () => {
  it('opens on files and keeps connection tools behind their own tab', () => {
    const html = renderToString(<OwnerBucketWorkspace connection={connection} english={false} onBusy={vi.fn()} connectionTools={<div>fixture connection configuration</div>} />);
    expect(html).toContain('aria-label="File browser"'); expect(html).toContain('<h2>media</h2>');
    expect(html).toContain('Koneksi S3'); expect(html).toContain('role="tabpanel"');
    expect(html).toContain('storage-file-skeleton'); expect(html).not.toContain('fixture connection configuration');
  });
  it('does not label every bucket private in the transfer rules section', () => {
    const html = renderToString(<BucketPolicy connection={connection} admin={false} english={false} locked={false} />);
    expect(html).toContain('Rules transfer panel');
    expect(html).toContain('Akses URL mengikuti pengaturan bucket');
    expect(html).not.toContain('Private');
  });
  it('distinguishes unmeasured totals from zero usage and formats disk allocations', () => {
    expect(storageBytes(null)).toBe('—'); expect(storageBytes(0)).toBe('0 B');
    expect(storageBytes(1073741824)).toBe('1 GiB');
  });
  it('gives operators allowance and measurement controls without file browsing', () => {
    const html = renderToString(<AllocationEditor policy={policy} english={false} disabled={false} onSave={vi.fn()} onMeasure={vi.fn()} />);
    expect(html).toContain('value="1"'); expect(html).toContain('Simpan alokasi'); expect(html).toContain('Ukur pemakaian');
    expect(html).toContain('nama dan isi file tetap tersembunyi'); expect(html).not.toContain('Upload');
  });
  it('gives owners explicit default read/write/delete rules and bounded file size', () => {
    const html = renderToString(<RulesEditor policy={policy} english={false} disabled={false} onSave={vi.fn()} />);
    expect(html).toContain('Akses baca publik mengikuti pilihan akses bucket');
    expect(html.match(/checked=""/g)).toHaveLength(3);
    expect(html).toContain('max="32"'); expect(html).toContain('Ekstensi diizinkan');
    expect(html).not.toContain('Simpan alokasi');
  });
  it('shows object details and authenticated temporary links instead of claiming private files have public URLs', () => {
    const html = renderToString(<ObjectDetail connection={connection} object={{ key: 'photos/my photo.png', size: 2306867, modified_at: '2026-10-29T13:45:09Z' }} english={false} disabled={false} canDelete onClose={vi.fn()} onDelete={vi.fn()} />);
    expect(html).toContain('Detail file'); expect(html).toContain('photos/my photo.png');
    expect(html).toMatch(/2[.,]2 MiB/); expect(html).toContain('Dapatkan URL'); expect(html).toContain('aria-label="Unduh file"');
    expect(html).toContain('tanpa login panel'); expect(html).toContain('Private');
    expect(html).not.toContain('X-Amz-Signature'); expect(html).not.toContain('src="http');
  });
  it('shows public URLs directly with copy and open icons, and keeps download as an icon', () => {
    const html = renderToString(<ObjectDetail connection={{ ...connection, endpoint: 'https://storage.example.test', addressing: 'path' }} access={{ revision: 2, visibility: 'public', desired_visibility: 'public', state: 'ready', request_id: null, can_change: true }} object={{ key: 'photos/my photo.png', size: 100, modified_at: '2026-10-30T00:00:00Z' }} english={false} disabled={false} canDelete onClose={vi.fn()} onDelete={vi.fn()} />);
    expect(html).toContain('https://storage.example.test/media/photos/my%20photo.png');
    expect(html).toContain('URL publik'); expect(html).toContain('aria-label="Salin URL"'); expect(html).toContain('aria-label="Buka URL"');
    expect(html).toContain('aria-label="Unduh file"'); expect(html).not.toContain('Buat tautan unduh'); expect(html).not.toContain('Dapatkan URL');
  });
  it('makes public read opt-in for the entire native bucket and exposes reconciliation for unknown changes', () => {
    const base = { revision: 1, visibility: 'private' as const, desired_visibility: 'private' as const, state: 'ready' as const, request_id: null, can_change: true };
    const render = (access: typeof base | { revision: number; visibility: 'private'; desired_visibility: 'private'; state: 'unknown'; request_id: string; can_change: boolean }) => renderToString(<AccessEditor connection={connection} access={access} english={false} onChanged={vi.fn()} onBusy={vi.fn()} />);
    expect(render(base)).toContain('Public'); expect(render(base)).toContain('semua file di bucket ini');
    expect(render(base)).toContain('Upload, hapus'); expect(render(base)).toMatch(/<button[^>]*disabled/);
    expect(render({ ...base, state: 'unknown', request_id: 'pending' })).toContain('Sinkronkan perubahan akses');
  });
  it('does not promise session-only access for an external presigned download', () => {
    const html = renderToString(<ObjectDetail connection={{ ...connection, managed: false }} object={{ key: 'document.svg', size: 100, modified_at: '2026-10-29T13:45:09Z' }} english disabled={false} canDelete={false} onClose={vi.fn()} onDelete={vi.fn()} />);
    expect(html).toContain('Anyone with the signed URL can open'); expect(html).toContain('Preview unavailable');
    expect(html).not.toContain('<img');
  });
  it('keeps rules visible with permission switches, presets and a disabled unchanged save', () => {
    const html = renderToString(<RulesEditor policy={policy} english={false} disabled={false} onSave={vi.fn()} />);
    expect(html).not.toContain('<details');
    expect(html.match(/role="switch"/g)).toHaveLength(3);
    expect(html).toContain('Preset ekstensi');
    expect(html).toContain('Pengaturan sudah tersimpan');
    expect(html).toContain('Batalkan perubahan');
  });
  it('groups S3 connection details, copy controls, application keys and environment without real secrets', () => {
    const html = renderToString(<StorageConnectionPanel connection={{ ...connection, endpoint: 'https://storage.example.test', provider: 'compatible', region: 'us-east-1', addressing: 'path' }} english={false} disabled={false} onTest={vi.fn()} onKeys={vi.fn()}><p>Project binding</p></StorageConnectionPanel>);
    expect(html).toContain('Salin Endpoint'); expect(html).toContain('Salin konfigurasi');
    expect(html).toContain('Kelola access key'); expect(html).toContain('Project binding');
    expect(html).toContain('AWS_SECRET_ACCESS_KEY=&lt;secret-access-key&gt;');
    expect(html).not.toContain('<details');
  });
});
