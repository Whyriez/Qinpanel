import { renderToString } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import { AllocationEditor, RulesEditor, storageBytes } from './BucketPolicy';
import { ObjectDetail } from './ObjectExplorer';
import type { StorageConnection, StoragePolicy } from '../../api/storage';

const policy: StoragePolicy = { revision: 1, quota_bytes: 1073741824, used_bytes: null, object_count: null, measured_at: null, visibility: 'owner', enforcement: 'panel', rules: { read: true, write: true, delete: true, max_file_bytes: 33554432, extensions: [] } };
const connection = { id: 'fixture', bucket: 'media', managed: true } as StorageConnection;
describe('Bucket workspace', () => {
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
    expect(html).toContain('Akses publik tidak diaktifkan');
    expect(html.match(/checked=""/g)).toHaveLength(3);
    expect(html).toContain('max="32"'); expect(html).toContain('Ekstensi diizinkan');
    expect(html).not.toContain('Simpan alokasi');
  });
  it('shows object details and authenticated temporary links instead of claiming private files have public URLs', () => {
    const html = renderToString(<ObjectDetail connection={connection} object={{ key: 'photos/my photo.png', size: 2306867, modified_at: '2026-10-29T13:45:09Z' }} english={false} disabled={false} canDelete onClose={vi.fn()} onDelete={vi.fn()} />);
    expect(html).toContain('Detail file'); expect(html).toContain('photos/my photo.png');
    expect(html).toMatch(/2[.,]2 MiB/); expect(html).toContain('Buat tautan unduh');
    expect(html).toContain('memerlukan sesi tenant pemilik'); expect(html).toContain('Private');
    expect(html).not.toContain('X-Amz-Signature'); expect(html).not.toContain('src="http');
  });
  it('does not promise session-only access for an external presigned download', () => {
    const html = renderToString(<ObjectDetail connection={{ ...connection, managed: false }} object={{ key: 'document.svg', size: 100, modified_at: '2026-10-29T13:45:09Z' }} english disabled={false} canDelete={false} onClose={vi.fn()} onDelete={vi.fn()} />);
    expect(html).toContain('Anyone receiving it can download'); expect(html).toContain('Preview unavailable');
    expect(html).not.toContain('<img');
  });
});
