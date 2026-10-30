import { ModalOverlay } from './ModalOverlay';
import React, { useState, useEffect, useId } from 'react';
import { AlertTriangle, X } from 'lucide-react';

export interface ConfirmDangerDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onConfirm: (purgeFiles?: boolean) => void;
  title: string;
  description: string;
  errorMessage?: string;
  confirmMatchText: string;
  confirmButtonLabel?: string;
  hasPurgeOption?: boolean;
  purgeOptionLabel?: string;
  isLoading?: boolean;
}

export const ConfirmDangerDialog: React.FC<ConfirmDangerDialogProps> = ({
  isOpen,
  onClose,
  onConfirm,
  title,
  description,
  errorMessage,
  confirmMatchText,
  confirmButtonLabel = 'Konfirmasi Penghapusan',
  hasPurgeOption = false,
  purgeOptionLabel = 'Hapus seluruh direktori dan berkas milik tenant secara permanen',
  isLoading = false,
}) => {
  const confirmationId = useId();
  const [inputText, setInputText] = useState('');
  const [purgeFiles, setPurgeFiles] = useState(false);

  useEffect(() => {
    if (isOpen) {
      setInputText('');
      setPurgeFiles(false);
    }
  }, [isOpen]);

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === 'Escape' && isOpen && !isLoading) {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKeyDown);
    return () => window.removeEventListener('keydown', handleKeyDown);
  }, [isOpen, isLoading, onClose]);

  if (!isOpen) return null;

  const isConfirmed = inputText.trim() === confirmMatchText.trim();

  return (
    <ModalOverlay
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs"
      role="dialog"
      aria-modal="true"
      aria-labelledby="danger-dialog-title"
      onDismiss={onClose}
      dismissDisabled={isLoading}
    >
      <div className="w-full max-w-md [overflow-wrap:anywhere] rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-6 shadow-[var(--shadow-floating)] animate-in fade-in zoom-in-95 duration-150 dialog-panel">
        {/* Header */}
        <div className="flex items-start justify-between gap-4 mb-4 dialog-header">
          <div className="flex items-center gap-3">
            <div className="w-9 h-9 rounded-[var(--radius-md)] bg-[var(--danger)]/10 text-[var(--danger)] flex items-center justify-center shrink-0">
              <AlertTriangle className="w-5 h-5" />
            </div>
            <div>
              <h2 id="danger-dialog-title" className="text-sm font-bold text-[var(--ink)]">
                {title}
              </h2>
              <p className="text-xs text-[var(--ink-muted)] mt-0.5">Tindakan ini memerlukan verifikasi ganda.</p>
            </div>
          </div>
          <button
            onClick={onClose}
            disabled={isLoading}
            className="min-h-11 min-w-11 flex items-center justify-center rounded-[var(--radius-sm)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--raised)] cursor-pointer disabled:opacity-50"
            title="Tutup"
          >
            <X className="w-4 h-4" />
          </button>
        </div>
        <div className="dialog-body ">
          {/* Description */}
          <div className="text-xs text-[var(--ink-muted)] leading-relaxed mb-4 bg-[var(--raised)] p-3 rounded-[var(--radius-sm)] border border-[var(--line)]">
            {description}
          </div>
          {errorMessage && <p role="alert" className="text-xs text-[var(--danger)] leading-relaxed mb-4">{errorMessage}</p>}

          {/* Input Confirmation */}
          <div className="mb-4">
            <label htmlFor={confirmationId} className="block text-xs font-medium text-[var(--ink)] mb-1.5">
              Ketik <span className="font-mono font-bold text-[var(--danger)]">{confirmMatchText}</span> untuk
              mengonfirmasi:
            </label>
            <input
              id={confirmationId}
              type="text"
              value={inputText}
              onChange={(e) => setInputText(e.target.value)}
              disabled={isLoading}
              placeholder={confirmMatchText}
              autoFocus
              className="w-full min-h-11 px-3 py-2 text-base sm:text-xs font-mono rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] focus:outline-none focus:border-[var(--danger)]"
            />
          </div>

          {/* Optional purge checkbox */}
          {hasPurgeOption && (
            <div className="mb-5">
              <label className="flex items-start gap-2 cursor-pointer select-none">
                <input
                  type="checkbox"
                  checked={purgeFiles}
                  onChange={(e) => setPurgeFiles(e.target.checked)}
                  disabled={isLoading}
                  className="mt-0.5 rounded border-[var(--line)] text-[var(--danger)] focus:ring-[var(--danger)]"
                />
                <span className="text-xs text-[var(--ink-muted)] leading-normal">{purgeOptionLabel}</span>
              </label>
            </div>
          )}

          {/* Actions */}
        </div>
        <div className="flex items-center justify-end gap-2 pt-2 border-t border-[var(--line)] dialog-footer">
          <button
            type="button"
            onClick={onClose}
            disabled={isLoading}
            className="min-h-11 px-3.5 py-1.5 rounded-[var(--radius-sm)] border border-[var(--line)] text-xs font-medium text-[var(--ink)] hover:bg-[var(--raised)] cursor-pointer disabled:opacity-50"
          >
            Batal
          </button>
          <button
            type="button"
            onClick={() => onConfirm(purgeFiles)}
            disabled={!isConfirmed || isLoading}
            className={`min-h-11 px-3.5 py-1.5 rounded-[var(--radius-sm)] text-xs font-semibold text-white transition-opacity cursor-pointer ${
              isConfirmed && !isLoading
                ? 'bg-[var(--danger)] hover:opacity-90'
                : 'bg-[var(--danger)]/40 cursor-not-allowed opacity-60'
            }`}
          >
            {isLoading ? 'Memproses...' : confirmButtonLabel}
          </button>
        </div>
      </div>
    </ModalOverlay>
  );
};
