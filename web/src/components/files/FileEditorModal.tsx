import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect, useId, useCallback, useRef } from 'react';
import CodeMirror from '@uiw/react-codemirror';
import {
  X,
  Save,
  AlertTriangle,
  RotateCcw,
  FileText,
  Lock,
  CheckCircle2,
  AlertCircle,
} from 'lucide-react';

export interface FileEditorModalProps {
  isOpen: boolean;
  onClose: () => void;
  filePath: string;
  fileName: string;
  initialContent: string;
  initialMtime?: number;
  initialEtag?: string;
  sizeBytes: number;
  isReadOnly?: boolean;
  onSave: (payload: {
    path: string;
    content: string;
    expectedMtime?: number;
    expectedEtag?: string;
    force?: boolean;
  }) => Promise<
    | { conflict: true; message?: string }
    | { conflict?: false; etag: string; modifiedAt?: number }
  >;
  onReload?: () => Promise<void>;
  isLoading?: boolean;
  presentation?: 'modal' | 'pane';
  isActive?: boolean;
  onStateChange?: (state: { dirty: boolean; saving: boolean }) => void;
}

export const FileEditorModal: React.FC<FileEditorModalProps> = ({
  isOpen,
  onClose,
  filePath,
  fileName,
  initialContent,
  initialMtime,
  initialEtag,
  sizeBytes,
  isReadOnly = false,
  onSave,
  onReload,
  isLoading = false,
  presentation = 'modal',
  isActive = true,
  onStateChange,
}) => {
  const modalId = useId();
  const [content, setContent] = useState(initialContent);
  const [savedContent, setSavedContent] = useState(initialContent);
  const [revision, setRevision] = useState({ etag: initialEtag, mtime: initialMtime });
  const [revisionUnknown, setRevisionUnknown] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [conflictError, setConflictError] = useState<string | null>(null);
  const [hasConflict, setHasConflict] = useState(false);
  const [statusMessage, setStatusMessage] = useState<string | null>(null);
  const savePending = useRef(false);
  const session = useRef(0);
  const isDirty = content !== savedContent;

  const isLargeFile = sizeBytes > 5 * 1024 * 1024;
  const readOnlyMode = isReadOnly || isLargeFile;

  useEffect(() => {
    onStateChange?.({ dirty: isDirty, saving: isSaving });
  }, [isDirty, isSaving, onStateChange]);

  useEffect(() => {
    setContent(initialContent);
    setSavedContent(initialContent);
    setRevision({ etag: initialEtag, mtime: initialMtime });
    setRevisionUnknown(false);
    setIsSaving(false);
    savePending.current = false;
    setConflictError(null);
    setHasConflict(false);
    setStatusMessage(null);
    return () => { session.current += 1; };
  }, [initialContent, initialEtag, initialMtime, filePath, isOpen]);

  const handleClose = useCallback(() => {
    if (savePending.current) return;
    if (presentation === 'pane' || !isDirty || window.confirm('Ada perubahan yang belum disimpan. Yakin ingin menutup?')) {
      onClose();
    }
  }, [isDirty, onClose, presentation]);

  const handleSaveInternal = useCallback(
    async (force = false) => {
      if (!isOpen || readOnlyMode || isLoading || savePending.current || !isDirty || revisionUnknown) return;
      savePending.current = true;
      const saveSession = session.current;
      const submittedContent = content;
      setIsSaving(true);
      setConflictError(null);
      setHasConflict(false);
      setStatusMessage(null);

      try {
        const res = await onSave({
          path: filePath,
          content: submittedContent,
          expectedMtime: revision.mtime,
          expectedEtag: revision.etag,
          force,
        });

        if (saveSession !== session.current) return;
        if (res.conflict) {
          setHasConflict(true);
          setConflictError(
            res.message ||
              'Konflik terdeteksi: Berkas telah diubah di server oleh proses lain sejak Anda membukanya.'
          );
        } else {
          // Only acknowledge the submitted snapshot. Typing during the request stays dirty.
          setSavedContent(submittedContent);
          if (typeof res.etag !== 'string' || !res.etag.trim()) {
            setRevisionUnknown(true);
            setConflictError('Berkas tersimpan, tetapi versi terbaru tidak diterima. Tutup dan buka kembali berkas sebelum menyimpan lagi. Salin perubahan yang belum disimpan terlebih dahulu.');
            return;
          }
          setRevision({ etag: res.etag, mtime: res.modifiedAt });
          setStatusMessage('Berkas berhasil disimpan');
        }
      } catch (err: unknown) {
        if (saveSession !== session.current) return;
        const msg = err instanceof Error ? err.message : String(err);
        setConflictError(`Gagal menyimpan: ${msg}`);
      } finally {
        if (saveSession === session.current) {
          savePending.current = false;
          setIsSaving(false);
        }
      }
    },
    [content, filePath, revision, onSave, readOnlyMode, isOpen, isLoading, isDirty, revisionUnknown]
  );

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (!isOpen || !isActive) return;

      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 's') {
        e.preventDefault();
        void handleSaveInternal(false);
      }
    };

    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, isActive, handleSaveInternal]);

  if (!isOpen) return null;

  const handleContentChange = (val: string) => {
    setContent(val);
    setStatusMessage(null);
  };

  const lineCount = content.split('\n').length;
  const charCount = content.length;

  const panel = (
      <div className={presentation === 'pane' ? 'file-editor-pane' : 'flex flex-col w-full max-w-6xl h-[92vh] rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] shadow-[var(--shadow-floating)] animate-in fade-in zoom-in-95 duration-150 text-[var(--foreground)] overflow-hidden dialog-panel'}>
        {/* Header Bar */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-[var(--line)] bg-[var(--surface)] dialog-header">
          <div className="flex items-center gap-3 min-w-0">
            <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-[var(--radius-md)] bg-blue-500/10 text-blue-500">
              <FileText className="h-4 w-4" />
            </div>
            <div className="min-w-0">
              <div className="flex items-center gap-2">
                <h3 id={`${modalId}-title`} className="text-sm font-semibold truncate text-[var(--foreground)]">
                  {fileName}
                </h3>
                {isDirty && (
                  <span className="px-1.5 py-0.5 text-[10px] rounded bg-amber-500/10 text-amber-500 border border-amber-500/20 font-medium">
                    Belum disimpan
                  </span>
                )}
                {readOnlyMode && (
                  <span className="flex items-center gap-1 px-1.5 py-0.5 text-[10px] rounded bg-zinc-500/10 text-zinc-400 border border-zinc-500/20 font-medium">
                    <Lock className="h-3 w-3" />
                    Hanya Baca
                  </span>
                )}
              </div>
              <p className="text-[11px] text-[var(--muted-foreground)] truncate max-w-lg" title={filePath}>
                {filePath}
              </p>
            </div>
          </div>

          <div className="flex items-center gap-2 shrink-0">
            {statusMessage && (
              <span className="hidden sm:flex items-center gap-1 text-xs text-emerald-500">
                <CheckCircle2 className="h-3.5 w-3.5" />
                {statusMessage}
              </span>
            )}
            {!readOnlyMode && (
              <button
                type="button"
                onClick={() => void handleSaveInternal(false)}
                disabled={isSaving || isLoading || !isDirty || revisionUnknown}
                className="flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-[var(--radius-md)] bg-blue-600 text-white hover:bg-blue-700 transition-colors disabled:opacity-50 shadow-sm"
              >
                <Save className="h-3.5 w-3.5" />
                <span>{isSaving ? 'Menyimpan...' : 'Simpan'}</span>
                <kbd className="hidden sm:inline-block text-[10px] bg-blue-700/60 px-1 rounded">Ctrl+S</kbd>
              </button>
            )}
            <button
              type="button"
              onClick={handleClose}
              disabled={isSaving}
              className="p-1 rounded-[var(--radius-sm)] text-[var(--muted-foreground)] hover:bg-[var(--surface-hover)] hover:text-[var(--foreground)]"
              aria-label={presentation === 'pane' ? 'Tutup berkas' : 'Tutup Editor'}
            >
              <X className="h-5 w-5" />
            </button>
          </div>
        </div>

        {/* Warning Banner for Large File */}
        {isLargeFile && (
          <div className="flex items-center gap-2 px-4 py-2 bg-amber-500/10 border-b border-amber-500/20 text-amber-500 text-xs">
            <AlertTriangle className="h-4 w-4 shrink-0" />
            <span>
              Ukuran berkas melebihi batas 5 MB ({(sizeBytes / (1024 * 1024)).toFixed(2)} MB). Berkas dibuka dalam
              mode hanya-baca untuk kestabilan browser.
            </span>
          </div>
        )}

        {/* Conflict Detection Banner */}
        {conflictError && (
          <div
            data-testid="conflict-banner"
            className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-3 px-4 py-2.5 bg-rose-500/10 border-b border-rose-500/30 text-rose-500 text-xs animate-in fade-in duration-150"
          >
            <div className="flex items-center gap-2">
              <AlertCircle className="h-4 w-4 shrink-0" />
              <span>{conflictError}</span>
            </div>
            <div className="flex items-center gap-2 shrink-0">
              {onReload && (
                <button
                  type="button"
                  onClick={() => void onReload()}
                  className="flex items-center gap-1 px-2.5 py-1 rounded-[var(--radius-sm)] border border-rose-500/30 bg-rose-500/10 hover:bg-rose-500/20 text-xs font-medium"
                >
                  <RotateCcw className="h-3.5 w-3.5" />
                  Muat Ulang Berkas
                </button>
              )}
              {hasConflict && <button
                type="button"
                onClick={() => void handleSaveInternal(true)}
                disabled={isSaving || isLoading}
                className="px-2.5 py-1 rounded-[var(--radius-sm)] bg-rose-600 hover:bg-rose-700 text-white text-xs font-medium"
              >
                Simpan Paksa (Overwrite)
              </button>}
            </div>
          </div>
        )}

        {/* CodeMirror 6 Editor Area */}
        <div className="flex-1 overflow-auto relative bg-[var(--background)] p-0 dialog-body">
          {isLoading ? (
            <div className="flex items-center justify-center h-full text-xs text-[var(--muted-foreground)]">
              Memuat isi berkas...
            </div>
          ) : (
            <CodeMirror
              value={content}
              height="100%"
              theme="dark"
              readOnly={readOnlyMode}
              editable={!readOnlyMode}
              onChange={handleContentChange}
              basicSetup={{
                lineNumbers: true,
                highlightActiveLineGutter: true,
                highlightSpecialChars: true,
                history: true,
                foldGutter: true,
                drawSelection: true,
                dropCursor: true,
                allowMultipleSelections: true,
                indentOnInput: true,
                bracketMatching: true,
                closeBrackets: true,
                autocompletion: true,
                rectangularSelection: true,
                crosshairCursor: true,
                highlightActiveLine: true,
                highlightSelectionMatches: true,
                closeBracketsKeymap: true,
                defaultKeymap: true,
                searchKeymap: true,
                historyKeymap: true,
                foldKeymap: true,
                completionKeymap: true,
                lintKeymap: true,
              }}
              className="h-full text-sm font-mono"
            />
          )}
        </div>

        {/* Status Bar */}
        <div className="flex items-center justify-between px-4 py-1.5 border-t border-[var(--line)] bg-[var(--surface)] text-[11px] text-[var(--muted-foreground)] dialog-footer">
          <div className="flex items-center gap-3">
            <span>Baris: {lineCount}</span>
            <span>Karakter: {charCount}</span>
            <span>Ukuran: {(sizeBytes / 1024).toFixed(1)} KB</span>
          </div>
          <div className="flex items-center gap-3">
            <span>Encoding: UTF-8</span>
            <span>{`Mode: ${readOnlyMode ? 'Read-Only' : 'Edit'}`}</span>
          </div>
        </div>
      </div>
  );
  if (presentation === 'pane') return panel;
  return <ModalOverlay
    className="fixed inset-0 z-50 flex items-center justify-center p-2 sm:p-4 bg-black/70 backdrop-blur-xs"
    aria-labelledby={`${modalId}-title`}
    onDismiss={handleClose}
    dismissDisabled={isSaving}
  >{panel}</ModalOverlay>;
};
