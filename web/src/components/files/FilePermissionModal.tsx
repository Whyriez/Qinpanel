import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect, useId } from 'react';
import { X, Folder, File, AlertTriangle, Check, Lock } from 'lucide-react';

export interface FileItemInfo {
  name: string;
  path: string;
  isDir: boolean;
  modeOctal: string; // e.g. "0755" or "0644"
  owner?: string;
  group?: string;
}

export interface FilePermissionModalProps {
  isOpen: boolean;
  onClose: () => void;
  onSave: (payload: {
    path: string;
    isRecursive: boolean;
    mode?: string;
    fileMode?: string;
    dirMode?: string;
  }) => Promise<void> | void;
  file: FileItemInfo | null;
  isLoading?: boolean;
}

interface PermissionGridState {
  ownerRead: boolean;
  ownerWrite: boolean;
  ownerExecute: boolean;
  groupRead: boolean;
  groupWrite: boolean;
  groupExecute: boolean;
  othersRead: boolean;
  othersWrite: boolean;
  othersExecute: boolean;
}

function parseOctalToGrid(octalStr: string): PermissionGridState {
  const clean = octalStr.replace(/^0o|^0/, '');
  const val = parseInt(clean || '0', 8);

  const owner = (val >> 6) & 0o7;
  const group = (val >> 3) & 0o7;
  const others = val & 0o7;

  return {
    ownerRead: (owner & 4) !== 0,
    ownerWrite: (owner & 2) !== 0,
    ownerExecute: (owner & 1) !== 0,
    groupRead: (group & 4) !== 0,
    groupWrite: (group & 2) !== 0,
    groupExecute: (group & 1) !== 0,
    othersRead: (others & 4) !== 0,
    othersWrite: (others & 2) !== 0,
    othersExecute: (others & 1) !== 0,
  };
}

function gridToOctal(grid: PermissionGridState): string {
  let owner = 0;
  if (grid.ownerRead) owner += 4;
  if (grid.ownerWrite) owner += 2;
  if (grid.ownerExecute) owner += 1;

  let group = 0;
  if (grid.groupRead) group += 4;
  if (grid.groupWrite) group += 2;
  if (grid.groupExecute) group += 1;

  let others = 0;
  if (grid.othersRead) others += 4;
  if (grid.othersWrite) others += 2;
  if (grid.othersExecute) others += 1;

  return `0${owner}${group}${others}`;
}

