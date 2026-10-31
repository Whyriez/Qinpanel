import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect, useId, useCallback, useRef } from 'react';
import {
  Search,
  X,
  FileText,
  Folder,
  FileCode,
  AlertTriangle,
  Clock,
  Filter,
  File,
} from 'lucide-react';

export interface ContentMatchItem {
  lineNumber: number;
  lineText: string;
}

export interface SearchResultItem {
  path: string;
  isDir: boolean;
  sizeBytes: number;
  modifiedAt?: number;
  matches: ContentMatchItem[];
}

export interface SearchReportData {
  items: SearchResultItem[];
  totalScanned: number;
  limitReached: boolean;
  timedOut: boolean;
  durationMs: number;
}

export interface FileSearchModalProps {
  isOpen: boolean;
  onClose: () => void;
  currentPath?: string;
  onSelectFile?: (path: string, isDir: boolean) => void;
  onOpenFileEditor?: (path: string) => void;
  onOpenFilePreview?: (path: string) => void;
  initialQuery?: string;
  initialReport?: SearchReportData;
  onExecuteSearch?: (params: {
    path: string;
    query: string;
    searchContent: boolean;
    caseSensitive: boolean;
    extension?: string;
  }) => Promise<SearchReportData>;
}

export const FileSearchModal: React.FC<FileSearchModalProps> = ({
  isOpen,
  onClose,
  currentPath = '',
  initialQuery = '',
  initialReport,
  onSelectFile,
  onOpenFileEditor,
  onOpenFilePreview,
  onExecuteSearch,
}) => {
  const modalId = useId();
  const inputRef = useRef<HTMLInputElement>(null);

  const [query, setQuery] = useState(initialQuery);
  const [searchInPath, setSearchInPath] = useState(currentPath);
  const [searchContent, setSearchContent] = useState(false);
  const [caseSensitive, setCaseSensitive] = useState(false);
  const [extensionFilter, setExtensionFilter] = useState('');
  const [showFilters, setShowFilters] = useState(false);

  const [isSearching, setIsSearching] = useState(false);
  const [hasSearched, setHasSearched] = useState(Boolean(initialReport));
  const [report, setReport] = useState<SearchReportData | null>(initialReport || null);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  // Focus input and sync path when modal opens
  useEffect(() => {
    if (isOpen) {
      setSearchInPath(currentPath);
      setErrorMessage(null);
      if (initialQuery) setQuery(initialQuery);
      if (initialReport) {
        setReport(initialReport);
        setHasSearched(true);
      }
      setTimeout(() => inputRef.current?.focus(), 50);
    } else {
      setQuery('');
      setHasSearched(false);
      setReport(null);
      setErrorMessage(null);
    }
  }, [isOpen, currentPath, initialQuery, initialReport]);

  // Handle escape key
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (!isOpen) return;
      if (e.key === 'Escape') {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, onClose]);

  const handleSearchSubmit = useCallback(
    async (e?: React.FormEvent) => {
      if (e) e.preventDefault();
      const trimmedQuery = query.trim();
      if (!trimmedQuery) {
        setErrorMessage('Silakan masukkan kata kunci pencarian');
        return;
      }

      if (!onExecuteSearch) return;

      setIsSearching(true);
      setErrorMessage(null);
      try {
        const result = await onExecuteSearch({
          path: searchInPath.trim(),
          query: trimmedQuery,
          searchContent,
          caseSensitive,
          extension: extensionFilter.trim() || undefined,
        });
        setReport(result);
        setHasSearched(true);
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : String(err);
        setErrorMessage(`Gagal melakukan pencarian: ${msg}`);
      } finally {
        setIsSearching(false);
      }
    },
    [query, searchInPath, searchContent, caseSensitive, extensionFilter, onExecuteSearch]
  );

  if (!isOpen) return null;

  const formatSize = (bytes: number): string => {
    if (bytes < 1024) return `${bytes} B`;
    if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
    return `${(bytes / (1024 * 1024)).toFixed(2)} MB`;
  };

  const getFileName = (fullPath: string): string => {
    return fullPath.split('/').pop() || fullPath;
  };

  const isTextOrCode = (fileName: string): boolean => {
    const ext = fileName.split('.').pop()?.toLowerCase() || '';
    return [
      'txt',
      'log',
      'md',
      'json',
      'js',
      'ts',
      'tsx',
      'jsx',
      'html',
      'css',
      'php',
      'py',
      'rs',
      'sh',
      'yml',
      'yaml',
      'toml',
      'sql',
      'env',
    ].includes(ext);
  };

  return (
    <ModalOverlay
      className="fb-modal fixed inset-0 z-50 flex items-center justify-center p-2 sm:p-4 bg-black/75 backdrop-blur-xs"
      role="dialog"
      aria-modal="true"
      aria-labelledby={`${modalId}-title`}
      onDismiss={onClose}
    >
      <div className="flex flex-col w-full max-w-4xl h-[85vh] rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] shadow-[var(--shadow-floating)] animate-in fade-in zoom-in-95 duration-150 text-[var(--foreground)] overflow-hidden dialog-panel">
        {/* Header Bar */}
        <div className="flex items-center justify-between px-4 py-3 border-b border-[var(--line)] bg-[var(--surface)] dialog-header">
          <div className="flex items-center gap-2.5">
            <div className="flex h-8 w-8 items-center justify-center rounded-[var(--radius-md)] bg-blue-500/10 text-blue-500">
              <Search className="h-4 w-4" />
            </div>
            <div>
              <h3 id={`${modalId}-title`} className="text-sm font-semibold text-[var(--foreground)]">
                Pencarian Berkas & Konten
              </h3>
              <p className="text-[11px] text-[var(--muted-foreground)]">
                Cari berkas berdasarkan nama atau telusuri teks di dalam kode
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="p-1.5 rounded-[var(--radius-sm)] text-[var(--muted-foreground)] hover:bg-[var(--surface-hover)] hover:text-[var(--foreground)]"
            aria-label="Tutup Pencarian"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        {/* Search Input & Options Bar */}
        <form onSubmit={handleSearchSubmit} className="p-4 border-b border-[var(--line)] bg-[var(--surface)]/50">
          <div className="flex items-center gap-2">
            <div className="relative flex-1">
              <Search className="absolute left-3 top-1/2 -translate-y-1/2 h-4 w-4 text-[var(--muted-foreground)]" />
              <input
                ref={inputRef}
                type="text"
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Ketik nama berkas atau kata kunci yang dicari..."
                className="w-full pl-9 pr-3 py-2 text-sm rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--background)] text-[var(--foreground)] placeholder:text-[var(--muted-foreground)] focus:outline-none focus:ring-1 focus:ring-blue-500"
              />
            </div>
            <button
              type="button"
              onClick={() => setShowFilters(!showFilters)}
              className={`flex items-center gap-1.5 px-3 py-2 text-xs font-medium rounded-[var(--radius-md)] border transition-colors ${
                showFilters || searchContent || extensionFilter
                  ? 'border-blue-500/30 bg-blue-500/10 text-blue-500'
                  : 'border-[var(--line)] hover:bg-[var(--surface-hover)] text-[var(--muted-foreground)]'
              }`}
            >
              <Filter className="h-3.5 w-3.5" />
              <span>Filter</span>
            </button>
            <button
              type="submit"
              disabled={isSearching || !query.trim()}
              className="flex items-center gap-1.5 px-4 py-2 text-xs font-medium rounded-[var(--radius-md)] bg-blue-600 hover:bg-blue-700 text-white disabled:opacity-50 transition-colors shadow-sm"
            >
              <Search className="h-3.5 w-3.5" />
              <span>{isSearching ? 'Mencari...' : 'Cari'}</span>
            </button>
          </div>

          {/* Collapsible Filter Row */}
          {showFilters && (
            <div className="mt-3 pt-3 border-t border-[var(--line)] grid grid-cols-1 sm:grid-cols-3 gap-3 text-xs animate-in fade-in duration-100">
              <div>
                <label className="block text-[11px] font-medium text-[var(--muted-foreground)] mb-1">
                  Cari di dalam folder:
                </label>
                <input
                  type="text"
                  value={searchInPath}
                  onChange={(e) => setSearchInPath(e.target.value)}
                  placeholder="Contoh: public atau sites/app"
                  className="w-full px-2.5 py-1.5 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--background)] text-xs text-[var(--foreground)] focus:outline-none focus:ring-1 focus:ring-blue-500"
                />
              </div>
              <div>
                <label className="block text-[11px] font-medium text-[var(--muted-foreground)] mb-1">
                  Filter ekstensi:
                </label>
                <input
                  type="text"
                  value={extensionFilter}
                  onChange={(e) => setExtensionFilter(e.target.value)}
                  placeholder="Contoh: php, js, html"
                  className="w-full px-2.5 py-1.5 rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--background)] text-xs text-[var(--foreground)] focus:outline-none focus:ring-1 focus:ring-blue-500"
                />
              </div>
              <div className="flex flex-col justify-end gap-1.5">
                <label className="flex items-center gap-2 cursor-pointer select-none">
                  <input
                    type="checkbox"
                    checked={searchContent}
                    onChange={(e) => setSearchContent(e.target.checked)}
                    className="rounded border-[var(--line)] text-blue-600 focus:ring-blue-500"
                  />
                  <span className="font-medium text-[var(--foreground)]">Cari di dalam isi berkas</span>
                </label>
                <label className="flex items-center gap-2 cursor-pointer select-none">
                  <input
                    type="checkbox"
                    checked={caseSensitive}
                    onChange={(e) => setCaseSensitive(e.target.checked)}
                    className="rounded border-[var(--line)] text-blue-600 focus:ring-blue-500"
                  />
                  <span>Peka huruf besar/kecil (Case-sensitive)</span>
                </label>
              </div>
            </div>
          )}
        </form>

        {/* Error Banner */}
        {errorMessage && (
          <div className="px-4 py-2 bg-rose-500/10 border-b border-rose-500/20 text-rose-500 text-xs flex items-center gap-2">
            <AlertTriangle className="h-4 w-4 shrink-0" />
            <span>{errorMessage}</span>
          </div>
        )}

        {/* Report Status Banner (Limit reached or Timed out) */}
        {report && (report.limitReached || report.timedOut) && (
          <div className="px-4 py-2 bg-amber-500/10 border-b border-amber-500/20 text-amber-500 text-xs flex items-center justify-between">
            <div className="flex items-center gap-2">
              <Clock className="h-4 w-4 shrink-0" />
              <span>
                {report.limitReached
                  ? 'Batas maksimum hasil (100) tercapai. Hasil lain mungkin tidak ditampilkan; silakan persempit kriteria pencarian.'
                  : 'Batas waktu pencarian tercapai (15 detik). Sebagian berkas mungkin belum terpindai.'}
              </span>
            </div>
          </div>
        )}

        {/* Results Area */}
        <div className="flex-1 overflow-auto p-4 bg-[var(--background)] dialog-body">
          {isSearching && (
            <div className="flex flex-col items-center justify-center h-48 text-center text-xs text-[var(--muted-foreground)]">
              <div className="h-6 w-6 border-2 border-blue-500 border-t-transparent rounded-full animate-spin mb-3" />
              <p>Memindai direktori dan berkas...</p>
            </div>
          )}

          {!isSearching && !hasSearched && (
            <div className="flex flex-col items-center justify-center h-48 text-center text-xs text-[var(--muted-foreground)]">
              <Search className="h-10 w-10 text-[var(--muted-foreground)]/40 mb-3" />
              <p className="font-medium text-[var(--foreground)]">Masukkan kata kunci untuk memulai pencarian</p>
              <p className="mt-1">Gunakan opsi filter untuk mencari cuplikan baris kode di dalam berkas teks.</p>
            </div>
          )}

          {!isSearching && hasSearched && report && report.items.length === 0 && (
            <div className="flex flex-col items-center justify-center h-48 text-center text-xs text-[var(--muted-foreground)]">
              <File className="h-10 w-10 text-[var(--muted-foreground)]/40 mb-3" />
              <p className="font-medium text-[var(--foreground)]">Tidak ada berkas yang cocok</p>
              <p className="mt-1">Coba periksa ejaan atau ubah opsi filter pencarian Anda.</p>
            </div>
          )}

          {!isSearching && hasSearched && report && report.items.length > 0 && (
            <div className="space-y-2">
              {report.items.map((item, idx) => {
                const fileName = getFileName(item.path);
                const isText = isTextOrCode(fileName);

                return (
                  <div
                    key={`${item.path}-${idx}`}
                    className="p-3 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] hover:border-blue-500/40 transition-colors shadow-xs"
                  >
                    <div className="flex items-start justify-between gap-3">
                      <div className="flex items-start gap-2.5 min-w-0">
                        <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-[var(--radius-sm)] bg-[var(--surface-hover)] text-blue-500 mt-0.5">
                          {item.isDir ? (
                            <Folder className="h-4 w-4" />
                          ) : isText ? (
                            <FileCode className="h-4 w-4" />
                          ) : (
                            <FileText className="h-4 w-4" />
                          )}
                        </div>
                        <div className="min-w-0">
                          <button
                            type="button"
                            onClick={() => onSelectFile?.(item.path, item.isDir)}
                            className="text-xs font-semibold text-[var(--foreground)] hover:text-blue-500 truncate text-left block"
                          >
                            {fileName}
                          </button>
                          <p
                            className="text-[11px] text-[var(--muted-foreground)] font-mono truncate"
                            title={item.path}
                          >
                            {item.path}
                          </p>
                        </div>
                      </div>

                      <div className="flex items-center gap-2 shrink-0">
                        <span className="text-[11px] text-[var(--muted-foreground)] font-mono">
                          {formatSize(item.sizeBytes)}
                        </span>
                        {!item.isDir && isText && onOpenFileEditor && (
                          <button
                            type="button"
                            onClick={() => onOpenFileEditor(item.path)}
                            className="px-2 py-1 text-[11px] font-medium rounded-[var(--radius-sm)] bg-blue-600/10 text-blue-500 hover:bg-blue-600/20 border border-blue-500/20"
                          >
                            Edit
                          </button>
                        )}
                        {!item.isDir && onOpenFilePreview && (
                          <button
                            type="button"
                            onClick={() => onOpenFilePreview(item.path)}
                            className="px-2 py-1 text-[11px] font-medium rounded-[var(--radius-sm)] bg-[var(--surface-hover)] text-[var(--foreground)] hover:bg-[var(--line)]"
                          >
                            Lihat
                          </button>
                        )}
                      </div>
                    </div>

                    {/* Content Matches Snippets */}
                    {item.matches.length > 0 && (
                      <div className="mt-2.5 pt-2 border-t border-[var(--line)]/60 space-y-1">
                        {item.matches.map((m, mIdx) => (
                          <div
                            key={mIdx}
                            className="flex items-baseline gap-2 text-[11px] font-mono px-2 py-1 rounded bg-[var(--background)] text-[var(--foreground)] border border-[var(--line)]/40 overflow-x-auto"
                          >
                            <span className="text-[var(--muted-foreground)] shrink-0 select-none">
                              {`Baris ${m.lineNumber}:`}
                            </span>
                            <span className="truncate">{m.lineText}</span>
                          </div>
                        ))}
                      </div>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>

        {/* Footer Bar */}
        <div className="flex items-center justify-between px-4 py-2 border-t border-[var(--line)] bg-[var(--surface)] text-[11px] text-[var(--muted-foreground)] dialog-footer">
          <div>
            {report ? (
              <span>{`Ditemukan: ${report.items.length} hasil • Dipindai: ${report.totalScanned} entri`}</span>
            ) : (
              <span>Pencarian siap digunakan</span>
            )}
          </div>
          <div>{report && <span>{`Waktu: ${report.durationMs} ms`}</span>}</div>
        </div>
      </div>
    </ModalOverlay>
  );
};
