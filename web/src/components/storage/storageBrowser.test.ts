import { beforeEach, describe, expect, it, vi } from 'vitest';
import { fetchStorageObjects, type StorageConnection } from '../../api/storage';
import { isFolderMarker, searchStorageFiles, storageFolderKey } from './storageBrowser';
vi.mock('../../api/storage', async original => ({ ...await original<typeof import('../../api/storage')>(), fetchStorageObjects: vi.fn() }));
const connection = { id: 'owner-bucket' } as StorageConnection;
const object = (key: string, size = 1) => ({ key, size, modified_at: '2026-10-30T00:00:00Z' });
const page = (objects = [] as ReturnType<typeof object>[], prefixes = [] as string[], next_token: string | null = null) => ({ listing: { objects, prefixes, next_token }, revision: 1, verified_at: 1 });
beforeEach(() => vi.mocked(fetchStorageObjects).mockReset());
describe('Storage folder and search behavior', () => {
  it('preserves spaces, plus and Unicode and rejects path segments or invalid names', () => {
    expect(storageFolderKey('photos/', 'Foto produk + 日本')).toBe('photos/Foto produk + 日本/');
    for (const name of ['', ' ', '..', '.', 'one/two', 'back\\slash', 'control\u0000', 'x'.repeat(1024)]) expect(storageFolderKey('', name)).toBeNull();
    expect(storageFolderKey('not-a-folder', 'child')).toBeNull();
    expect(isFolderMarker(object('photos/', 0))).toBe(true);
    expect(isFolderMarker(object('photos/', 1))).toBe(false);
  });
  it('searches every page in a folder with opaque continuation tokens', async () => {
    vi.mocked(fetchStorageObjects).mockResolvedValueOnce(page([object('root/unrelated.txt')], [], 'opaque+/%=='))
      .mockResolvedValueOnce(page([object('root/Report final.pdf')]));
    const signal = new AbortController().signal;
    const result = await searchStorageFiles(connection, 'root/', 'report', false, signal);
    expect(result.listing.objects.map(item => item.key)).toEqual(['root/Report final.pdf']);
    expect(fetchStorageObjects).toHaveBeenNthCalledWith(2, connection, 'root/', 'opaque+/%==', signal);
    expect(result.partial).toBe(false);
  });
  it('optionally visits subfolders, deduplicates markers and avoids cycles/out-of-scope prefixes', async () => {
    vi.mocked(fetchStorageObjects).mockResolvedValueOnce(page([object('root/', 0)], ['root/sub/', 'root/sub/', 'other/']))
      .mockResolvedValueOnce(page([object('root/sub/report.csv')], ['root/sub/']));
    const result = await searchStorageFiles(connection, 'root/', 'report', true, new AbortController().signal);
    expect(result.listing.objects.map(item => item.key)).toEqual(['root/sub/report.csv']);
    expect(fetchStorageObjects).toHaveBeenCalledTimes(2);
    expect(result.partial).toBe(false);
  });
  it('reports partial results when provider pagination loops', async () => {
    vi.mocked(fetchStorageObjects).mockResolvedValue(page([object('report.txt')], [], 'same-token'));
    const result = await searchStorageFiles(connection, '', 'report', false, new AbortController().signal);
    expect(result.listing.objects).toHaveLength(1);
    expect(result.partial).toBe(true);
    expect(fetchStorageObjects).toHaveBeenCalledTimes(2);
  });
  it('caps scans and matched results instead of claiming a complete bucket search', async () => {
    vi.mocked(fetchStorageObjects).mockResolvedValue(page(Array.from({ length: 300 }, (_, index) => object(`report-${index}.txt`))));
    const result = await searchStorageFiles(connection, '', 'report', false, new AbortController().signal);
    expect(result.listing.objects).toHaveLength(250);
    expect(result.partial).toBe(true);
  });
  it('aborts before reading a provider page', async () => {
    const controller = new AbortController(); controller.abort();
    await expect(searchStorageFiles(connection, '', 'report', true, controller.signal)).rejects.toMatchObject({ name: 'AbortError' });
    expect(fetchStorageObjects).not.toHaveBeenCalled();
  });
});