export const FilePermissionModal: React.FC<FilePermissionModalProps> = ({
  isOpen,
  onClose,
  onSave,
  file,
  isLoading = false,
}) => {
  const modalId = useId();
  const initialMode = file?.modeOctal || (file?.isDir ? '0755' : '0644');
  const [octalInput, setOctalInput] = useState(initialMode);
  const [grid, setGrid] = useState<PermissionGridState>(() => parseOctalToGrid(initialMode));

  const [isRecursive, setIsRecursive] = useState(false);
  const [recursiveFileMode, setRecursiveFileMode] = useState('0644');
  const [recursiveDirMode, setRecursiveDirMode] = useState('0755');

  useEffect(() => {
    if (file) {
      const mode = file.modeOctal || (file.isDir ? '0755' : '0644');
      setOctalInput(mode);
      setGrid(parseOctalToGrid(mode));
      setIsRecursive(false);
      setRecursiveFileMode('0644');
      setRecursiveDirMode('0755');
    }
  }, [file]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isOpen && !isLoading) {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, isLoading, onClose]);

  if (!isOpen || !file) return null;

  const handleOctalChange = (val: string) => {
    const cleaned = val.replace(/[^0-7]/g, '').slice(0, 4);
    setOctalInput(cleaned);
    if (cleaned.length >= 3) {
      setGrid(parseOctalToGrid(cleaned));
    }
  };

  const handleGridToggle = (key: keyof PermissionGridState) => {
    const updated = { ...grid, [key]: !grid[key] };
    setGrid(updated);
    setOctalInput(gridToOctal(updated));
  };

  const applyPreset = (presetOctal: string) => {
    setOctalInput(presetOctal);
    setGrid(parseOctalToGrid(presetOctal));
  };

  const isPermissive777 =
    octalInput.endsWith('777') ||
    octalInput === '0777' ||
    octalInput === '777' ||
    grid.othersWrite;

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (isRecursive && file.isDir) {
      onSave({
        path: file.path,
        isRecursive: true,
        fileMode: recursiveFileMode,
        dirMode: recursiveDirMode,
      });
    } else {
      onSave({
        path: file.path,
        isRecursive: false,
        mode: octalInput.startsWith('0') ? octalInput : `0${octalInput}`,
      });
    }
  };

  return (
    <ModalOverlay
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs"
      role="dialog"
      aria-modal="true"
      aria-labelledby={`${modalId}-title`}
      onDismiss={onClose}
      dismissDisabled={isLoading}
    >
      <div className="w-full max-w-lg rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-6 shadow-[var(--shadow-floating)] animate-in fade-in zoom-in-95 duration-150 text-[var(--foreground)] dialog-panel">
        <div className="flex items-start justify-between pb-4 border-b border-[var(--line)] dialog-header">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-[var(--radius-md)] bg-blue-500/10 text-blue-500">
              {file.isDir ? <Folder className="h-5 w-5" /> : <File className="h-5 w-5" />}
            </div>
            <div>
              <h3 id={`${modalId}-title`} className="text-lg font-semibold text-[var(--foreground)]">
                Ubah Izin Akses
              </h3>
              <p className="text-xs text-[var(--muted-foreground)] truncate max-w-sm" title={file.path}>
                {file.name} ({file.path})
              </p>
            </div>
          </div>
          <button
            type="button"
            onClick={onClose}
            disabled={isLoading}
            className="rounded-[var(--radius-sm)] p-1 text-[var(--muted-foreground)] hover:bg-[var(--surface-hover)] hover:text-[var(--foreground)]"
            aria-label="Tutup modal"
          >
            <X className="h-5 w-5" />
          </button>
        </div>

        <form onSubmit={handleSubmit} className="mt-4 dialog-form">
          <div className="dialog-body space-y-4">
            {/* Owner & Group Info */}
            <div className="flex items-center justify-between text-xs px-3 py-2 rounded-[var(--radius-md)] bg-[var(--surface-hover)] border border-[var(--line)]">
              <div className="flex items-center gap-2">
                <Lock className="h-3.5 w-3.5 text-[var(--muted-foreground)]" />
                <span>
                  Pemilik: <strong className="text-[var(--foreground)]">{file.owner || 'p_tenant'}</strong>
                </span>
                <span>·</span>
                <span>
                  Grup: <strong className="text-[var(--foreground)]">{file.group || 'panel-tenants'}</strong>
                </span>
              </div>
              <span className="text-[10px] text-[var(--muted-foreground)]">Chown dibatasi</span>
            </div>

            {/* Octal Input & Presets */}
            <div className="flex flex-col sm:flex-row items-start sm:items-center justify-between gap-3 bg-[var(--surface)] p-3 rounded-[var(--radius-md)] border border-[var(--line)]">
              <div className="flex items-center gap-2">
                <label htmlFor={`${modalId}-octal`} className="text-sm font-medium text-[var(--foreground)]">
                  Mode Oktal:
                </label>
                <input
                  id={`${modalId}-octal`}
                  type="text"
                  maxLength={4}
                  value={octalInput}
                  onChange={(e) => handleOctalChange(e.target.value)}
                  className="w-20 px-2 py-1 text-center font-mono text-sm rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--background)] text-[var(--foreground)] focus:outline-none focus:ring-1 focus:ring-blue-500"
                />
              </div>
              <div className="flex items-center gap-1.5 flex-wrap">
                <span className="text-xs text-[var(--muted-foreground)]">Preset:</span>
                <button
                  type="button"
                  onClick={() => applyPreset('0644')}
                  className="px-2 py-1 text-xs rounded-[var(--radius-sm)] bg-[var(--surface-hover)] hover:bg-blue-500/10 hover:text-blue-500 border border-[var(--line)]"
                >
                  644 (Berkas)
                </button>
                <button
                  type="button"
                  onClick={() => applyPreset('0755')}
                  className="px-2 py-1 text-xs rounded-[var(--radius-sm)] bg-[var(--surface-hover)] hover:bg-blue-500/10 hover:text-blue-500 border border-[var(--line)]"
                >
                  755 (Folder)
                </button>
                <button
                  type="button"
                  onClick={() => applyPreset('0600')}
                  className="px-2 py-1 text-xs rounded-[var(--radius-sm)] bg-[var(--surface-hover)] hover:bg-blue-500/10 hover:text-blue-500 border border-[var(--line)]"
                >
                  600 (Privat)
                </button>
              </div>
            </div>

            {/* 3x3 Checkbox Grid */}
            <div className="border border-[var(--line)] rounded-[var(--radius-md)] overflow-hidden">
              <table className="w-full text-xs text-left">
                <thead className="bg-[var(--surface-hover)] border-b border-[var(--line)] text-[var(--muted-foreground)]">
                  <tr>
                    <th className="py-2 px-3 font-medium">Pengguna</th>
                    <th className="py-2 px-3 font-medium text-center">Baca (r / 4)</th>
                    <th className="py-2 px-3 font-medium text-center">Tulis (w / 2)</th>
                    <th className="py-2 px-3 font-medium text-center">Eksekusi (x / 1)</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-[var(--line)]">
                  {/* Pemilik / Owner */}
                  <tr>
                    <td className="py-2.5 px-3 font-medium">Pemilik (Owner)</td>
                    <td className="py-2.5 px-3 text-center">
                      <input
                        type="checkbox"
                        aria-label="Pemilik Baca"
                        checked={grid.ownerRead}
                        onChange={() => handleGridToggle('ownerRead')}
                        className="rounded accent-blue-600"
                      />
                    </td>
                    <td className="py-2.5 px-3 text-center">
                      <input
                        type="checkbox"
                        aria-label="Pemilik Tulis"
                        checked={grid.ownerWrite}
                        onChange={() => handleGridToggle('ownerWrite')}
                        className="rounded accent-blue-600"
                      />
                    </td>
                    <td className="py-2.5 px-3 text-center">
                      <input
                        type="checkbox"
                        aria-label="Pemilik Eksekusi"
                        checked={grid.ownerExecute}
                        onChange={() => handleGridToggle('ownerExecute')}
                        className="rounded accent-blue-600"
                      />
                    </td>
                  </tr>
                  {/* Grup / Group */}
                  <tr>
                    <td className="py-2.5 px-3 font-medium">Grup (Group)</td>
                    <td className="py-2.5 px-3 text-center">
                      <input
                        type="checkbox"
                        aria-label="Grup Baca"
                        checked={grid.groupRead}
                        onChange={() => handleGridToggle('groupRead')}
                        className="rounded accent-blue-600"
                      />
                    </td>
                    <td className="py-2.5 px-3 text-center">
                      <input
                        type="checkbox"
                        aria-label="Grup Tulis"
                        checked={grid.groupWrite}
                        onChange={() => handleGridToggle('groupWrite')}
                        className="rounded accent-blue-600"
                      />
                    </td>
                    <td className="py-2.5 px-3 text-center">
                      <input
                        type="checkbox"
                        aria-label="Grup Eksekusi"
                        checked={grid.groupExecute}
                        onChange={() => handleGridToggle('groupExecute')}
                        className="rounded accent-blue-600"
                      />
                    </td>
                  </tr>
                  {/* Lainnya / Others */}
                  <tr>
                    <td className="py-2.5 px-3 font-medium">Lainnya (Others)</td>
                    <td className="py-2.5 px-3 text-center">
                      <input
                        type="checkbox"
                        aria-label="Lainnya Baca"
                        checked={grid.othersRead}
                        onChange={() => handleGridToggle('othersRead')}
                        className="rounded accent-blue-600"
                      />
                    </td>
                    <td className="py-2.5 px-3 text-center">
                      <input
                        type="checkbox"
                        aria-label="Lainnya Tulis"
                        checked={grid.othersWrite}
                        onChange={() => handleGridToggle('othersWrite')}
                        className="rounded accent-blue-600"
                      />
                    </td>
                    <td className="py-2.5 px-3 text-center">
                      <input
                        type="checkbox"
                        aria-label="Lainnya Eksekusi"
                        checked={grid.othersExecute}
                        onChange={() => handleGridToggle('othersExecute')}
                        className="rounded accent-blue-600"
                      />
                    </td>
                  </tr>
                </tbody>
              </table>
            </div>

            {/* 777 Security Warning Banner */}
            {isPermissive777 && (
              <div
                data-testid="warning-777-banner"
                className="flex items-start gap-2.5 p-3 rounded-[var(--radius-md)] bg-amber-500/10 border border-amber-500/30 text-amber-500 text-xs"
              >
                <AlertTriangle className="h-4 w-4 shrink-0 mt-0.5" />
                <div>
                  <strong className="font-semibold">Peringatan Keamanan:</strong> Mode 0777 atau izin tulis publik
                  memungkinkan siapa saja di server memodifikasi berkas ini. Disarankan menggunakan preset aman
                  (0644 untuk berkas, 0755 untuk folder).
                </div>
              </div>
            )}

            {/* Recursive Option (Directores Only) */}
            {file.isDir && (
              <div className="pt-2 border-t border-[var(--line)] space-y-3">
                <label className="flex items-center gap-2 text-sm text-[var(--foreground)] cursor-pointer">
                  <input
                    type="checkbox"
                    checked={isRecursive}
                    onChange={(e) => setIsRecursive(e.target.checked)}
                    className="rounded accent-blue-600"
                  />
                  <span className="font-medium">Terapkan secara rekursif ke seluruh subfolder dan berkas</span>
                </label>

                {isRecursive && (
                  <div className="grid grid-cols-2 gap-3 p-3 bg-[var(--surface-hover)] rounded-[var(--radius-md)] border border-[var(--line)]">
                    <div>
                      <label className="block text-xs font-medium text-[var(--muted-foreground)] mb-1">
                        Mode Berkas (Files)
                      </label>
                      <input
                        type="text"
                        maxLength={4}
                        value={recursiveFileMode}
                        onChange={(e) => setRecursiveFileMode(e.target.value)}
                        className="w-full px-2 py-1 text-sm font-mono rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--background)] text-[var(--foreground)]"
                      />
                      <span className="text-[10px] text-[var(--muted-foreground)]">Direkomendasikan: 0644</span>
                    </div>
                    <div>
                      <label className="block text-xs font-medium text-[var(--muted-foreground)] mb-1">
                        Mode Folder (Directories)
                      </label>
                      <input
                        type="text"
                        maxLength={4}
                        value={recursiveDirMode}
                        onChange={(e) => setRecursiveDirMode(e.target.value)}
                        className="w-full px-2 py-1 text-sm font-mono rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--background)] text-[var(--foreground)]"
                      />
                      <span className="text-[10px] text-[var(--muted-foreground)]">Direkomendasikan: 0755</span>
                    </div>
                  </div>
                )}
              </div>
            )}

            {/* Action Buttons */}
          </div>
          <div className="flex items-center justify-end gap-3 pt-4 border-t border-[var(--line)] dialog-footer">
            <button
              type="button"
              onClick={onClose}
              disabled={isLoading}
              className="px-4 py-2 text-sm font-medium rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--foreground)] hover:bg-[var(--surface-hover)] transition-colors"
            >
              Batal
            </button>
            <button
              type="submit"
              disabled={isLoading}
              className="flex items-center gap-1.5 px-4 py-2 text-sm font-medium rounded-[var(--radius-md)] bg-blue-600 text-white hover:bg-blue-700 transition-colors shadow-sm disabled:opacity-50"
            >
              {isLoading ? (
                <span>Menyimpan...</span>
              ) : (
                <>
                  <Check className="h-4 w-4" />
                  <span>Simpan Perubahan</span>
                </>
              )}
            </button>
          </div>
        </form>
      </div>
    </ModalOverlay>
  );
};
