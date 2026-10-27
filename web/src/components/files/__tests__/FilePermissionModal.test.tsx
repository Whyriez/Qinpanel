import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { FilePermissionModal, FileItemInfo } from '../FilePermissionModal';

describe('FilePermissionModal Component', () => {
  const mockFile: FileItemInfo = {
    name: 'index.php',
    path: 'sites/example.com/index.php',
    isDir: false,
    modeOctal: '0644',
    owner: 'p_alice',
    group: 'panel-tenants',
  };

  const mockDir: FileItemInfo = {
    name: 'public',
    path: 'sites/example.com/public',
    isDir: true,
    modeOctal: '0755',
    owner: 'p_alice',
    group: 'panel-tenants',
  };

  it('renders modal when open with file details and 644 octal', () => {
    const html = renderToString(
      <FilePermissionModal
        isOpen={true}
        onClose={() => {}}
        onSave={() => {}}
        file={mockFile}
      />
    );

    expect(html).toContain('Ubah Izin Akses');
    expect(html).toContain('index.php');
    expect(html).toContain('0644');
    expect(html).toContain('p_alice');
    expect(html).toContain('panel-tenants');
    expect(html).toContain('Pemilik (Owner)');
    expect(html).toContain('Grup (Group)');
    expect(html).toContain('Lainnya (Others)');
    expect(html).not.toContain('Terapkan secara rekursif');
  });

  it('renders recursive option when target is a directory', () => {
    const html = renderToString(
      <FilePermissionModal
        isOpen={true}
        onClose={() => {}}
        onSave={() => {}}
        file={mockDir}
      />
    );

    expect(html).toContain('Ubah Izin Akses');
    expect(html).toContain('public');
    expect(html).toContain('0755');
    expect(html).toContain('Terapkan secara rekursif');
  });

  it('renders warning banner when mode is 777', () => {
    const file777: FileItemInfo = {
      ...mockFile,
      modeOctal: '0777',
    };

    const html = renderToString(
      <FilePermissionModal
        isOpen={true}
        onClose={() => {}}
        onSave={() => {}}
        file={file777}
      />
    );

    expect(html).toContain('Peringatan Keamanan:');
    expect(html).toContain('Mode 0777 atau izin tulis publik');
  });

  it('does not render when isOpen is false', () => {
    const html = renderToString(
      <FilePermissionModal
        isOpen={false}
        onClose={() => {}}
        onSave={() => {}}
        file={mockFile}
      />
    );

    expect(html).toBe('');
  });
});
