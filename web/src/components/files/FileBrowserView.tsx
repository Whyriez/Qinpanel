import type { MouseEvent, ReactElement } from 'react';
import { ArrowDown, ArrowUp, ChevronRight, Download, Edit3, Eye, Folder, MoreHorizontal, ShieldCheck, X } from 'lucide-react';
import type { FsEntry, SortDirection, SortField, ViewMode } from '../../types/files';

export function fileSize(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return '—';
  const units = ['B', 'KiB', 'MiB', 'GiB', 'TiB'];
  const index = Math.min(4, Math.floor(Math.log2(Math.max(1, bytes)) / 10));
  return `${(bytes / 1024 ** index).toLocaleString(undefined, { maximumFractionDigits: index ? 1 : 0 })} ${units[index]}`;
}
export const fileType = (entry: FsEntry, en = false) => entry.is_dir ? (en ? 'Folder' : 'Folder') : entry.is_symlink ? (en ? 'Symbolic link' : 'Tautan simbolis') : entry.name.startsWith('.') && !entry.name.slice(1).includes('.') ? (en ? 'Configuration' : 'Konfigurasi') : entry.name.includes('.') ? `${entry.name.split('.').at(-1)?.toUpperCase()} ${en ? 'file' : 'file'}` : (en ? 'File' : 'Berkas');
export const fileDate = (seconds: number, en = false) => seconds ? new Date(seconds * 1000).toLocaleString(en ? 'en-GB' : 'id-ID', { day: 'numeric', month: 'short', year: 'numeric', hour: '2-digit', minute: '2-digit' }) : '—';
const mode = (entry: FsEntry) => (entry.mode & 0o777).toString(8).padStart(3, '0');

export function FileBrowserListing({ entries, selected, view, sortBy, sortOrder, onSort, onSelect, onToggle, onSelectAll, onOpen, onMenu, icon, english }: {
  entries: FsEntry[]; selected: Set<string>; view: ViewMode; sortBy: SortField; sortOrder: SortDirection; english: boolean;
  onSort: (field: SortField) => void; onSelect: (entry: FsEntry, index: number, event: MouseEvent) => void; onToggle: (entry: FsEntry) => void;
  onSelectAll: (checked: boolean) => void; onOpen: (entry: FsEntry) => void; onMenu: (event: MouseEvent, entry: FsEntry) => void; icon: (entry: FsEntry) => ReactElement;
}) {
  const text = (id: string, en: string) => english ? en : id;
  const sort = (field: SortField, label: string) => <button type="button" onClick={() => onSort(field)}>{label}{sortBy === field && (sortOrder === 'asc' ? <ArrowUp size={12} /> : <ArrowDown size={12} />)}</button>;
  const checkbox = (entry: FsEntry) => <input type="checkbox" aria-label={`${text('Pilih', 'Select')} ${entry.name}`} checked={selected.has(entry.path)} onClick={event => event.stopPropagation()} onChange={() => onToggle(entry)} />;
  const menu = (entry: FsEntry) => <button type="button" className="fb-row-menu" aria-label={`${text('Tindakan untuk', 'Actions for')} ${entry.name}`} onDoubleClick={event => event.stopPropagation()} onClick={event => onMenu(event, entry)}><MoreHorizontal size={17} aria-hidden="true" /></button>;
  const shared = (entry: FsEntry, index: number) => ({
    'data-file-item': 'true', 'data-path': entry.path, 'data-selected': selected.has(entry.path), draggable: true, tabIndex: 0,
    onClick: (event: MouseEvent) => onSelect(entry, index, event), onDoubleClick: () => onOpen(entry), onContextMenu: (event: MouseEvent) => onMenu(event, entry),
    onKeyDown: (event: React.KeyboardEvent) => { if (event.target !== event.currentTarget) return; if (event.key === 'Enter') { event.preventDefault(); onOpen(entry); } else if (event.key === ' ') { event.preventDefault(); onToggle(entry); } },
    onDragStart: (event: React.DragEvent) => { event.dataTransfer.setData('application/panel-files', JSON.stringify(selected.has(entry.path) ? [...selected] : [entry.path])); },
  });
  return view === 'list' ? <div className="fb-table-scroll"><table className="fb-table" aria-label={text('Daftar berkas', 'File listing')}><thead><tr>
    <th className="fb-check-col"><input type="checkbox" aria-label={text('Pilih semua berkas', 'Select all files')} checked={entries.length > 0 && entries.every(entry => selected.has(entry.path))} onChange={event => onSelectAll(event.target.checked)} /></th>
    <th aria-sort={sortBy === 'name' ? sortOrder === 'asc' ? 'ascending' : 'descending' : 'none'}>{sort('name', text('Nama', 'Name'))}</th>
    <th className="fb-size-col" aria-sort={sortBy === 'size' ? sortOrder === 'asc' ? 'ascending' : 'descending' : 'none'}>{sort('size', text('Ukuran', 'Size'))}</th>
    <th className="fb-date-col" aria-sort={sortBy === 'modified' ? sortOrder === 'asc' ? 'ascending' : 'descending' : 'none'}>{sort('modified', text('Diubah', 'Modified'))}</th><th className="fb-mode-col">{text('Izin', 'Permissions')}</th><th className="fb-menu-col"><span className="sr-only">{text('Tindakan', 'Actions')}</span></th>
  </tr></thead><tbody>{entries.map((entry, index) => <tr key={entry.path} {...shared(entry, index)} aria-selected={selected.has(entry.path)}>
    <td className="fb-check-col">{checkbox(entry)}</td><td className="fb-name-col"><div className="fb-name"><span className={`fb-file-symbol ${entry.is_dir ? 'is-folder' : ''}`}>{icon(entry)}</span><span><strong title={entry.name}>{entry.name}</strong><small>{fileType(entry, english)}{entry.is_symlink && <span> · {text('Tautan', 'Link')}</span>}</small></span></div></td><td className="fb-size-col">{entry.is_dir ? '—' : fileSize(entry.size_bytes)}</td><td className="fb-date-col"><time>{fileDate(entry.modified_at, english)}</time></td><td className="fb-mode-col"><code>{mode(entry)}</code></td><td>{menu(entry)}</td>
  </tr>)}</tbody></table></div> : <div className="fb-grid" role="list" aria-label={text('Daftar berkas', 'File listing')}>{entries.map((entry, index) => <div key={entry.path} {...shared(entry, index)} className="fb-grid-item" role="listitem" aria-label={entry.name}>
    <div className="fb-grid-check">{checkbox(entry)}</div>{menu(entry)}<span className={`fb-grid-symbol ${entry.is_dir ? 'is-folder' : ''}`}>{icon(entry)}</span><strong title={entry.name}>{entry.name}</strong><span>{entry.is_dir ? text('Folder', 'Folder') : fileSize(entry.size_bytes)}</span>
  </div>)}</div>;
}

