import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';
import { AlertCircle, ChevronDown, ChevronRight, FileText, Folder, FolderOpen, Loader2, PanelLeft, RefreshCw, X } from 'lucide-react';
import { createFileClient } from '../../api/files';
import type { FsEntry } from '../../types/files';
import { ModalOverlay } from '../ui/ModalOverlay';
import { FileEditorModal, type FileEditorModalProps } from './FileEditorModal';
import '../../styles/file-editor.css';

export interface EditorDocument {
  filePath: string;
  fileName: string;
  initialContent: string;
  sizeBytes: number;
  mtime?: number;
  etag?: string;
  readOnly?: boolean;
}

type DocumentState = { dirty: boolean; saving: boolean };
type FileClient = ReturnType<typeof createFileClient>;
const MAX_OPEN_FILES = 8;

// Convenience boundary for navigation; the API/tenant fs-worker remains authoritative.
export function isEditorPath(path: string, root: string) {
  const valid = (value: string) => !value.includes('\0') && !value.startsWith('/') &&
    (!value || value.split('/').every(part => part !== '' && part !== '.' && part !== '..'));
  return valid(path) && valid(root) && (!root || path === root || path.startsWith(`${root}/`));
}

function FolderBranch({ path, name, client, initialPath, activePath, rootPath, onOpen, showHidden }: {
  path: string; name: string; client: FileClient; initialPath: string; activePath: string;
  rootPath: string; onOpen: (entry: FsEntry) => void; showHidden: boolean;
}) {
  const [expanded, setExpanded] = useState(path === rootPath || initialPath.startsWith(`${path}/`));
  const [entries, setEntries] = useState<FsEntry[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [busy, setBusy] = useState(false);
  const [hasMore, setHasMore] = useState(false);
  const [nextOffset, setNextOffset] = useState(0);
  const [error, setError] = useState('');
  const pending = useRef(false);
  const live = useRef(true);
  useEffect(() => { live.current = true; return () => { live.current = false; }; }, []);
  const load = useCallback(async (offset = 0) => {
    if (pending.current) return;
    pending.current = true; setBusy(true); setError('');
    try {
      const data = await client.fetchFileList({ path, offset, limit: 100, show_hidden: showHidden, sort_by: 'name', sort_order: 'asc' });
      if (!live.current) return;
      const prefix = path ? `${path}/` : '';
      const safeEntries = data.entries.filter(entry => isEditorPath(entry.path, rootPath) && entry.path.startsWith(prefix) &&
        entry.path.length > prefix.length && !entry.path.slice(prefix.length).includes('/'));
      setEntries(previous => offset ? [...previous, ...safeEntries.filter(entry => !previous.some(old => old.path === entry.path))] : safeEntries);
      setNextOffset(offset + data.entries.length);
      setHasMore(data.has_more && data.entries.length > 0); setLoaded(true);
    } catch (cause) {
      if (live.current) setError(cause instanceof Error ? cause.message : 'Daftar berkas belum dapat dimuat.');
    } finally {
      pending.current = false;
      if (live.current) setBusy(false);
    }
  }, [client, path, rootPath, showHidden]);
  useEffect(() => { if (expanded && !loaded) void load(); }, [expanded, loaded, load]);
  const sorted = [...entries].sort((a, b) => Number(b.is_dir) - Number(a.is_dir) || a.name.localeCompare(b.name));
  return <li className="editor-folder">
    <button type="button" className="editor-explorer-row" aria-expanded={expanded} title={path || 'Ruang tenant'} onClick={() => setExpanded(value => !value)}>
      {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
      {expanded ? <FolderOpen size={15} /> : <Folder size={15} />}<span>{name}</span>
    </button>
    {expanded && <ul>
      {sorted.map(entry => entry.is_dir && !entry.is_symlink ? <FolderBranch key={entry.path} path={entry.path} name={entry.name} client={client} initialPath={initialPath} activePath={activePath} rootPath={rootPath} onOpen={onOpen} showHidden={showHidden} /> : <li key={entry.path}>
        <button type="button" className="editor-explorer-row editor-explorer-file" aria-current={activePath === entry.path ? 'true' : undefined} disabled={entry.is_symlink} title={entry.is_symlink ? 'Tautan simbolik tidak dibuka di editor' : entry.path} onClick={() => onOpen(entry)}>
          <FileText size={15} /><span>{entry.name}</span>{entry.is_symlink && <small>Tautan</small>}
        </button>
      </li>)}
      {busy && <li className="editor-explorer-notice" role="status"><Loader2 size={14} className="animate-spin" />Memuat berkas…</li>}
      {error && <li className="editor-explorer-notice" role="alert"><span>{error}</span><button type="button" onClick={() => void load(nextOffset)}>Coba lagi</button></li>}
      {!busy && !error && loaded && entries.length === 0 && <li className="editor-explorer-notice">Folder kosong</li>}
      {hasMore && !busy && !error && <li><button type="button" className="editor-explorer-more" onClick={() => void load(nextOffset)}>Muat lebih banyak</button></li>}
    </ul>}
  </li>;
}

function DocumentPane({ document, active, onClose, onStateChange, onSave }: {
  document: EditorDocument; active: boolean; onClose: (path: string) => void;
  onStateChange: (path: string, state: DocumentState) => void; onSave: FileEditorModalProps['onSave'];
}) {
  const reportState = useCallback((state: DocumentState) => onStateChange(document.filePath, state), [document.filePath, onStateChange]);
  return <FileEditorModal isOpen presentation="pane" isActive={active}
    filePath={document.filePath} fileName={document.fileName} initialContent={document.initialContent}
    initialMtime={document.mtime} initialEtag={document.etag} sizeBytes={document.sizeBytes} isReadOnly={document.readOnly}
    onStateChange={reportState} onSave={onSave} onClose={() => onClose(document.filePath)} />;
}

export function FileEditorWorkspace({ initialDocument, tenant, rootPath = '', onClose, onSaved }: {
  initialDocument: EditorDocument; tenant?: string; rootPath?: string; onClose: () => void; onSaved: () => void;
}) {
  const id = useId();
  const client = useMemo(() => createFileClient(tenant), [tenant]);
  const [documents, setDocuments] = useState<EditorDocument[]>([initialDocument]);
  const [activePath, setActivePath] = useState(initialDocument.filePath);
  const [states, setStates] = useState<Record<string, DocumentState>>({});
  const statesRef = useRef(states);
  const [compact, setCompact] = useState(() => typeof window !== 'undefined' && window.matchMedia('(max-width: 640px)').matches);
  const [sidebarOpen, setSidebarOpen] = useState(() => typeof window === 'undefined' || !window.matchMedia('(max-width: 640px)').matches);
  const [showHidden, setShowHidden] = useState(false);
  const [treeRevision, setTreeRevision] = useState(0);
  const [opening, setOpening] = useState('');
  const [error, setError] = useState('');
  const openRequest = useRef<AbortController | null>(null);
  const requestId = useRef(0);
  const container = useRef<HTMLDivElement>(null);
  const saving = Object.values(states).some(state => state.saving);
  const dirtyCount = Object.values(states).filter(state => state.dirty).length;

  useEffect(() => () => { requestId.current += 1; openRequest.current?.abort(); }, []);
  useEffect(() => {
    const media = window.matchMedia('(max-width: 640px)');
    const update = () => setCompact(media.matches);
    media.addEventListener('change', update);
    return () => media.removeEventListener('change', update);
  }, []);
  useEffect(() => {
    const beforeUnload = (event: BeforeUnloadEvent) => {
      if (!Object.values(statesRef.current).some(state => state.dirty || state.saving)) return;
      event.preventDefault(); event.returnValue = '';
    };
    window.addEventListener('beforeunload', beforeUnload);
    return () => window.removeEventListener('beforeunload', beforeUnload);
  }, []);
  useEffect(() => {
    const frame = requestAnimationFrame(() => {
      if (sidebarOpen && compact) {
        container.current?.querySelector<HTMLElement>('.editor-workspace__sidebar button')?.focus();
      } else if (document.activeElement?.getAttribute('role') !== 'tab') {
        container.current?.querySelector<HTMLElement>('.editor-workspace__document:not([hidden]) .cm-content')?.focus();
      }
    });
    return () => cancelAnimationFrame(frame);
  }, [activePath, sidebarOpen, compact]);
  const reportState = useCallback((path: string, state: DocumentState) => {
    statesRef.current = { ...statesRef.current, [path]: state };
    setStates(statesRef.current);
  }, []);

  const closeWorkspace = () => {
    if (Object.values(statesRef.current).some(state => state.saving)) return;
    const count = Object.values(statesRef.current).filter(state => state.dirty).length;
    if (count && !window.confirm(`${count} berkas memiliki perubahan yang belum disimpan. Tutup editor dan buang perubahan?`)) return;
    onClose();
  };
  const closeDocument = (path: string) => {
    const state = statesRef.current[path];
    if (state?.saving || (state?.dirty && !window.confirm(`Perubahan pada ${path} belum disimpan. Tutup berkas dan buang perubahan?`))) return;
    const remaining = documents.filter(document => document.filePath !== path);
    setDocuments(remaining);
    const next = { ...statesRef.current }; delete next[path]; statesRef.current = next; setStates(next);
    if (activePath === path) setActivePath(remaining.at(-1)?.filePath || '');
  };
  const selectDocument = (path: string) => {
    setActivePath(path);
    if (window.matchMedia('(max-width: 640px)').matches) setSidebarOpen(false);
  };
  const openFile = async (entry: FsEntry) => {
    if (!isEditorPath(entry.path, rootPath) || entry.is_symlink || entry.is_dir) return;
    const sequence = ++requestId.current;
    openRequest.current?.abort(); setError(''); setOpening('');
    if (documents.some(document => document.filePath === entry.path)) { selectDocument(entry.path); return; }
    if (documents.length >= MAX_OPEN_FILES) { setError(`Maksimal ${MAX_OPEN_FILES} berkas terbuka. Tutup salah satu tab untuk membuka berkas lain.`); return; }
    const controller = new AbortController(); openRequest.current = controller; setOpening(entry.path);
    try {
      const data = await client.request<{ content: string; size_bytes: number; modified_at?: number; etag?: string; read_only?: boolean; is_binary?: boolean }>(`/api/v1/files/content?path=${encodeURIComponent(entry.path)}`, { signal: controller.signal });
      if (sequence !== requestId.current) return;
      setDocuments(previous => previous.some(document => document.filePath === entry.path) ? previous : [...previous, { filePath: entry.path, fileName: entry.name, initialContent: data.content, sizeBytes: data.size_bytes, mtime: data.modified_at, etag: data.etag, readOnly: data.read_only || data.is_binary }]);
      selectDocument(entry.path);
    } catch (cause) {
      if (sequence === requestId.current && !controller.signal.aborted) setError(cause instanceof Error ? cause.message : 'Berkas belum dapat dibuka.');
    } finally { if (sequence === requestId.current) setOpening(''); }
  };
  const saveFile: FileEditorModalProps['onSave'] = async payload => {
    try {
      const saved = await client.request<{ etag: string; modified_at?: number }>('/api/v1/files/content', { method: 'PUT', body: JSON.stringify({ path: payload.path, content: payload.content, expected_mtime: payload.expectedMtime, expected_etag: payload.expectedEtag, force: payload.force }) });
      onSaved();
      return { etag: saved.etag, modifiedAt: saved.modified_at };
    } catch (cause) {
      if ((cause as Error & { status?: number }).status === 409) return { conflict: true, message: (cause as Error).message };
      throw cause;
    }
  };
  const panelId = (path: string) => `${id}-file-${encodeURIComponent(path)}`;
  return <ModalOverlay className="editor-workspace-overlay" aria-labelledby={`${id}-title`} onDismiss={closeWorkspace} dismissDisabled={saving}>
    <div className="dialog-panel editor-workspace" ref={container}>
      <header className="editor-workspace__header dialog-header">
        <button type="button" className="editor-workspace__icon" aria-label={sidebarOpen ? 'Sembunyikan daftar berkas' : 'Tampilkan daftar berkas'} aria-expanded={sidebarOpen} aria-controls={`${id}-explorer`} onClick={() => setSidebarOpen(value => !value)}><PanelLeft size={18} /></button>
        <div><h2 id={`${id}-title`}>Editor berkas</h2><p>{dirtyCount ? `${dirtyCount} berkas belum disimpan` : 'Pilih berkas, sunting, lalu simpan'}</p></div>
        <button type="button" className="editor-workspace__icon" aria-label="Tutup Editor" disabled={saving} onClick={closeWorkspace}><X size={19} /></button>
      </header>
      <div className="editor-workspace__body dialog-body">
        {sidebarOpen && <button type="button" className="editor-workspace__scrim" aria-label="Tutup daftar berkas" onClick={() => setSidebarOpen(false)} />}
        <aside id={`${id}-explorer`} className="editor-workspace__sidebar" hidden={!sidebarOpen} aria-label="Daftar berkas">
          <div className="editor-explorer-heading"><span>Berkas</span><button type="button" className="editor-workspace__icon" aria-label="Muat ulang daftar berkas" onClick={() => setTreeRevision(value => value + 1)}><RefreshCw size={15} /></button></div>
          <label className="editor-explorer-hidden"><input type="checkbox" checked={showHidden} onChange={event => setShowHidden(event.target.checked)} />Berkas tersembunyi</label>
          <nav className="editor-explorer-tree" aria-label="Folder dan berkas"><ul>
            <FolderBranch key={`${treeRevision}-${showHidden}`} path={rootPath} name={rootPath.split('/').pop() || 'Ruang tenant'} client={client} initialPath={initialDocument.filePath} activePath={activePath} rootPath={rootPath} onOpen={entry => void openFile(entry)} showHidden={showHidden} />
          </ul></nav>
        </aside>
        <div className="editor-workspace__main" inert={compact && sidebarOpen}>
          <div className="editor-workspace__tabs" role="tablist" aria-label="Berkas terbuka" onKeyDown={event => {
            if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key) || (event.target as HTMLElement).getAttribute('role') !== 'tab') return;
            event.preventDefault();
            const current = documents.findIndex(document => `${panelId(document.filePath)}-tab` === (event.target as HTMLElement).id);
            const index = event.key === 'Home' ? 0 : event.key === 'End' ? documents.length - 1 : (current + (event.key === 'ArrowRight' ? 1 : -1) + documents.length) % documents.length;
            const path = documents[index]?.filePath;
            if (path) { setActivePath(path); document.getElementById(`${panelId(path)}-tab`)?.focus(); }
          }}>
            {documents.map(document => <div key={document.filePath} className="editor-workspace__tab" data-active={activePath === document.filePath}>
              <button type="button" id={`${panelId(document.filePath)}-tab`} role="tab" aria-selected={activePath === document.filePath} aria-controls={panelId(document.filePath)} tabIndex={activePath === document.filePath ? 0 : -1} title={document.filePath} onClick={() => selectDocument(document.filePath)}>
                <FileText size={14} /><span>{document.fileName}</span>{states[document.filePath]?.saving ? <Loader2 size={12} className="animate-spin" aria-label="Menyimpan" /> : states[document.filePath]?.dirty && <span className="editor-workspace__dirty" aria-label="Belum disimpan">●</span>}
              </button>
              <button type="button" aria-label={`Tutup ${document.filePath}`} disabled={states[document.filePath]?.saving} onClick={() => closeDocument(document.filePath)}><X size={13} /></button>
            </div>)}
          </div>
          {opening && <div className="editor-workspace__notice" role="status"><Loader2 size={14} className="animate-spin" />Membuka {opening}…</div>}
          {error && <div className="editor-workspace__notice" role="alert"><AlertCircle size={15} /><span>{error}</span><button type="button" aria-label="Tutup pesan" onClick={() => setError('')}><X size={14} /></button></div>}
          {documents.map(document => <div key={document.filePath} id={panelId(document.filePath)} role="tabpanel" aria-labelledby={`${panelId(document.filePath)}-tab`} className="editor-workspace__document" hidden={activePath !== document.filePath}>
            <DocumentPane document={document} active={activePath === document.filePath} onClose={closeDocument} onStateChange={reportState} onSave={saveFile} />
          </div>)}
          {documents.length === 0 && <div className="editor-workspace__empty"><FileText size={28} /><p>Pilih berkas dari sidebar untuk mulai menyunting.</p><button type="button" onClick={() => setSidebarOpen(true)}>Buka daftar berkas</button></div>}
        </div>
      </div>
    </div>
  </ModalOverlay>;
}
