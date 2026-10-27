import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect, useMemo, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import {
  X,
  RefreshCw,
  Search,
  CheckCircle2,
  AlertCircle,
  Download,
  Trash2,
  Loader2,
  ShieldCheck,
  Power,
  Layers,
} from 'lucide-react';
import { PhpExtensionItem } from '../../types/api';
import {
  fetchPhpExtensions,
  installPhpExtension,
  togglePhpExtension,
  removePhpExtension,
} from '../../api/client';
import { toast } from 'sonner';

interface PhpExtensionsModalProps {
  version: string;
  isOpen: boolean;
  onClose: () => void;
}

export const PhpExtensionsModal: React.FC<PhpExtensionsModalProps> = ({
  version,
  isOpen,
  onClose,
}) => {
  const { t } = useTranslation();
  const [extensions, setExtensions] = useState<PhpExtensionItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [search, setSearch] = useState('');
  const [selectedCategory, setSelectedCategory] = useState<string>('all');
  const [actionInProgress, setActionInProgress] = useState<string | null>(null);

  const loadExtensions = useCallback(
    async (quiet = false) => {
      if (!version) return;
      if (!quiet) setLoading(true);
      setError(null);
      try {
        const res = await fetchPhpExtensions(version);
        setExtensions(res.extensions || []);
      } catch (err: unknown) {
        const msg = err instanceof Error ? err.message : 'Gagal memuat daftar ekstensi PHP.';
        setError(msg);
        if (!quiet) {
          toast.error('Gagal memuat ekstensi', { description: msg });
        }
      } finally {
        if (!quiet) setLoading(false);
      }
    },
    [version]
  );

  useEffect(() => {
    if (isOpen && version) {
      loadExtensions();
    }
  }, [isOpen, version, loadExtensions]);

  // Categories
  const categories = useMemo(() => {
    const set = new Set<string>();
    extensions.forEach((e) => {
      if (e.category) set.add(e.category);
    });
    return Array.from(set).sort();
  }, [extensions]);

  // Filtered list
  const filteredExtensions = useMemo(() => {
    return extensions.filter((ext) => {
      const matchSearch =
        ext.name.toLowerCase().includes(search.toLowerCase()) ||
        ext.description.toLowerCase().includes(search.toLowerCase()) ||
        ext.package_name.toLowerCase().includes(search.toLowerCase());
      const matchCat = selectedCategory === 'all' || ext.category === selectedCategory;
      return matchSearch && matchCat;
    });
  }, [extensions, search, selectedCategory]);

  const installedCount = useMemo(() => extensions.filter((e) => e.installed).length, [extensions]);
  const enabledCount = useMemo(() => extensions.filter((e) => e.enabled).length, [extensions]);

  const handleToggle = async (ext: PhpExtensionItem) => {
    setActionInProgress(ext.name);
    const newStatus = !ext.enabled;
    try {
      await togglePhpExtension(version, ext.name, newStatus);
      toast.success(
        `Ekstensi ${ext.name} berhasil ${newStatus ? 'diaktifkan' : 'dinonaktifkan'}`
      );
      // Optimistic update
      setExtensions((prev) =>
        prev.map((e) => (e.name === ext.name ? { ...e, enabled: newStatus } : e))
      );
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal mengubah status ekstensi.';
      toast.error(`Gagal mengubah status ${ext.name}`, { description: msg });
      loadExtensions(true);
    } finally {
      setActionInProgress(null);
    }
  };

  const handleInstall = async (ext: PhpExtensionItem) => {
    setActionInProgress(ext.name);
    try {
      const job = await installPhpExtension(version, ext.name);
      toast.success(`Tugas pemasangan ekstensi ${ext.name} telah dibuat`, {
        description: `ID Tugas: ${job.id}. Paket sedang diinstal di latar belakang.`,
      });
      setTimeout(() => loadExtensions(true), 3000);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal memasang ekstensi.';
      toast.error(`Gagal memasang ${ext.name}`, { description: msg });
    } finally {
      setActionInProgress(null);
    }
  };

  const handleRemove = async (ext: PhpExtensionItem) => {
    if (ext.is_base) {
      toast.error('Ekstensi dasar tidak dapat dicopot');
      return;
    }
    setActionInProgress(ext.name);
    try {
      const job = await removePhpExtension(version, ext.name);
      toast.success(`Tugas pencopotan ekstensi ${ext.name} telah dibuat`, {
        description: `ID Tugas: ${job.id}. Paket sedang dicopot dari sistem.`,
      });
      setTimeout(() => loadExtensions(true), 3000);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal mencopot ekstensi.';
      toast.error(`Gagal mencopot ${ext.name}`, { description: msg });
    } finally {
      setActionInProgress(null);
    }
  };

  if (!isOpen) return null;

  return (
    <ModalOverlay
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm animate-in fade-in duration-200"
      onDismiss={onClose}
      aria-labelledby="extensions-modal-title"
    >
      <div className="bg-[var(--card)] border border-[var(--line)] rounded-[var(--radius-lg)] shadow-2xl w-full max-w-4xl max-h-[90vh] flex flex-col overflow-hidden dialog-panel">
        {/* Header */}
        <div className="p-5 border-b border-[var(--line)] flex items-center justify-between bg-[var(--bg)]/50 dialog-header">
          <div className="flex items-center gap-3">
            <div className="w-10 h-10 rounded-[var(--radius-md)] bg-[var(--primary)]/10 text-[var(--primary)] font-bold text-sm flex items-center justify-center">
              {version}
            </div>
            <div>
              <h2 id="extensions-modal-title" className="text-base font-semibold text-[var(--ink)]">
                {t('runtime.extensionsTitle', { version })}
              </h2>
              <p className="text-xs text-[var(--ink-muted)]">{t('runtime.extensionsSubtitle')}</p>
            </div>
          </div>

          <div className="flex items-center gap-2">
            <button
              onClick={() => loadExtensions()}
              disabled={loading}
              title="Muat Ulang"
              className="p-2 text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--line)]/50 rounded-[var(--radius-md)] transition-colors disabled:opacity-50"
            >
              <RefreshCw className={`w-4 h-4 ${loading ? 'animate-spin' : ''}`} />
            </button>
            <button
              onClick={onClose}
              className="p-2 text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--line)]/50 rounded-[var(--radius-md)] transition-colors"
            >
              <X className="w-4 h-4" />
            </button>
          </div>
        </div>

        {/* Toolbar: Stats, Search, and Category Filters */}
        <div className="p-4 border-b border-[var(--line)] space-y-3 bg-[var(--bg)]/20">
          <div className="flex flex-col sm:flex-row items-stretch sm:items-center justify-between gap-3">
            {/* Search */}
            <div className="relative flex-1 max-w-md">
              <Search className="w-4 h-4 absolute left-3 top-1/2 -translate-y-1/2 text-[var(--ink-muted)]" />
              <input
                type="text"
                placeholder={t('runtime.searchExtensions')}
                value={search}
                onChange={(e) => setSearch(e.target.value)}
                className="w-full pl-9 pr-3 py-1.5 text-xs rounded-[var(--radius-md)] bg-[var(--bg)] border border-[var(--line)] text-[var(--ink)] focus:outline-none focus:ring-1 focus:ring-[var(--primary)]"
              />
            </div>

            {/* Stats chips */}
            <div className="flex items-center gap-2 text-xs">
              <span className="px-2.5 py-1 rounded-full bg-[var(--card)] border border-[var(--line)] text-[var(--ink-muted)]">
                Total: <strong className="text-[var(--ink)]">{extensions.length}</strong>
              </span>
              <span className="px-2.5 py-1 rounded-full bg-emerald-500/10 border border-emerald-500/20 text-emerald-600 dark:text-emerald-400">
                Terpasang: <strong>{installedCount}</strong>
              </span>
              <span className="px-2.5 py-1 rounded-full bg-[var(--primary)]/10 border border-[var(--primary)]/20 text-[var(--primary)]">
                Aktif: <strong>{enabledCount}</strong>
              </span>
            </div>
          </div>

          {/* Category Filter Pills */}
          <div className="flex items-center gap-1.5 overflow-x-auto pb-1 text-xs no-scrollbar">
            <button
              onClick={() => setSelectedCategory('all')}
              className={`px-3 py-1 rounded-full whitespace-nowrap transition-colors text-xs font-medium ${
                selectedCategory === 'all'
                  ? 'bg-[var(--primary)] text-white'
                  : 'bg-[var(--card)] border border-[var(--line)] text-[var(--ink-muted)] hover:text-[var(--ink)]'
              }`}
            >
              {t('runtime.allCategories')}
            </button>
            {categories.map((cat) => (
              <button
                key={cat}
                onClick={() => setSelectedCategory(cat)}
                className={`px-3 py-1 rounded-full whitespace-nowrap transition-colors text-xs font-medium ${
                  selectedCategory === cat
                    ? 'bg-[var(--primary)] text-white'
                    : 'bg-[var(--card)] border border-[var(--line)] text-[var(--ink-muted)] hover:text-[var(--ink)]'
                }`}
              >
                {cat}
              </button>
            ))}
          </div>
        </div>

        {/* Content list */}
        <div className="flex-1 overflow-y-auto p-4 space-y-2.5 dialog-body">
          {loading && extensions.length === 0 ? (
            <div className="p-12 text-center text-xs text-[var(--ink-muted)] flex flex-col items-center gap-2">
              <Loader2 className="w-5 h-5 animate-spin text-[var(--primary)]" />
              <span>Memuat daftar ekstensi PHP {version}...</span>
            </div>
          ) : error ? (
            <div className="p-8 text-center text-xs text-rose-500 flex flex-col items-center gap-2">
              <AlertCircle className="w-5 h-5" />
              <span>{error}</span>
            </div>
          ) : filteredExtensions.length === 0 ? (
            <div className="p-12 text-center text-xs text-[var(--ink-muted)]">
              Tidak ada ekstensi yang cocok dengan filter pencarian.
            </div>
          ) : (
            filteredExtensions.map((ext) => {
              const isBusy = actionInProgress === ext.name;

              return (
                <div
                  key={ext.name}
                  className="p-3.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--card)] hover:border-[var(--line-strong,var(--primary))]/30 transition-all flex flex-col sm:flex-row sm:items-center justify-between gap-3"
                >
                  {/* Left: Info */}
                  <div className="space-y-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="font-mono text-xs font-bold text-[var(--ink)] bg-[var(--bg)] px-2 py-0.5 rounded border border-[var(--line)]">
                        {ext.name}
                      </span>
                      <span className="text-[10px] font-medium text-[var(--ink-muted)] bg-[var(--line)]/40 px-2 py-0.5 rounded-full">
                        {ext.category}
                      </span>

                      {ext.is_base ? (
                        <span className="inline-flex items-center gap-1 text-[10px] font-semibold text-violet-600 dark:text-violet-400 bg-violet-500/10 px-2 py-0.5 rounded-full border border-violet-500/20">
                          <ShieldCheck className="w-2.5 h-2.5" />
                          {t('runtime.coreModule')}
                        </span>
                      ) : ext.installed && ext.enabled ? (
                        <span className="inline-flex items-center gap-1 text-[10px] font-semibold text-emerald-600 dark:text-emerald-400 bg-emerald-500/10 px-2 py-0.5 rounded-full border border-emerald-500/20">
                          <CheckCircle2 className="w-2.5 h-2.5" />
                          {t('runtime.extensionActive')}
                        </span>
                      ) : ext.installed && !ext.enabled ? (
                        <span className="inline-flex items-center gap-1 text-[10px] font-semibold text-amber-600 dark:text-amber-400 bg-amber-500/10 px-2 py-0.5 rounded-full border border-amber-500/20">
                          <Power className="w-2.5 h-2.5" />
                          {t('runtime.extensionInactive')}
                        </span>
                      ) : (
                        <span className="inline-flex items-center text-[10px] font-medium text-[var(--ink-muted)] bg-[var(--line)]/40 px-2 py-0.5 rounded-full">
                          {t('runtime.available')}
                        </span>
                      )}
                    </div>

                    <p className="text-xs text-[var(--ink-muted)] line-clamp-1">{ext.description}</p>

                    <div className="text-[10px] text-[var(--ink-muted)] font-mono">Paket: {ext.package_name}</div>
                  </div>

                  {/* Right: Actions */}
                  <div className="flex items-center gap-2 self-end sm:self-center">
                    {ext.is_base ? (
                      <span className="text-[11px] text-[var(--ink-muted)] italic px-2">
                        Modul Inti (Terkunci)
                      </span>
                    ) : ext.installed ? (
                      <>
                        {/* Toggle Enable/Disable Button */}
                        <button
                          onClick={() => handleToggle(ext)}
                          disabled={isBusy}
                          className={`inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] border transition-colors disabled:opacity-50 ${
                            ext.enabled
                              ? 'border-amber-500/30 text-amber-600 dark:text-amber-400 hover:bg-amber-500/10'
                              : 'border-emerald-500/30 text-emerald-600 dark:text-emerald-400 hover:bg-emerald-500/10'
                          }`}
                        >
                          {isBusy ? (
                            <Loader2 className="w-3.5 h-3.5 animate-spin" />
                          ) : (
                            <Power className="w-3.5 h-3.5" />
                          )}
                          {ext.enabled ? t('runtime.toggleDisable') : t('runtime.toggleEnable')}
                        </button>

                        {/* Remove Button */}
                        <button
                          onClick={() => handleRemove(ext)}
                          disabled={isBusy}
                          title={t('runtime.removeExt')}
                          className="p-1.5 text-rose-500 hover:bg-rose-500/10 rounded-[var(--radius-md)] border border-rose-500/20 transition-colors disabled:opacity-50"
                        >
                          <Trash2 className="w-3.5 h-3.5" />
                        </button>
                      </>
                    ) : (
                      /* Install Button */
                      <button
                        onClick={() => handleInstall(ext)}
                        disabled={isBusy}
                        className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-semibold rounded-[var(--radius-md)] bg-[var(--primary)] text-white hover:opacity-90 transition-opacity disabled:opacity-50"
                      >
                        {isBusy ? (
                          <Loader2 className="w-3.5 h-3.5 animate-spin" />
                        ) : (
                          <Download className="w-3.5 h-3.5" />
                        )}
                        {t('runtime.installExt')}
                      </button>
                    )}
                  </div>
                </div>
              );
            })
          )}
        </div>

        {/* Footer */}
        <div className="p-4 border-t border-[var(--line)] bg-[var(--bg)]/50 flex items-center justify-between text-xs text-[var(--ink-muted)] dialog-footer">
          <span className="flex items-center gap-1.5">
            <Layers className="w-3.5 h-3.5 text-[var(--primary)]" />
            Preferensi ekstensi tingkat situs dapat disesuaikan pada pengaturan masing-masing situs.
          </span>
          <button
            onClick={onClose}
            className="px-4 py-1.5 rounded-[var(--radius-md)] bg-[var(--card)] border border-[var(--line)] text-[var(--ink)] hover:bg-[var(--line)]/50 transition-colors"
          >
            Tutup
          </button>
        </div>
      </div>
    </ModalOverlay>
  );
};

export default PhpExtensionsModal;
