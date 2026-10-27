import { describe, it, expect, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { FileManager } from '../FileManager';
import { FileManagerPage } from '../../../pages/FileManagerPage';

vi.mock('../../../context/AuthContext', () => ({ useAuth: () => ({user:{id:42,role:'user'},isLoading:false}) }));

// Mock client api calls to avoid actual fetch during test
vi.mock('../../../api/client', () => ({
  fetchFileList: vi.fn().mockResolvedValue({
    entries: [
      {
        name: 'public',
        path: 'public',
        is_dir: true,
        size_bytes: 4096,
        modified_at: 1700000000,
        mode: 0o755,
        owner_user: 'p_alice',
        owner_group: 'panel-tenants',
      },
      {
        name: 'index.php',
        path: 'index.php',
        is_dir: false,
        size_bytes: 405,
        modified_at: 1700000000,
        mode: 0o644,
        owner_user: 'p_alice',
        owner_group: 'panel-tenants',
      },
    ],
    total: 2,
    has_more: false,
    current_path: '',
  }),
  createFolder: vi.fn().mockResolvedValue({ success: true, message: 'Created' }),
  renameFile: vi.fn().mockResolvedValue({ success: true, message: 'Renamed' }),
  moveFile: vi.fn().mockResolvedValue({ success: true, message: 'Moved' }),
  copyFile: vi.fn().mockResolvedValue({ success: true, message: 'Copied' }),
  moveToTrash: vi.fn().mockResolvedValue({ success: true, message: 'Trashed' }),
  fetchTrashList: vi.fn().mockResolvedValue({ items: [], total_size_bytes: 0, count: 0 }),
  restoreTrash: vi.fn().mockResolvedValue({ success: true, message: 'Restored' }),
  emptyTrash: vi.fn().mockResolvedValue({ success: true, message: 'Emptied' }),
  deleteFilePermanent: vi.fn().mockResolvedValue({ success: true, message: 'Deleted' }),
  compressArchive: vi.fn().mockResolvedValue({ success: true, message: 'Compressed' }),
  extractArchive: vi.fn().mockResolvedValue({ success: true, message: 'Extracted' }),
  initUpload: vi.fn().mockResolvedValue({ upload_id: 'test-id', chunk_size: 1048576 }),
  uploadChunk: vi.fn().mockResolvedValue({ success: true, current_offset: 100, complete: true }),
}));

describe('FileManager Component', () => {
  it('renders toolbar buttons and breadcrumbs root', () => {
    const html = renderToString(
      <FileManager
        initialPath=""
        quotaInfo={{ usedBytes: 2 * 1024 * 1024 * 1024, limitBytes: 10 * 1024 * 1024 * 1024, percentage: 20 }}
      />
    );

    expect(html).toContain('root');
    expect(html).toContain('Unggah');
    expect(html).toContain('Folder Baru');
    expect(html).toContain('Berkas Baru');
    expect(html).toContain('Tempat Sampah');
    expect(html).toContain('Kuota:');
    expect(html).toContain('2 GB');
    expect(html).toContain('10 GB');
  });

  it('renders interactive "Terminal di sini" button in toolbar', () => {
    const html = renderToString(<FileManager />);

    expect(html).toContain('Terminal di sini');
    expect(html).toContain('Buka terminal di folder ini');
  });

  it('renders collapsible left folder tree and breadcrumbs for nested paths', () => {
    const html = renderToString(
      <FileManager initialPath="sites/example.com/public" />
    );

    expect(html).toContain('Pohon Folder');
    expect(html).toContain('sites');
    expect(html).toContain('example.com');
    expect(html).toContain('public');
  });

  it('renders quota percentage warning indicator when quota exceeds 90%', () => {
    const html = renderToString(
      <FileManager
        quotaInfo={{ usedBytes: 4.8 * 1024 * 1024 * 1024, limitBytes: 5 * 1024 * 1024 * 1024, percentage: 96 }}
      />
    );

    expect(html).toContain('bg-red-500');
    expect(html).toContain('4.8 GB');
    expect(html).toContain('5 GB');
  });

  it('renders standalone FileManagerPage with header and description', () => {
    const html = renderToString(<FileManagerPage initialPath="sites" />);

    expect(html).toContain('Pengelola Berkas');
    expect(html).toContain('Jelajahi, sunting, unggah, dan kelola seluruh berkas hosting dalam ruang terisolasi tenant');
    expect(html).toContain('root');
    expect(html).toContain('sites');
  });
});