export function FileInspector({ entry, english, icon, onClose, onOpen, onPreview, onDownload, onRename, onPermissions }: {
  entry: FsEntry; english: boolean; icon: ReactElement; onClose: () => void; onOpen: () => void; onPreview: () => void; onDownload: () => void; onRename: () => void; onPermissions: () => void;
}) {
  const text = (id: string, en: string) => english ? en : id;
  return <section className="fb-inspector-content" aria-label={text('Detail berkas', 'File details')}><header><h3>{text('Detail berkas', 'File details')}</h3><button type="button" onClick={onClose} aria-label={text('Tutup detail berkas', 'Close file details')}><X size={17} /></button></header>
    <div className={`fb-inspector-symbol ${entry.is_dir ? 'is-folder' : ''}`}>{icon}</div><h4>{entry.name}</h4><p className="fb-inspector-type">{fileType(entry, english)}</p>
    <button type="button" className="fb-primary" onClick={onOpen}>{entry.is_dir ? <Folder size={15} /> : <Edit3 size={15} />}{entry.is_dir ? text('Buka folder', 'Open folder') : text('Buka berkas', 'Open file')}<ChevronRight size={14} /></button>
    <dl><div><dt>{text('Lokasi', 'Location')}</dt><dd><code>{entry.path}</code></dd></div><div><dt>{text('Ukuran', 'Size')}</dt><dd>{entry.is_dir ? '—' : fileSize(entry.size_bytes)}</dd></div><div><dt>{text('Diubah', 'Modified')}</dt><dd>{fileDate(entry.modified_at, english)}</dd></div><div><dt>{text('Pemilik', 'Owner')}</dt><dd>{entry.owner_user || '—'}</dd></div><div><dt>{text('Grup', 'Group')}</dt><dd>{entry.owner_group || '—'}</dd></div><div><dt>{text('Izin', 'Permissions')}</dt><dd><code>{mode(entry)}</code></dd></div>{entry.is_symlink && <div><dt>{text('Target tautan', 'Link target')}</dt><dd><code>{entry.symlink_target || '—'}</code></dd></div>}</dl>
    <div className="fb-inspector-actions">{!entry.is_dir && <><button type="button" onClick={onPreview}><Eye size={15} />{text('Pratinjau', 'Preview')}</button><button type="button" onClick={onDownload}><Download size={15} />{text('Unduh', 'Download')}</button></>}<button type="button" onClick={onRename}><Edit3 size={15} />{text('Ganti nama', 'Rename')}</button><button type="button" onClick={onPermissions}><ShieldCheck size={15} />{text('Atur izin', 'Edit permissions')}</button></div>
  </section>;
}
