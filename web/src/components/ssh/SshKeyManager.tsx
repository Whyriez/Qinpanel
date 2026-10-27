import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect, useCallback } from 'react';
import {
  Key,
  Plus,
  Trash2,
  Copy,
  Check,
  AlertCircle,
  CheckCircle2,
  Terminal,
  ShieldCheck,
  Calendar,
  X,
  Loader2,
} from 'lucide-react';
import { SshKeyItem, ValidateSshKeyResponse } from '../../types/api';
import {
  fetchSshKeys,
  createSshKey,
  deleteSshKey,
  validateSshKey,
} from '../../api/client';
import { StatusPill } from '../ui/StatusPill';
import { EmptyState } from '../ui/EmptyState';

export interface SshKeyManagerProps {
  userId?: number;
  tenantSlug?: string;
  readOnly?: boolean;
}

export const SshKeyManager: React.FC<SshKeyManagerProps> = ({
  userId,
  tenantSlug,
  readOnly = false,
}) => {
  const [keys, setKeys] = useState<SshKeyItem[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Add Key Modal State
  const [isAddModalOpen, setIsAddModalOpen] = useState(false);
  const [name, setName] = useState('');
  const [publicKey, setPublicKey] = useState('');
  const [validation, setValidation] = useState<ValidateSshKeyResponse | null>(null);
  const [isValidating, setIsValidating] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [modalError, setModalError] = useState<string | null>(null);

  // Delete State
  const [deletingKey, setDeletingKey] = useState<SshKeyItem | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  // Copy Feedback
  const [copiedFingerprint, setCopiedFingerprint] = useState<string | null>(null);
  const [copiedKeygenCmd, setCopiedKeygenCmd] = useState(false);

  const loadKeys = useCallback(async () => {
    setIsLoading(true);
    setError(null);
    try {
      const data = await fetchSshKeys({ user_id: userId, tenant: tenantSlug });
      setKeys(data);
    } catch (err: unknown) {
      const e = err as Error;
      setError(e.message || 'Gagal memuat daftar kunci SSH');
    } finally {
      setIsLoading(false);
    }
  }, [userId, tenantSlug]);

  useEffect(() => {
    loadKeys();
  }, [loadKeys]);

  // Live validation debounced
  useEffect(() => {
    const trimmed = publicKey.trim();
    if (!trimmed) {
      setValidation(null);
      setIsValidating(false);
      return;
    }

    const timer = setTimeout(async () => {
      setIsValidating(true);
      try {
        const res = await validateSshKey(trimmed);
        setValidation(res);
        // Auto-populate name if empty and comment is available
        if (res.valid && res.comment && !name.trim()) {
          setName(res.comment);
        }
      } catch {
        setValidation({
          valid: false,
          error: 'Gagal memvalidasi format kunci SSH',
        });
      } finally {
        setIsValidating(false);
      }
    }, 350);

    return () => clearTimeout(timer);
  }, [publicKey, name]);

  const handleCopyFingerprint = (fingerprint: string) => {
    navigator.clipboard.writeText(fingerprint);
    setCopiedFingerprint(fingerprint);
    setTimeout(() => setCopiedFingerprint(null), 2000);
  };

  const handleCopyKeygenCmd = () => {
    const cmd = 'ssh-keygen -t ed25519 -C "developer@example.com"';
    navigator.clipboard.writeText(cmd);
    setCopiedKeygenCmd(true);
    setTimeout(() => setCopiedKeygenCmd(false), 2000);
  };

  const handleOpenAddModal = () => {
    setName('');
    setPublicKey('');
    setValidation(null);
    setModalError(null);
    setIsAddModalOpen(true);
  };

  const handleCloseAddModal = () => {
    if (isSubmitting) return;
    setIsAddModalOpen(false);
  };

  const handleCreateKey = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) {
      setModalError('Nama label kunci wajib diisi');
      return;
    }
    if (!publicKey.trim()) {
      setModalError('Kunci publik SSH wajib diisi');
      return;
    }
    if (validation && !validation.valid) {
      setModalError(validation.error || 'Format kunci SSH tidak valid');
      return;
    }

    setIsSubmitting(true);
    setModalError(null);
    try {
      await createSshKey({
        name: name.trim(),
        public_key: publicKey.trim(),
      });
      setIsAddModalOpen(false);
      await loadKeys();
    } catch (err: unknown) {
      const e = err as Error;
      setModalError(e.message || 'Gagal menambahkan kunci SSH');
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleDeleteKey = async () => {
    if (!deletingKey) return;
    setIsDeleting(true);
    try {
      await deleteSshKey(deletingKey.id);
      setDeletingKey(null);
      await loadKeys();
    } catch (err: unknown) {
      const e = err as Error;
      setError(e.message || 'Gagal menghapus kunci SSH');
    } finally {
      setIsDeleting(false);
    }
  };

  const getKeyTypeVariant = (type: string): 'success' | 'info' | 'neutral' => {
    if (type.includes('ed25519')) return 'success';
    if (type.includes('rsa')) return 'info';
    return 'neutral';
  };

  const formatDate = (timestamp: number) => {
    try {
      const date = new Date(timestamp * 1000);
      return new Intl.DateTimeFormat('id-ID', {
        day: 'numeric',
        month: 'short',
        year: 'numeric',
      }).format(date);
    } catch {
      return 'Baru saja';
    }
  };

  return (
    <div className="space-y-6">
      {/* Header section */}
      <div className="ssh-key-header flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 p-5 rounded-[var(--radius-lg)] bg-[var(--surface)] border border-[var(--line)] shadow-sm">
        <div className="flex items-start gap-3.5">
          <div className="p-2.5 rounded-[var(--radius-md)] bg-[var(--accent)]/10 text-[var(--accent)] flex items-center justify-center">
            <Key className="w-5 h-5" />
          </div>
          <div>
            <h3 className="text-base font-semibold text-[var(--ink)] flex items-center gap-2">
              Kunci SSH Publik
              <span className="text-xs font-normal text-[var(--ink-muted)] px-2 py-0.5 rounded-full bg-[var(--surface-sunken)] border border-[var(--line)]">
                {keys.length} terdaftar
              </span>
            </h3>
            <p className="text-xs text-[var(--ink-muted)] mt-1 max-w-xl">
              Kelola kunci publik untuk masuk melalui SSH atau SFTP sesuai izin akses akun Anda.
            </p>
          </div>
        </div>

        {!readOnly && (
          <button
            type="button"
            onClick={handleOpenAddModal}
            className="inline-flex items-center justify-center gap-2 px-4 py-2 text-xs font-medium rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:bg-[var(--primary-hover)] transition-colors shadow-sm self-start sm:self-auto"
          >
            <Plus className="w-4 h-4" />
            <span>Tambah Kunci SSH</span>
          </button>
        )}
      </div>

      {/* Error state */}
      {error && (
        <div className="p-4 rounded-[var(--radius-md)] bg-[var(--danger)]/10 border border-[var(--danger)]/20 text-[var(--danger)] text-xs flex items-center gap-2.5">
          <AlertCircle className="w-4 h-4 shrink-0" />
          <span>{error}</span>
        </div>
      )}

      {/* Loading state */}
      {isLoading && (
        <div className="p-12 text-center rounded-[var(--radius-lg)] bg-[var(--surface)] border border-[var(--line)] flex flex-col items-center justify-center gap-3">
          <Loader2 className="w-6 h-6 animate-spin text-[var(--accent)]" />
          <p className="text-xs text-[var(--ink-muted)]">Memuat kunci SSH terdaftar...</p>
        </div>
      )}

      {/* Empty state */}
      {!isLoading && keys.length === 0 && (
        <EmptyState
          icon={Key}
          title="Belum Ada Kunci SSH"
          description="Tambahkan kunci publik Anda untuk autentikasi interaktif terminal atau login SFTP tanpa memasukkan kata sandi."
          actionLabel={!readOnly ? 'Tambah Kunci Pertama' : undefined}
          onAction={!readOnly ? handleOpenAddModal : undefined}
        />
      )}

      {/* Keys list */}
      {!isLoading && keys.length > 0 && (
        <div className="grid gap-3">
          {keys.map((key) => {
            const isCopied = copiedFingerprint === key.fingerprint_sha256;
            return (
              <div
                key={key.id}
                className="p-4 rounded-[var(--radius-lg)] bg-[var(--surface)] border border-[var(--line)] hover:border-[var(--line-strong)] transition-all flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4 shadow-sm"
              >
                <div className="space-y-2 min-w-0 flex-1">
                  <div className="flex items-center gap-2.5 flex-wrap">
                    <span className="text-sm font-semibold text-[var(--ink)] truncate">
                      {key.name}
                    </span>
                    <StatusPill
                      variant={getKeyTypeVariant(key.key_type)}
                      label={key.key_type.replace('ssh-', '').toUpperCase()}
                      size="sm"
                    />
                    <div className="inline-flex items-center gap-1 text-[11px] text-[var(--ink-muted)]">
                      <Calendar className="w-3 h-3" />
                      <span>{formatDate(key.created_at)}</span>
                    </div>
                  </div>

                  {/* Fingerprint block */}
                  <div className="flex items-center gap-2 flex-wrap">
                    <div className="inline-flex items-center gap-1.5 px-2.5 py-1 rounded bg-[var(--surface-sunken)] border border-[var(--line)] font-mono text-[11px] text-[var(--ink)] select-all max-w-full overflow-x-auto">
                      <ShieldCheck className="w-3.5 h-3.5 text-[var(--accent)] shrink-0" />
                      <span className="font-semibold text-[var(--accent)]">SHA256:</span>
                      <span>{key.fingerprint_sha256.replace('SHA256:', '')}</span>
                    </div>

                    <button
                      type="button"
                      onClick={() => handleCopyFingerprint(key.fingerprint_sha256)}
                      title="Salin fingerprint"
                      className="inline-flex items-center gap-1 px-2 py-1 text-[11px] rounded bg-[var(--surface)] hover:bg-[var(--surface-hover)] border border-[var(--line)] text-[var(--ink-muted)] hover:text-[var(--ink)] transition-colors"
                    >
                      {isCopied ? (
                        <>
                          <Check className="w-3 h-3 text-[var(--success)]" />
                          <span className="text-[var(--success)] font-medium">Disalin!</span>
                        </>
                      ) : (
                        <>
                          <Copy className="w-3 h-3" />
                          <span>Salin</span>
                        </>
                      )}
                    </button>
                  </div>
                </div>

                {!readOnly && (
                  <div className="flex items-center gap-2 self-end sm:self-center shrink-0">
                    <button
                      type="button"
                      onClick={() => setDeletingKey(key)}
                      className="p-2 rounded-[var(--radius-md)] text-[var(--ink-muted)] hover:text-[var(--danger)] hover:bg-[var(--danger)]/10 border border-transparent hover:border-[var(--danger)]/20 transition-colors"
                      title="Hapus kunci SSH"
                      aria-label={`Hapus kunci ${key.name}`}
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}

      {/* Helpful instruction banner */}
      <div className="p-4 rounded-[var(--radius-lg)] bg-[var(--surface-sunken)] border border-[var(--line)] text-xs space-y-2.5">
        <div className="flex items-center gap-2 font-medium text-[var(--ink)]">
          <Terminal className="w-4 h-4 text-[var(--accent)]" />
          <span>Cara membuat pasangan kunci SSH di komputer lokal Anda:</span>
        </div>
        <div className="flex items-center justify-between gap-3 p-2.5 rounded bg-[var(--surface)] border border-[var(--line)] font-mono text-[11px] text-[var(--ink)]">
          <span className="overflow-x-auto whitespace-nowrap">
            ssh-keygen -t ed25519 -C &quot;developer@example.com&quot;
          </span>
          <button
            type="button"
            onClick={handleCopyKeygenCmd}
            className="inline-flex items-center gap-1 px-2 py-1 rounded bg-[var(--surface-sunken)] hover:bg-[var(--surface-hover)] border border-[var(--line)] text-[var(--ink-muted)] hover:text-[var(--ink)] transition-colors shrink-0"
          >
            {copiedKeygenCmd ? (
              <>
                <Check className="w-3 h-3 text-[var(--success)]" />
                <span className="text-[var(--success)]">Disalin</span>
              </>
            ) : (
              <>
                <Copy className="w-3 h-3" />
                <span>Salin</span>
              </>
            )}
          </button>
        </div>
        <p className="text-[11px] text-[var(--ink-muted)]">
          Salin isi berkas publik <code className="px-1 py-0.5 rounded bg-[var(--surface)] border border-[var(--line)] text-[var(--ink)] font-mono">~/.ssh/id_ed25519.pub</code> dan tempelkan ke formulir di atas. Jangan pernah membagikan kunci privat Anda (<code className="font-mono">id_ed25519</code>).
        </p>
      </div>

      {/* Modal: Tambah Kunci SSH */}
      {isAddModalOpen && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm animate-in fade-in duration-150"
          onDismiss={handleCloseAddModal}
          dismissDisabled={isSubmitting}
          aria-labelledby="add-ssh-key-title"
        >
          <div className="relative w-full max-w-lg rounded-[var(--radius-xl)] bg-[var(--surface)] border border-[var(--line)] shadow-2xl p-6 space-y-5 animate-in zoom-in-95 duration-150 max-h-[90vh] overflow-y-auto dialog-panel">
            <div className="flex items-center justify-between border-b border-[var(--line)] pb-4 dialog-header">
              <div className="flex items-center gap-2.5">
                <div className="p-2 rounded-[var(--radius-md)] bg-[var(--accent)]/10 text-[var(--accent)]">
                  <Key className="w-4 h-4" />
                </div>
                <div>
                  <h3 id="add-ssh-key-title" className="text-base font-semibold text-[var(--ink)]">
                    Tambah Kunci SSH Publik
                  </h3>
                  <p className="text-xs text-[var(--ink-muted)]">
                    Daftarkan public key baru untuk akses terminal & SFTP
                  </p>
                </div>
              </div>
              <button
                type="button"
                onClick={handleCloseAddModal}
                disabled={isSubmitting}
                className="p-1 rounded-[var(--radius-md)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--surface-hover)] transition-colors"
                aria-label="Tutup"
              >
                <X className="w-4 h-4" />
              </button>
            </div>
            <form onSubmit={handleCreateKey} className="dialog-form">
              <div className="dialog-body space-y-4">
                {modalError && (
                  <div className="p-3.5 rounded-[var(--radius-md)] bg-[var(--danger)]/10 border border-[var(--danger)]/20 text-[var(--danger)] text-xs flex items-center gap-2">
                    <AlertCircle className="w-4 h-4 shrink-0" />
                    <span>{modalError}</span>
                  </div>
                )}

                {/* Nama Kunci */}
                <div className="space-y-1.5">
                  <label className="block text-xs font-medium text-[var(--ink)]">
                    Label / Nama Kunci <span className="text-[var(--danger)]">*</span>
                  </label>
                  <input
                    type="text"
                    value={name}
                    onChange={(e) => setName(e.target.value)}
                    placeholder="Contoh: Laptop Kantor, MacBook Pro, CI Runner"
                    required
                    maxLength={100}
                    disabled={isSubmitting}
                    className="w-full px-3 py-2 text-xs rounded-[var(--radius-md)] bg-[var(--surface-sunken)] border border-[var(--line)] text-[var(--ink)] placeholder-[var(--ink-muted)] focus:outline-none focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)] transition-all"
                  />
                </div>

                {/* Isi Public Key */}
                <div className="space-y-1.5">
                  <label className="block text-xs font-medium text-[var(--ink)]">
                    Kunci Publik (OpenSSH format) <span className="text-[var(--danger)]">*</span>
                  </label>
                  <textarea
                    value={publicKey}
                    onChange={(e) => setPublicKey(e.target.value)}
                    placeholder="ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI... user@example.com"
                    rows={4}
                    required
                    disabled={isSubmitting}
                    className="w-full px-3 py-2 text-xs font-mono rounded-[var(--radius-md)] bg-[var(--surface-sunken)] border border-[var(--line)] text-[var(--ink)] placeholder-[var(--ink-muted)] focus:outline-none focus:border-[var(--accent)] focus:ring-1 focus:ring-[var(--accent)] transition-all resize-none"
                  />
                </div>

                {/* Validation Status Indicator */}
                {isValidating && (
                  <div className="flex items-center gap-2 text-xs text-[var(--ink-muted)] py-1">
                    <Loader2 className="w-3.5 h-3.5 animate-spin text-[var(--accent)]" />
                    <span>Memvalidasi struktur kunci...</span>
                  </div>
                )}

                {validation && !isValidating && (
                  <div
                    className={`p-3 rounded-[var(--radius-md)] text-xs border flex items-start gap-2.5 ${
                      validation.valid
                        ? 'bg-[var(--success)]/10 border-[var(--success)]/20 text-[var(--success)]'
                        : 'bg-[var(--danger)]/10 border-[var(--danger)]/20 text-[var(--danger)]'
                    }`}
                  >
                    {validation.valid ? (
                      <>
                        <CheckCircle2 className="w-4 h-4 shrink-0 mt-0.5" />
                        <div className="space-y-1">
                          <p className="font-semibold">
                            Format valid: {validation.key_type} ({validation.bit_length}-bit)
                          </p>
                          {validation.fingerprint_sha256 && (
                            <p className="font-mono text-[11px] opacity-90 break-all">
                              {validation.fingerprint_sha256}
                            </p>
                          )}
                          {validation.comment && (
                            <p className="text-[11px] opacity-80">Komentar: {validation.comment}</p>
                          )}
                        </div>
                      </>
                    ) : (
                      <>
                        <AlertCircle className="w-4 h-4 shrink-0 mt-0.5" />
                        <div>
                          <p className="font-semibold">Format tidak valid</p>
                          <p className="text-[11px] opacity-90 mt-0.5">
                            {validation.error || 'Pastikan baris diawali dengan ssh-ed25519 atau ssh-rsa.'}
                          </p>
                        </div>
                      </>
                    )}
                  </div>
                )}

                {/* Root-owned protection note */}
                <div className="p-3 rounded-[var(--radius-md)] bg-[var(--surface-sunken)] border border-[var(--line)] text-[11px] text-[var(--ink-muted)] flex items-start gap-2">
                  <ShieldCheck className="w-4 h-4 text-[var(--accent)] shrink-0 mt-0.5" />
                  <span>
                    Sistem Qinpanel menyimpan authorized_keys di lokasi root-owned dengan proteksi ketat agar kunci
                    tidak dapat dimanipulasi dari dalam direktori tenant.
                  </span>
                </div>

                {/* Modal Actions */}
              </div>
              <div className="flex items-center justify-end gap-2.5 pt-3 border-t border-[var(--line)] dialog-footer">
                <button
                  type="button"
                  onClick={handleCloseAddModal}
                  disabled={isSubmitting}
                  className="px-4 py-2 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] text-[var(--ink)] hover:bg-[var(--surface-hover)] transition-colors"
                >
                  Batal
                </button>
                <button
                  type="submit"
                  disabled={isSubmitting || (validation !== null && !validation.valid)}
                  className="inline-flex items-center justify-center gap-1.5 px-4 py-2 text-xs font-medium rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:bg-[var(--primary-hover)] transition-colors disabled:opacity-50 disabled:cursor-not-allowed shadow-sm"
                >
                  {isSubmitting && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                  <span>{isSubmitting ? 'Menyimpan...' : 'Simpan Kunci SSH'}</span>
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* Modal: Konfirmasi Hapus Kunci */}
      {deletingKey && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm animate-in fade-in duration-150"
          onDismiss={() => setDeletingKey(null)}
          dismissDisabled={isDeleting}
          aria-labelledby="delete-ssh-key-title"
        >
          <div className="relative w-full max-w-md rounded-[var(--radius-xl)] bg-[var(--surface)] border border-[var(--line)] shadow-2xl p-6 space-y-4 animate-in zoom-in-95 duration-150 dialog-panel">
            <div className="flex items-center gap-3 dialog-header">
              <div className="p-2.5 rounded-full bg-[var(--danger)]/10 text-[var(--danger)]">
                <Trash2 className="w-5 h-5" />
              </div>
              <div>
                <h3 id="delete-ssh-key-title" className="text-base font-semibold text-[var(--ink)]">
                  Hapus Kunci SSH?
                </h3>
                <p className="text-xs text-[var(--ink-muted)]">Tindakan ini akan mencabut izin autentikasi</p>
              </div>
            </div>
            <div className="dialog-body space-y-4">
              <p className="text-xs text-[var(--ink)] leading-relaxed">
                Apakah Anda yakin ingin menghapus kunci{' '}
                <span className="font-semibold text-[var(--danger)]">{deletingKey.name}</span>? Kunci ini akan segera
                dihapus dari daftar authorized_keys di server.
              </p>

              <div className="p-2.5 rounded bg-[var(--surface-sunken)] border border-[var(--line)] font-mono text-[11px] text-[var(--ink-muted)] break-all select-all">
                {deletingKey.fingerprint_sha256}
              </div>
            </div>
            <div className="flex items-center justify-end gap-2.5 pt-3 border-t border-[var(--line)] dialog-footer">
              <button
                type="button"
                onClick={() => setDeletingKey(null)}
                disabled={isDeleting}
                className="px-4 py-2 text-xs font-medium rounded-[var(--radius-md)] border border-[var(--line)] text-[var(--ink)] hover:bg-[var(--surface-hover)] transition-colors"
              >
                Batal
              </button>
              <button
                type="button"
                onClick={handleDeleteKey}
                disabled={isDeleting}
                className="inline-flex items-center justify-center gap-1.5 px-4 py-2 text-xs font-medium rounded-[var(--radius-md)] bg-[var(--danger)] text-white hover:bg-[var(--danger-hover)] transition-colors shadow-sm"
              >
                {isDeleting && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                <span>{isDeleting ? 'Menghapus...' : 'Ya, Hapus Kunci'}</span>
              </button>
            </div>
          </div>
        </ModalOverlay>
      )}
    </div>
  );
};
