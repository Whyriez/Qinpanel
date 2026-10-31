import { renderToStaticMarkup } from 'react-dom/server';
import { describe, expect, it, vi } from 'vitest';
import { Folder } from 'lucide-react';
import { FileBrowserListing, FileInspector, fileSize, fileType } from '../FileBrowserView';
import type { FsEntry } from '../../../types/files';
const entry: FsEntry = { name: 'photo + image.png', path: 'sites/project/photo + image.png', is_dir: false, size_bytes: 2048, modified_at: 1700000000, mode: 0o644, owner_user: 'p_alice', owner_group: 'panel-tenants' };
const props = { entries: [entry], selected: new Set([entry.path]), sortBy: 'name' as const, sortOrder: 'asc' as const, english: false, onSort: vi.fn(), onSelect: vi.fn(), onToggle: vi.fn(), onSelectAll: vi.fn(), onOpen: vi.fn(), onMenu: vi.fn(), icon: () => <Folder /> };
describe('File browser presentation', () => {
  it('uses real checkboxes, sortable column controls and explicit row actions', () => {
    const html = renderToStaticMarkup(<FileBrowserListing {...props} view="list" />);
    expect(html).toContain('aria-sort="ascending"');
    expect(html).toContain(`data-path="${entry.path}"`);
    expect(html).toContain('aria-label="Pilih photo + image.png"');
    expect(html).toContain('aria-label="Tindakan untuk photo + image.png"');
    expect(html).toContain('aria-selected="true"');
    expect(html).toContain('tabindex="0"');
    expect(html).toContain('2 KiB');
  });
  it('preserves grid selection and exact filenames with keyboard focus', () => {
    const html = renderToStaticMarkup(<FileBrowserListing {...props} view="grid" />);
    expect(html).toContain('role="listitem"');
    expect(html).toContain('data-selected="true"');
    expect(html).toContain('photo + image.png');
  });
  it('shows metadata and explicit actions for files without inventing folder sizes', () => {
    const controls = { english: false, icon: <Folder />, onClose: vi.fn(), onOpen: vi.fn(), onPreview: vi.fn(), onDownload: vi.fn(), onRename: vi.fn(), onPermissions: vi.fn() };
    const html = renderToStaticMarkup(<FileInspector {...controls} entry={entry} />);
    expect(html).toContain(entry.path); expect(html).toContain('644'); expect(html).toContain('p_alice');
    expect(html).toContain('Pratinjau'); expect(html).toContain('Unduh');
    const folder = renderToStaticMarkup(<FileInspector {...controls} entry={{ ...entry, is_dir: true }} />);
    expect(folder).toContain('Buka folder'); expect(folder).not.toContain('>Unduh'); expect(folder).not.toContain('2 KiB');
  });
  it('provides honest sizes, configuration types and English labels', () => {
    expect(fileSize(0)).toBe('0 B'); expect(fileSize(Number.NaN)).toBe('—');
    expect(fileType({ ...entry, name: '.env' })).toBe('Konfigurasi');
    expect(renderToStaticMarkup(<FileBrowserListing {...props} view="list" english />)).toContain('Select all files');
  });
});
