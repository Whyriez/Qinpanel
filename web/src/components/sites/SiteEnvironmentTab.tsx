import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect, useCallback, useMemo } from 'react';
import {
  Key,
  Eye,
  EyeOff,
  Plus,
  Upload,
  RefreshCw,
  Trash2,
  Edit3,
  Copy,
  Check,
  AlertTriangle,
  Shield,
  Search,
  FileText,
  X,
  Loader2,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  fetchSiteEnv,
  updateSiteEnv,
  importSiteDotEnv,
  restartSiteForEnv,
} from '../../api/client';
import { EnvVariableItem } from '../../types/api';

export interface SiteEnvironmentTabProps {
  siteId: number;
  onSiteRestarted?: () => void;
}

export const SiteEnvironmentTab: React.FC<SiteEnvironmentTabProps> = ({
  siteId,
  onSiteRestarted,
}) => {
  const { t } = useTranslation();

  const [variables, setVariables] = useState<EnvVariableItem[]>([]);
  const [restartRequired, setRestartRequired] = useState(false);
  const [isLoading, setIsLoading] = useState(true);
  const [isRestarting, setIsRestarting] = useState(false);
  const [searchQuery, setSearchQuery] = useState('');

  // Local cache of revealed values
  const [revealedValues, setRevealedValues] = useState<Record<string, string>>({});
  const [revealedKeys, setRevealedKeys] = useState<Set<string>>(new Set());
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  // Global toast
  const [toastMessage, setToastMessage] = useState<{
    text: string;
    type: 'success' | 'error';
  } | null>(null);

  // Add / Edit Modal state
  const [isEditModalOpen, setIsEditModalOpen] = useState(false);
  const [modalMode, setModalMode] = useState<'add' | 'edit'>('add');
  const [formKey, setFormKey] = useState('');
  const [formValue, setFormValue] = useState('');
  const [isSaving, setIsSaving] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  // Import Modal state
  const [isImportModalOpen, setIsImportModalOpen] = useState(false);
  const [importContent, setImportContent] = useState('');
  const [importMerge, setImportMerge] = useState(true);
  const [isImporting, setIsImporting] = useState(false);
  const [importError, setImportError] = useState<string | null>(null);

  // Delete state
  const [keyToDelete, setKeyToDelete] = useState<string | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  const showToast = (text: string, type: 'success' | 'error' = 'success') => {
    setToastMessage({ text, type });
    setTimeout(() => setToastMessage(null), 4000);
  };

  // Load environment variables (masked by default)
  const loadVariables = useCallback(async () => {
    setIsLoading(true);
    try {
      const res = await fetchSiteEnv(siteId, false);
      setVariables(res.variables);
      setRestartRequired(res.restart_required);
    } catch (err: unknown) {
      const e = err as Error;
      showToast(e.message || t('env.load_failed', 'Gagal memuat variabel lingkungan'), 'error');
    } finally {
      setIsLoading(false);
    }
  }, [siteId, t]);

  useEffect(() => {
    loadVariables();
  }, [loadVariables]);

  // Fetch revealed values from backend on demand
  const ensureRevealed = async (keysToReveal?: string[]) => {
    try {
      const res = await fetchSiteEnv(siteId, true);
      const newMap: Record<string, string> = {};
      for (const item of res.variables) {
        newMap[item.key] = item.value;
      }
      setRevealedValues(newMap);
      if (keysToReveal) {
        setRevealedKeys((prev) => {
          const next = new Set(prev);
          for (const k of keysToReveal) next.add(k);
          return next;
        });
      } else {
        setRevealedKeys(new Set(res.variables.map((v) => v.key)));
      }
    } catch (err: unknown) {
      const e = err as Error;
      showToast(e.message || t('env.reveal_failed', 'Gagal mendekripsi variabel lingkungan'), 'error');
    }
  };

  // Toggle single variable reveal
  const handleToggleReveal = async (key: string) => {
    if (revealedKeys.has(key)) {
      setRevealedKeys((prev) => {
        const next = new Set(prev);
        next.delete(key);
        return next;
      });
    } else {
      if (revealedValues[key] !== undefined) {
        setRevealedKeys((prev) => new Set(prev).add(key));
      } else {
        await ensureRevealed([key]);
      }
    }
  };

  // Reveal / Hide All
  const areAllRevealed = variables.length > 0 && variables.every((v) => revealedKeys.has(v.key));

  const handleToggleRevealAll = async () => {
    if (areAllRevealed) {
      setRevealedKeys(new Set());
    } else {
      await ensureRevealed();
    }
  };

  // Copy to clipboard helper
  const handleCopy = (text: string, identifier: string) => {
    navigator.clipboard.writeText(text);
    setCopiedKey(identifier);
    setTimeout(() => setCopiedKey(null), 2000);
  };

  // Open modal for adding
  const handleOpenAdd = () => {
    setModalMode('add');
    setFormKey('');
    setFormValue('');
    setFormError(null);
    setIsEditModalOpen(true);
  };

  // Open modal for editing
  const handleOpenEdit = async (item: EnvVariableItem) => {
    setModalMode('edit');
    setFormKey(item.key);
    setFormError(null);

    // Make sure we have plaintext value
    let val = revealedValues[item.key];
    if (val === undefined) {
      try {
        const res = await fetchSiteEnv(siteId, true);
        const map: Record<string, string> = {};
        for (const v of res.variables) map[v.key] = v.value;
        setRevealedValues(map);
        val = map[item.key] || '';
      } catch {
        val = '';
      }
    }
    setFormValue(val || '');
    setIsEditModalOpen(true);
  };

  // Save (Add / Edit) Variable
  const handleSaveVariable = async (e: React.FormEvent) => {
    e.preventDefault();
    const cleanKey = formKey.trim();
    if (!cleanKey) {
      setFormError(t('env.key_required', 'Nama variabel tidak boleh kosong'));
      return;
    }
    const keyRegex = /^[a-zA-Z_][a-zA-Z0-9_]*$/;
    if (!keyRegex.test(cleanKey)) {
      setFormError(
        t(
          'env.key_invalid',
          'Nama variabel harus diawali huruf/underscore dan hanya mengandung huruf, angka, atau underscore'
        )
      );
      return;
    }

    setIsSaving(true);
    setFormError(null);

    try {
      // First ensure we have all plaintext values
      const revealRes = await fetchSiteEnv(siteId, true);
      const currentMap: Record<string, string> = {};
      for (const v of revealRes.variables) {
        currentMap[v.key] = v.value;
      }
      currentMap[cleanKey] = formValue;

      const res = await updateSiteEnv(siteId, { variables: currentMap });
      showToast(res.message);
      setIsEditModalOpen(false);

      // Update local cache
      setRevealedValues(currentMap);
      setRevealedKeys((prev) => new Set(prev).add(cleanKey));
      await loadVariables();
    } catch (err: unknown) {
      const e = err as Error;
      setFormError(e.message || t('env.save_failed', 'Gagal menyimpan variabel lingkungan'));
    } finally {
      setIsSaving(false);
    }
  };

  // Delete Variable
  const handleDeleteVariable = async () => {
    if (!keyToDelete) return;
    setIsDeleting(true);
    try {
      const revealRes = await fetchSiteEnv(siteId, true);
      const currentMap: Record<string, string> = {};
      for (const v of revealRes.variables) {
        if (v.key !== keyToDelete) {
          currentMap[v.key] = v.value;
        }
      }

      const res = await updateSiteEnv(siteId, { variables: currentMap });
      showToast(res.message);
      setKeyToDelete(null);

      // Update local cache
      setRevealedValues(currentMap);
      setRevealedKeys((prev) => {
        const next = new Set(prev);
        next.delete(keyToDelete);
        return next;
      });
      await loadVariables();
    } catch (err: unknown) {
      const e = err as Error;
      showToast(e.message || t('env.delete_failed', 'Gagal menghapus variabel'), 'error');
    } finally {
      setIsDeleting(false);
    }
  };

  // Import .env
  const handleImportSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!importContent.trim()) {
      setImportError(t('env.import_empty', 'Konten file .env tidak boleh kosong'));
      return;
    }
    setIsImporting(true);
    setImportError(null);
    try {
      const res = await importSiteDotEnv(siteId, {
        content: importContent,
        merge: importMerge,
      });
      showToast(res.message);
      setIsImportModalOpen(false);
      setImportContent('');
      setRevealedValues({});
      setRevealedKeys(new Set());
      await loadVariables();
    } catch (err: unknown) {
      const e = err as Error;
      setImportError(e.message || t('env.import_failed', 'Gagal mengimpor file .env'));
    } finally {
      setIsImporting(false);
    }
  };

  // Handle .env file upload
  const handleFileUpload = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    const reader = new FileReader();
    reader.onload = (event) => {
      const text = event.target?.result as string;
      if (text) {
        setImportContent(text);
      }
    };
    reader.readAsText(file);
  };

  // Restart Service for Env changes
  const handleRestartNow = async () => {
    setIsRestarting(true);
    try {
      const res = await restartSiteForEnv(siteId);
      showToast(res.message);
      setRestartRequired(false);
      onSiteRestarted?.();
    } catch (err: unknown) {
      const e = err as Error;
      showToast(e.message || t('env.restart_failed', 'Gagal memulai ulang proses'), 'error');
    } finally {
      setIsRestarting(false);
    }
  };

  // Filter variables by search query
  const filteredVariables = useMemo(() => {
    if (!searchQuery.trim()) return variables;
    const q = searchQuery.toLowerCase();
    return variables.filter((v) => v.key.toLowerCase().includes(q));
  }, [variables, searchQuery]);

  return (
    <div className="space-y-6">
      {/* Toast Alert */}
      {toastMessage && (
        <div
          role="status"
          className={`p-4 rounded-xl flex items-center gap-3 text-sm font-medium transition-all shadow-sm ${
            toastMessage.type === 'success'
              ? 'bg-emerald-50 text-emerald-800 dark:bg-emerald-950/60 dark:text-emerald-200 border border-emerald-200 dark:border-emerald-800/60'
              : 'bg-rose-50 text-rose-800 dark:bg-rose-950/60 dark:text-rose-200 border border-rose-200 dark:border-rose-800/60'
          }`}
        >
          {toastMessage.type === 'success' ? (
            <Check className="w-5 h-5 flex-shrink-0 text-emerald-600 dark:text-emerald-400" />
          ) : (
            <AlertTriangle className="w-5 h-5 flex-shrink-0 text-rose-600 dark:text-rose-400" />
          )}
          <span>{toastMessage.text}</span>
        </div>
      )}

      {/* Restart Required Warning Banner */}
      {restartRequired && (
        <div
          role="alert"
          className="p-5 rounded-2xl bg-amber-50/80 dark:bg-amber-950/40 border border-amber-300 dark:border-amber-800/60 flex flex-col sm:flex-row items-start sm:items-center justify-between gap-4 shadow-sm"
        >
          <div className="flex items-start gap-3.5">
            <div className="p-2.5 rounded-xl bg-amber-100 dark:bg-amber-900/60 text-amber-700 dark:text-amber-300 flex-shrink-0 mt-0.5">
              <AlertTriangle className="w-5 h-5" />
            </div>
            <div>
              <h4 className="text-sm font-semibold text-amber-900 dark:text-amber-200">
                {t('env.restart_banner_title', 'Perubahan Variabel Lingkungan Memerlukan Restart')}
              </h4>
              <p className="text-xs text-amber-700 dark:text-amber-300/90 mt-0.5 leading-relaxed">
                {t(
                  'env.restart_banner_desc',
                  'Perubahan nilai atau penambahan variabel telah dienkripsi dan disimpan, tetapi proses aplikasi perlu dimulai ulang agar dapat memuat variabel baru.'
                )}
              </p>
            </div>
          </div>
          <button
            onClick={handleRestartNow}
            disabled={isRestarting}
            className="w-full sm:w-auto px-4 py-2.5 rounded-xl bg-amber-600 hover:bg-amber-700 text-white font-medium text-xs flex items-center justify-center gap-2 shadow-sm transition-colors flex-shrink-0 disabled:opacity-50"
          >
            {isRestarting ? (
              <Loader2 className="w-4 h-4 animate-spin" />
            ) : (
              <RefreshCw className="w-4 h-4" />
            )}
            {t('env.restart_now', 'Restart Sekarang')}
          </button>
        </div>
      )}

      {/* Main Card */}
      <div className="bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700/60 rounded-2xl shadow-sm overflow-hidden">
        {/* Card Header & Toolbar */}
        <div className="p-6 border-b border-slate-200 dark:border-slate-700/60 flex flex-col md:flex-row md:items-center justify-between gap-4">
          <div>
            <div className="flex items-center gap-2.5">
              <div className="p-2 rounded-xl bg-indigo-50 dark:bg-indigo-950/60 text-indigo-600 dark:text-indigo-400">
                <Key className="w-5 h-5" />
              </div>
              <h3 className="text-base font-semibold text-slate-900 dark:text-white">
                {t('env.title', 'Variabel Lingkungan (Environment Variables)')}
              </h3>
            </div>
            <p className="text-xs text-slate-500 dark:text-slate-400 mt-1">
              {t(
                'env.subtitle',
                'Terenkripsi dengan XChaCha20-Poly1305 di level agent daemon. Plaintext tidak pernah disimpan di database atau log.'
              )}
            </p>
          </div>

          <div className="flex items-center flex-wrap gap-2.5">
            <button
              onClick={handleToggleRevealAll}
              disabled={isLoading || variables.length === 0}
              className="px-3 py-2 rounded-xl border border-slate-300 dark:border-slate-600 hover:bg-slate-50 dark:hover:bg-slate-700/50 text-slate-700 dark:text-slate-200 text-xs font-medium flex items-center gap-2 transition-colors disabled:opacity-50"
              title={areAllRevealed ? 'Sembunyikan Semua Nilai' : 'Tampilkan Semua Nilai'}
            >
              {areAllRevealed ? (
                <>
                  <EyeOff className="w-4 h-4 text-slate-500" />
                  {t('env.hide_all', 'Sembunyikan Semua')}
                </>
              ) : (
                <>
                  <Eye className="w-4 h-4 text-slate-500" />
                  {t('env.reveal_all', 'Tampilkan Semua')}
                </>
              )}
            </button>

            <button
              onClick={() => setIsImportModalOpen(true)}
              disabled={isLoading}
              className="px-3 py-2 rounded-xl border border-slate-300 dark:border-slate-600 hover:bg-slate-50 dark:hover:bg-slate-700/50 text-slate-700 dark:text-slate-200 text-xs font-medium flex items-center gap-2 transition-colors disabled:opacity-50"
            >
              <Upload className="w-4 h-4 text-slate-500" />
              {t('env.import_btn', 'Import .env')}
            </button>

            <button
              onClick={handleOpenAdd}
              disabled={isLoading}
              className="px-3.5 py-2 rounded-xl bg-indigo-600 hover:bg-indigo-700 text-white text-xs font-medium flex items-center gap-2 shadow-sm transition-colors disabled:opacity-50"
            >
              <Plus className="w-4 h-4" />
              {t('env.add_btn', 'Tambah Variabel')}
            </button>
          </div>
        </div>

        {/* Filter bar */}
        <div className="px-6 py-3 bg-slate-50/60 dark:bg-slate-800/40 border-b border-slate-200/80 dark:border-slate-700/40 flex items-center justify-between gap-4">
          <div className="relative flex-1 max-w-sm">
            <Search className="w-4 h-4 absolute left-3 top-2.5 text-slate-400" />
            <input
              type="text"
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              placeholder={t('env.search_placeholder', 'Cari variabel...')}
              className="w-full pl-9 pr-3 py-1.5 text-xs rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-900 dark:text-white placeholder-slate-400 focus:outline-none focus:ring-2 focus:ring-indigo-500"
            />
          </div>

          <div className="flex items-center gap-2 text-xs text-slate-500 dark:text-slate-400">
            <Shield className="w-3.5 h-3.5 text-emerald-500" />
            <span>
              {variables.length} {t('env.vars_count', 'variabel terenkripsi')}
            </span>
          </div>
        </div>

        {/* Table Content */}
        {isLoading ? (
          <div className="p-12 text-center">
            <Loader2 className="w-6 h-6 animate-spin mx-auto text-indigo-600 dark:text-indigo-400 mb-2" />
            <p className="text-xs text-slate-500 dark:text-slate-400">
              {t('env.loading', 'Memuat variabel lingkungan...')}
            </p>
          </div>
        ) : filteredVariables.length === 0 ? (
          <div className="p-12 text-center">
            <FileText className="w-10 h-10 text-slate-300 dark:text-slate-600 mx-auto mb-3" />
            <h4 className="text-sm font-semibold text-slate-800 dark:text-slate-200">
              {variables.length === 0
                ? t('env.empty_title', 'Belum Ada Variabel Lingkungan')
                : t('env.no_search_results', 'Tidak ada variabel yang cocok')}
            </h4>
            <p className="text-xs text-slate-500 dark:text-slate-400 mt-1 max-w-sm mx-auto">
              {variables.length === 0
                ? t(
                    'env.empty_desc',
                    'Tambahkan variabel baru atau impor file .env untuk mengonfigurasi aplikasi Anda secara aman.'
                  )
                : t('env.try_different_search', 'Coba gunakan kata kunci pencarian yang lain.')}
            </p>
            {variables.length === 0 && (
              <div className="mt-4 flex items-center justify-center gap-3">
                <button
                  onClick={handleOpenAdd}
                  className="px-4 py-2 rounded-xl bg-indigo-600 hover:bg-indigo-700 text-white text-xs font-medium flex items-center gap-2"
                >
                  <Plus className="w-4 h-4" />
                  {t('env.add_btn', 'Tambah Variabel')}
                </button>
                <button
                  onClick={() => setIsImportModalOpen(true)}
                  className="px-4 py-2 rounded-xl border border-slate-300 dark:border-slate-600 hover:bg-slate-50 dark:hover:bg-slate-700/50 text-slate-700 dark:text-slate-200 text-xs font-medium flex items-center gap-2"
                >
                  <Upload className="w-4 h-4" />
                  {t('env.import_btn', 'Import .env')}
                </button>
              </div>
            )}
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-xs border-collapse">
              <thead>
                <tr className="border-b border-slate-200 dark:border-slate-700/60 bg-slate-50/40 dark:bg-slate-800/20 text-slate-500 dark:text-slate-400 font-medium">
                  <th className="py-3 px-6 w-1/3">{t('env.col_key', 'Nama Variabel (Key)')}</th>
                  <th className="py-3 px-6">{t('env.col_value', 'Nilai (Value)')}</th>
                  <th className="py-3 px-6 text-right w-28">{t('env.col_actions', 'Aksi')}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-100 dark:divide-slate-700/40">
                {filteredVariables.map((item) => {
                  const isRevealed = revealedKeys.has(item.key);
                  const displayValue = isRevealed
                    ? revealedValues[item.key] ?? '••••••••'
                    : '••••••••';

                  return (
                    <tr
                      key={item.key}
                      className="hover:bg-slate-50/70 dark:hover:bg-slate-750/50 transition-colors group"
                    >
                      {/* Variable Key */}
                      <td className="py-3.5 px-6 font-mono font-semibold text-slate-800 dark:text-slate-100">
                        <div className="flex items-center gap-2">
                          <span className="truncate">{item.key}</span>
                          <button
                            onClick={() => handleCopy(item.key, `k_${item.key}`)}
                            className="opacity-0 group-hover:opacity-100 p-1 text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 rounded transition-opacity"
                            title="Salin Key"
                          >
                            {copiedKey === `k_${item.key}` ? (
                              <Check className="w-3.5 h-3.5 text-emerald-500" />
                            ) : (
                              <Copy className="w-3.5 h-3.5" />
                            )}
                          </button>
                        </div>
                      </td>

                      {/* Variable Value */}
                      <td className="py-3.5 px-6">
                        <div className="flex items-center gap-3">
                          <span
                            className={`font-mono text-xs px-2.5 py-1 rounded-lg ${
                              isRevealed
                                ? 'bg-slate-100 dark:bg-slate-900 text-slate-800 dark:text-slate-200'
                                : 'bg-slate-100/70 dark:bg-slate-800 text-slate-400 tracking-widest'
                            } max-w-md truncate`}
                          >
                            {displayValue}
                          </span>

                          {/* Reveal/Hide toggle button */}
                          <button
                            onClick={() => handleToggleReveal(item.key)}
                            className="p-1.5 rounded-lg text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-700/50 transition-colors"
                            title={isRevealed ? 'Sembunyikan' : 'Tampilkan'}
                          >
                            {isRevealed ? (
                              <EyeOff className="w-4 h-4" />
                            ) : (
                              <Eye className="w-4 h-4" />
                            )}
                          </button>

                          {/* Copy revealed value button */}
                          {isRevealed && (
                            <button
                              onClick={() => handleCopy(displayValue, `v_${item.key}`)}
                              className="p-1.5 rounded-lg text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-700/50 transition-colors"
                              title="Salin Nilai"
                            >
                              {copiedKey === `v_${item.key}` ? (
                                <Check className="w-4 h-4 text-emerald-500" />
                              ) : (
                                <Copy className="w-4 h-4" />
                              )}
                            </button>
                          )}
                        </div>
                      </td>

                      {/* Actions */}
                      <td className="py-3.5 px-6 text-right">
                        <div className="flex items-center justify-end gap-1.5">
                          <button
                            onClick={() => handleOpenEdit(item)}
                            className="p-1.5 rounded-lg text-slate-400 hover:text-indigo-600 dark:hover:text-indigo-400 hover:bg-slate-100 dark:hover:bg-slate-700/50 transition-colors"
                            title="Edit Variabel"
                          >
                            <Edit3 className="w-4 h-4" />
                          </button>
                          <button
                            onClick={() => setKeyToDelete(item.key)}
                            className="p-1.5 rounded-lg text-slate-400 hover:text-rose-600 dark:hover:text-rose-400 hover:bg-slate-100 dark:hover:bg-slate-700/50 transition-colors"
                            title="Hapus Variabel"
                          >
                            <Trash2 className="w-4 h-4" />
                          </button>
                        </div>
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
        )}
      </div>

      {/* Add / Edit Variable Modal */}
      {isEditModalOpen && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/50 backdrop-blur-sm animate-in fade-in duration-150"
          onDismiss={() => setIsEditModalOpen(false)}
        >
          <div className="bg-white dark:bg-slate-800 rounded-2xl max-w-lg w-full p-6 shadow-xl border border-slate-200 dark:border-slate-700/60 relative dialog-panel">
            <button
              onClick={() => setIsEditModalOpen(false)}
              className="absolute top-5 right-5 p-1 rounded-lg text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-700 dialog-header"
            >
              <X className="w-5 h-5" />
            </button>
            <form onSubmit={handleSaveVariable} className="dialog-form">
              <div className="dialog-body space-y-4">
                <div className="flex items-center gap-3 mb-5">
                  <div className="p-2 rounded-xl bg-indigo-50 dark:bg-indigo-950/60 text-indigo-600 dark:text-indigo-400">
                    <Key className="w-5 h-5" />
                  </div>
                  <h3 className="text-base font-semibold text-slate-900 dark:text-white">
                    {modalMode === 'add'
                      ? t('env.modal_add_title', 'Tambah Variabel Lingkungan')
                      : t('env.modal_edit_title', 'Edit Variabel Lingkungan')}
                  </h3>
                </div>

                {formError && (
                  <div className="mb-4 p-3 rounded-xl bg-rose-50 dark:bg-rose-950/60 text-rose-800 dark:text-rose-200 border border-rose-200 dark:border-rose-800/60 text-xs flex items-center gap-2">
                    <AlertTriangle className="w-4 h-4 flex-shrink-0" />
                    <span>{formError}</span>
                  </div>
                )}

                <div>
                  <label className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1.5">
                    {t('env.form_key_label', 'Nama Variabel (Key)')}
                  </label>
                  <input
                    type="text"
                    value={formKey}
                    onChange={(e) => setFormKey(e.target.value)}
                    disabled={modalMode === 'edit'}
                    placeholder="DATABASE_URL, API_KEY, PORT..."
                    className="w-full px-3.5 py-2 text-xs font-mono rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-900 dark:text-white placeholder-slate-400 focus:outline-none focus:ring-2 focus:ring-indigo-500 disabled:opacity-60"
                    required
                  />
                  <p className="text-[11px] text-slate-400 mt-1">
                    {t(
                      'env.form_key_hint',
                      'Format POSIX: huruf besar/kecil, angka, atau underscore. Contoh: DB_HOST',
                    )}
                  </p>
                </div>

                <div>
                  <label className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1.5">
                    {t('env.form_val_label', 'Nilai (Value)')}
                  </label>
                  <textarea
                    value={formValue}
                    onChange={(e) => setFormValue(e.target.value)}
                    placeholder="Isi nilai variabel..."
                    rows={4}
                    className="w-full px-3.5 py-2 text-xs font-mono rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-900 dark:text-white placeholder-slate-400 focus:outline-none focus:ring-2 focus:ring-indigo-500"
                  />
                </div>
              </div>
              <div className="flex items-center justify-end gap-3 pt-3 dialog-footer">
                <button
                  type="button"
                  onClick={() => setIsEditModalOpen(false)}
                  className="px-4 py-2 rounded-xl border border-slate-300 dark:border-slate-600 hover:bg-slate-50 dark:hover:bg-slate-700/50 text-slate-700 dark:text-slate-200 text-xs font-medium"
                >
                  {t('common.cancel', 'Batal')}
                </button>
                <button
                  type="submit"
                  disabled={isSaving}
                  className="px-4 py-2 rounded-xl bg-indigo-600 hover:bg-indigo-700 text-white text-xs font-medium flex items-center gap-2 disabled:opacity-50"
                >
                  {isSaving && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                  {t('common.save', 'Simpan')}
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* Import .env Modal */}
      {isImportModalOpen && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/50 backdrop-blur-sm animate-in fade-in duration-150"
          onDismiss={() => setIsImportModalOpen(false)}
        >
          <div className="bg-white dark:bg-slate-800 rounded-2xl max-w-xl w-full p-6 shadow-xl border border-slate-200 dark:border-slate-700/60 relative dialog-panel">
            <button
              onClick={() => setIsImportModalOpen(false)}
              className="absolute top-5 right-5 p-1 rounded-lg text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 hover:bg-slate-100 dark:hover:bg-slate-700 dialog-header"
            >
              <X className="w-5 h-5" />
            </button>
            <form onSubmit={handleImportSubmit} className="dialog-form">
              <div className="dialog-body space-y-4">
                <div className="flex items-center gap-3 mb-5">
                  <div className="p-2 rounded-xl bg-indigo-50 dark:bg-indigo-950/60 text-indigo-600 dark:text-indigo-400">
                    <Upload className="w-5 h-5" />
                  </div>
                  <div>
                    <h3 className="text-base font-semibold text-slate-900 dark:text-white">
                      {t('env.import_modal_title', 'Import File .env')}
                    </h3>
                    <p className="text-xs text-slate-500 dark:text-slate-400">
                      {t('env.import_modal_subtitle', 'Tempel konten atau unggah file .env')}
                    </p>
                  </div>
                </div>

                {importError && (
                  <div className="mb-4 p-3 rounded-xl bg-rose-50 dark:bg-rose-950/60 text-rose-800 dark:text-rose-200 border border-rose-200 dark:border-rose-800/60 text-xs flex items-center gap-2">
                    <AlertTriangle className="w-4 h-4 flex-shrink-0" />
                    <span>{importError}</span>
                  </div>
                )}

                <div>
                  <div className="flex items-center justify-between mb-1.5">
                    <label className="text-xs font-medium text-slate-700 dark:text-slate-300">
                      {t('env.import_content_label', 'Konten .env')}
                    </label>
                    <label className="cursor-pointer text-xs font-medium text-indigo-600 dark:text-indigo-400 hover:underline flex items-center gap-1">
                      <FileText className="w-3.5 h-3.5" />
                      <span>{t('env.choose_file', 'Pilih File .env')}</span>
                      <input type="file" accept=".env,text/plain" onChange={handleFileUpload} className="hidden" />
                    </label>
                  </div>
                  <textarea
                    value={importContent}
                    onChange={(e) => setImportContent(e.target.value)}
                    placeholder={`# Contoh:\nDATABASE_URL="postgres://..."\nPORT=3000\nexport API_KEY='xyz'`}
                    rows={8}
                    className="w-full px-3.5 py-2.5 text-xs font-mono rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-900 text-slate-900 dark:text-white placeholder-slate-400 focus:outline-none focus:ring-2 focus:ring-indigo-500"
                    required
                  />
                </div>

                {/* Merge or Replace option */}
                <div className="p-3.5 rounded-xl bg-slate-50 dark:bg-slate-900/60 border border-slate-200 dark:border-slate-700/50 space-y-2">
                  <label className="flex items-center gap-2.5 cursor-pointer">
                    <input
                      type="radio"
                      name="importMode"
                      checked={importMerge}
                      onChange={() => setImportMerge(true)}
                      className="text-indigo-600 focus:ring-indigo-500"
                    />
                    <span className="text-xs text-slate-700 dark:text-slate-300 font-medium">
                      {t('env.merge_option', 'Gabungkan dengan variabel yang ada (Merge)')}
                    </span>
                  </label>
                  <label className="flex items-center gap-2.5 cursor-pointer">
                    <input
                      type="radio"
                      name="importMode"
                      checked={!importMerge}
                      onChange={() => setImportMerge(false)}
                      className="text-indigo-600 focus:ring-indigo-500"
                    />
                    <span className="text-xs text-slate-700 dark:text-slate-300 font-medium">
                      {t('env.replace_option', 'Ganti semua variabel yang ada (Replace All)')}
                    </span>
                  </label>
                </div>
              </div>
              <div className="flex items-center justify-end gap-3 pt-3 dialog-footer">
                <button
                  type="button"
                  onClick={() => setIsImportModalOpen(false)}
                  className="px-4 py-2 rounded-xl border border-slate-300 dark:border-slate-600 hover:bg-slate-50 dark:hover:bg-slate-700/50 text-slate-700 dark:text-slate-200 text-xs font-medium"
                >
                  {t('common.cancel', 'Batal')}
                </button>
                <button
                  type="submit"
                  disabled={isImporting}
                  className="px-4 py-2 rounded-xl bg-indigo-600 hover:bg-indigo-700 text-white text-xs font-medium flex items-center gap-2 disabled:opacity-50"
                >
                  {isImporting && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                  {t('env.import_confirm_btn', 'Import Sekarang')}
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* Delete Confirmation Modal */}
      {keyToDelete && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/50 backdrop-blur-sm animate-in fade-in duration-150"
          onDismiss={() => setKeyToDelete(null)}
        >
          <div className="bg-white dark:bg-slate-800 rounded-2xl max-w-sm w-full p-6 shadow-xl border border-slate-200 dark:border-slate-700/60 dialog-panel">
            <div className="flex items-center gap-3 mb-4 dialog-header">
              <div className="p-2 rounded-xl bg-rose-50 dark:bg-rose-950/60 text-rose-600 dark:text-rose-400">
                <Trash2 className="w-5 h-5" />
              </div>
              <h3 className="text-base font-semibold text-slate-900 dark:text-white">
                {t('env.delete_modal_title', 'Hapus Variabel?')}
              </h3>
            </div>
            <div className="dialog-body ">
              <p className="text-xs text-slate-600 dark:text-slate-300 leading-relaxed mb-5">
                {t('env.delete_confirm_desc', 'Apakah Anda yakin ingin menghapus variabel')}{' '}
                <span className="font-mono font-bold text-slate-900 dark:text-white">{keyToDelete}</span>?{' '}
                {t('env.restart_warning', 'Aplikasi akan memerlukan restart setelah variabel dihapus.')}
              </p>
            </div>
            <div className="flex items-center justify-end gap-3 dialog-footer">
              <button
                onClick={() => setKeyToDelete(null)}
                className="px-4 py-2 rounded-xl border border-slate-300 dark:border-slate-600 hover:bg-slate-50 dark:hover:bg-slate-700/50 text-slate-700 dark:text-slate-200 text-xs font-medium"
              >
                {t('common.cancel', 'Batal')}
              </button>
              <button
                onClick={handleDeleteVariable}
                disabled={isDeleting}
                className="px-4 py-2 rounded-xl bg-rose-600 hover:bg-rose-700 text-white text-xs font-medium flex items-center gap-2 disabled:opacity-50"
              >
                {isDeleting && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                {t('common.delete', 'Hapus')}
              </button>
            </div>
          </div>
        </ModalOverlay>
      )}
    </div>
  );
};
