import { ModalOverlay } from '../ui/ModalOverlay';
import { DatabaseWebAccessButton } from './DatabaseWebAccessButton';
import { DatabaseEnginePicker } from './DatabaseEnginePicker';
import React, { useState, useEffect, useCallback, useMemo, useRef } from 'react';
import {
  Database,
  Plus,
  Trash2,
  KeyRound,
  Globe,
  Download,
  Upload,
  Copy,
  Check,
  Search,
  CheckCircle2,
  AlertCircle,
  Loader2,
  Server,
  ShieldCheck,
  ShieldAlert,
  RefreshCw,
  X,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  fetchDatabases,
  fetchAvailableDatabaseEngines,
  createDatabase,
  deleteDatabase,
  rotateDatabasePassword,
  updateDatabaseRemoteAccess,
  exportDatabase,
  importDatabase,
} from '../../api/client';
import {
  DatabaseItem,
  AvailableDatabaseEngine,
  DatabaseQuotaInfo,
  DatabaseEngine,
  CreateDatabaseRequest,
  RotatePasswordRequest,
  UpdateRemoteAccessRequest,
} from '../../types/api';
import { ConfirmDangerDialog } from '../ui/ConfirmDangerDialog';

export interface SiteDatabaseTabProps {
  siteId?: number;
  siteDomain?: string;
}

export const SiteDatabaseTab: React.FC<SiteDatabaseTabProps> = ({
  siteId,
  siteDomain,
}) => {
  const { i18n } = useTranslation();
  const isEn = i18n.language.startsWith('en');

  const [databases, setDatabases] = useState<DatabaseItem[]>([]);
  const [quota, setQuota] = useState<DatabaseQuotaInfo | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(true);
  const [searchQuery, setSearchQuery] = useState('');
  const [engineFilter, setEngineFilter] = useState<'all' | DatabaseEngine>('all');
  const [toastMessage, setToastMessage] = useState<{
    text: string;
    type: 'success' | 'error';
  } | null>(null);

  // Modals state
  const [isCreateModalOpen, setIsCreateModalOpen] = useState(false);
  const [createEngine, setCreateEngine] = useState('');
  const [engines, setEngines] = useState<AvailableDatabaseEngine[]>([]);
  const [enginesLoading, setEnginesLoading] = useState(true);
  const [enginesError, setEnginesError] = useState('');
  const engineRequest = useRef(0);
  const loadEngines = useCallback(async (quiet = false) => {
    const request = ++engineRequest.current;
    if (!quiet) setEnginesLoading(true);
    setEnginesError('');
    if (!quiet) setEngines([]);
    try {
      const available = await fetchAvailableDatabaseEngines();
      if (engineRequest.current !== request) return;
      setEngines(available);
      setCreateEngine(current => available.some(item => item.engine === current) ? current : available[0]?.engine || '');
    } catch (cause: unknown) {
      if (engineRequest.current === request) {
        setEnginesError(cause instanceof Error ? cause.message : 'Gagal memeriksa mesin basis data.');
        setEngines([]);
        setCreateEngine('');
      }
    } finally {
      if (engineRequest.current === request) setEnginesLoading(false);
    }
  }, []);
  useEffect(() => {
    void loadEngines();
    const onFocus = () => { void loadEngines(true); };
    window.addEventListener('focus', onFocus);
    const timer = window.setInterval(() => { if (document.visibilityState === 'visible') void loadEngines(true); }, 30000);
    return () => { engineRequest.current++; window.removeEventListener('focus', onFocus); window.clearInterval(timer); };
  }, [loadEngines]);
  const openCreate = () => { setIsCreateModalOpen(true); void loadEngines(); };
  const [createDbName, setCreateDbName] = useState('');
  const [createDbUser, setCreateDbUser] = useState('');
  const [createPassword, setCreatePassword] = useState('');
  const [autoGeneratePassword, setAutoGeneratePassword] = useState(true);
  const [createRemoteAccess, setCreateRemoteAccess] = useState(false);
  const [createAllowedHosts, setCreateAllowedHosts] = useState('localhost');
  const [isCreating, setIsCreating] = useState(false);
  const [showCreatedPasswordModal, setShowCreatedPasswordModal] = useState<{
    dbName: string;
    dbUser: string;
    password: string;
  } | null>(null);

  // Rotate Password Modal
  const [rotatingDb, setRotatingDb] = useState<DatabaseItem | null>(null);
  const [rotateNewPassword, setRotateNewPassword] = useState('');
  const [autoRotatePassword, setAutoRotatePassword] = useState(true);
  const [isRotating, setIsRotating] = useState(false);
  const [rotatedPasswordResult, setRotatedPasswordResult] = useState<string | null>(null);

  // Remote Access Modal
  const [remoteDb, setRemoteDb] = useState<DatabaseItem | null>(null);
  const [remoteAccessEnabled, setRemoteAccessEnabled] = useState(false);
  const [remoteAllowedHosts, setRemoteAllowedHosts] = useState('localhost');
  const [isSavingRemote, setIsSavingRemote] = useState(false);

  // Export / Import Modals
  const [exportDb, setExportDb] = useState<DatabaseItem | null>(null);
  const [exportPath, setExportPath] = useState('');
  const [isExporting, setIsExporting] = useState(false);

  const [importDb, setImportDb] = useState<DatabaseItem | null>(null);
  const [importPath, setImportPath] = useState('');
  const [isImporting, setIsImporting] = useState(false);

  // Delete Dialog
  const [deletingDb, setDeletingDb] = useState<DatabaseItem | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  // Copy helper
  const [copiedKey, setCopiedKey] = useState<string | null>(null);

  const handleCopy = (text: string, key: string) => {
    navigator.clipboard.writeText(text);
    setCopiedKey(key);
    setTimeout(() => setCopiedKey(null), 2000);
  };

  const showToast = (text: string, type: 'success' | 'error') => {
    setToastMessage({ text, type });
    setTimeout(() => setToastMessage(null), 4000);
  };

  const loadDatabases = useCallback(async () => {
    setIsLoading(true);
    setLoadError(null);
    try {
      const res = await fetchDatabases(siteId ? { site_id: siteId } : undefined);
      setDatabases(res.databases);
      setQuota(res.quota);
    } catch (err: unknown) {
      setQuota(null);
      setDatabases([]);
      setLoadError(err instanceof Error ? err.message : 'Gagal memuat basis data');
    } finally {
      setIsLoading(false);
    }
  }, [siteId]);

  useEffect(() => {
    loadDatabases();
  }, [loadDatabases]);

  const filteredDatabases = useMemo(() => {
    return databases.filter((db) => {
      const matchesSearch =
        db.name.toLowerCase().includes(searchQuery.toLowerCase()) ||
        db.db_user.toLowerCase().includes(searchQuery.toLowerCase());
      const matchesEngine = engineFilter === 'all' || db.engine === engineFilter;
      return matchesSearch && matchesEngine;
    });
  }, [databases, searchQuery, engineFilter]);

  const formatBytes = (bytes: number) => {
    if (bytes <= 0) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return `${parseFloat((bytes / Math.pow(k, i)).toFixed(2))} ${sizes[i]}`;
  };

  // -------------------------------------------------------------
  // Create Database Handler
  // -------------------------------------------------------------
  const handleCreateDatabase = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!canCreate || isCreating || !engines.some(engine => engine.engine === createEngine) || !createDbName.trim()) return;

    setIsCreating(true);
    try {
      const payload: CreateDatabaseRequest = {
        engine: createEngine,
        name: createDbName.trim(),
        db_user: createDbUser.trim() || undefined,
        password: autoGeneratePassword ? undefined : createPassword || undefined,
        site_id: siteId,
        remote_access: createRemoteAccess,
        allowed_hosts: createRemoteAccess ? createAllowedHosts.trim() : 'localhost',
      };

      const res = await createDatabase(payload);
      showToast(`Basis data '${res.database.name}' berhasil dibuat`, 'success');
      setIsCreateModalOpen(false);

      if (res.generated_password) {
        setShowCreatedPasswordModal({
          dbName: res.database.name,
          dbUser: res.database.db_user,
          password: res.generated_password,
        });
      }

      // Reset form
      setCreateDbName('');
      setCreateDbUser('');
      setCreatePassword('');
      setAutoGeneratePassword(true);
      setCreateRemoteAccess(false);
      setCreateAllowedHosts('localhost');

      await loadDatabases();
    } catch (err: unknown) {
      showToast(err instanceof Error ? err.message : 'Gagal membuat basis data', 'error');
      void loadEngines();
    } finally {
      setIsCreating(false);
    }
  };

  // -------------------------------------------------------------
  // Rotate Password Handler
  // -------------------------------------------------------------
  const handleRotatePassword = async () => {
    if (!rotatingDb) return;
    setIsRotating(true);
    try {
      const payload: RotatePasswordRequest = {
        new_password: autoRotatePassword ? undefined : rotateNewPassword || undefined,
      };
      const res = await rotateDatabasePassword(rotatingDb.id, payload);
      setRotatedPasswordResult(res.new_password);
      showToast(`Kata sandi untuk '${rotatingDb.name}' berhasil diperbarui`, 'success');
    } catch (err: any) {
      showToast(err.message || 'Gagal memperbarui kata sandi', 'error');
    } finally {
      setIsRotating(false);
    }
  };

  // -------------------------------------------------------------
  // Remote Access Handler
  // -------------------------------------------------------------
  const handleSaveRemoteAccess = async () => {
    if (!remoteDb) return;
    setIsSavingRemote(true);
    try {
      const payload: UpdateRemoteAccessRequest = {
        remote_access: remoteAccessEnabled,
        allowed_hosts: remoteAccessEnabled ? remoteAllowedHosts.trim() : 'localhost',
      };
      await updateDatabaseRemoteAccess(remoteDb.id, payload);
      showToast(`Pengaturan akses jarak jauh untuk '${remoteDb.name}' disimpan`, 'success');
      setRemoteDb(null);
      await loadDatabases();
    } catch (err: any) {
      showToast(err.message || 'Gagal menyimpan akses jarak jauh', 'error');
    } finally {
      setIsSavingRemote(false);
    }
  };

  // -------------------------------------------------------------
  // Export Handler
  // -------------------------------------------------------------
  const handleExport = async () => {
    if (!exportDb) return;
    setIsExporting(true);
    try {
      const res = await exportDatabase(exportDb.id, {
        export_file_path: exportPath.trim() || undefined,
      });
      showToast(
        res.message || `Pencadangan basis data '${exportDb.name}' telah dijadwalkan`,
        'success'
      );
      setExportDb(null);
    } catch (err: any) {
      showToast(err.message || 'Gagal menjadwalkan ekspor basis data', 'error');
    } finally {
      setIsExporting(false);
    }
  };

  // -------------------------------------------------------------
  // Import Handler
  // -------------------------------------------------------------
  const handleImport = async () => {
    if (!importDb || !importPath.trim()) return;
    setIsImporting(true);
    try {
      const res = await importDatabase(importDb.id, {
        import_file_path: importPath.trim(),
      });
      showToast(
        res.message || `Pemulihan basis data '${importDb.name}' telah dijadwalkan`,
        'success'
      );
      setImportDb(null);
    } catch (err: any) {
      showToast(err.message || 'Gagal menjadwalkan impor berkas SQL', 'error');
    } finally {
      setIsImporting(false);
    }
  };

  // -------------------------------------------------------------
  // Delete Handler
  // -------------------------------------------------------------
  const handleDelete = async () => {
    if (!deletingDb) return;
    setIsDeleting(true);
    try {
      await deleteDatabase(deletingDb.id);
      showToast(`Basis data '${deletingDb.name}' berhasil dihapus`, 'success');
      setDeletingDb(null);
      await loadDatabases();
    } catch (err: any) {
      showToast(err.message || 'Gagal menghapus basis data', 'error');
    } finally {
      setIsDeleting(false);
    }
  };

  const isQuotaFull = quota !== null && quota.limit > 0 && quota.used >= quota.limit;
  const canCreate = quota !== null && !isLoading && !loadError && !isQuotaFull && !enginesLoading && !enginesError && engines.length > 0;
  const quotaPercentage =
    quota && quota.limit > 0 ? Math.min(100, Math.round((quota.used / quota.limit) * 100)) : 0;

  return (
    <div className="space-y-6">
      {/* Toast Notification */}
      {toastMessage && (
        <div
          role="alert"
          aria-live="polite"
          className={`fixed bottom-5 right-5 z-50 flex items-center gap-3 px-4 py-3 rounded-xl shadow-xl text-sm font-medium transition-all animate-in fade-in slide-in-from-bottom-2 ${
            toastMessage.type === 'success'
              ? 'bg-emerald-600 text-white'
              : 'bg-rose-600 text-white'
          }`}
        >
          {toastMessage.type === 'success' ? (
            <CheckCircle2 className="w-5 h-5 flex-shrink-0" />
          ) : (
            <AlertCircle className="w-5 h-5 flex-shrink-0" />
          )}
          <span>{toastMessage.text}</span>
          <button
            onClick={() => setToastMessage(null)}
            className="ml-2 hover:opacity-75 focus:outline-none"
            aria-label="Tutup notifikasi"
          >
            <X className="w-4 h-4" />
          </button>
        </div>
      )}

      {/* Quota Banner & Header */}
      <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl p-6 shadow-sm">
        <div className="flex flex-col md:flex-row md:items-center justify-between gap-4">
          <div className="space-y-1">
            <div className="flex items-center gap-2">
              <Database className="w-6 h-6 text-indigo-600 dark:text-indigo-400" />
              <h2 className="text-xl font-bold text-slate-900 dark:text-slate-100">
                {isEn ? 'Database Management' : 'Manajemen Basis Data'}
              </h2>
            </div>
            <p className="text-sm text-slate-500 dark:text-slate-400">
              {siteDomain
                ? isEn
                  ? `Isolated databases for ${siteDomain}`
                  : `Basis data terisolasi untuk situs ${siteDomain}`
                : isEn
                ? 'Create, manage, export, and protect your application databases'
                : 'Buat, kelola, ekspor, dan amankan basis data aplikasi Anda'}
            </p>
          </div>

          <button
            onClick={openCreate}
            disabled={!canCreate}
            className={`inline-flex items-center gap-2 px-4 py-2.5 rounded-xl font-medium text-sm transition shadow-sm ${
              isQuotaFull
                ? 'bg-slate-100 dark:bg-slate-800 text-slate-400 dark:text-slate-600 cursor-not-allowed border border-slate-200 dark:border-slate-700'
                : 'bg-indigo-600 hover:bg-indigo-700 active:bg-indigo-800 text-white shadow-indigo-600/20'
            }`}
          >
            <Plus className="w-4 h-4" />
            <span>{isEn ? 'Create Database' : 'Buat Basis Data'}</span>
          </button>
        </div>

        {/* Plan Quota Bar */}
        <div className="mt-6 pt-5 border-t border-slate-100 dark:border-slate-800">
          <div className="flex items-center justify-between text-xs font-semibold mb-2">
            <span className="text-slate-600 dark:text-slate-400">
              {isEn ? 'Database Quota Usage' : 'Penggunaan Kuota Basis Data'}
            </span>
            <span className="text-slate-900 dark:text-slate-200">
              {quota ? <>
                {quota.used} / {quota.limit > 0 ? quota.limit : (isEn ? 'Unlimited' : 'Tanpa batas')}{' '}
                {isEn ? 'Databases' : 'Basis Data'}
                {quota.remaining !== null && <> ({quota.remaining} {isEn ? 'remaining' : 'tersisa'})</>}
              </> : isLoading ? (isEn ? 'Loading quota...' : 'Memuat kuota...') : (isEn ? 'Quota unavailable' : 'Kuota belum tersedia')}
            </span>
          </div>

          <div
            className="w-full bg-slate-100 dark:bg-slate-800 h-2.5 rounded-full overflow-hidden"
            role={quota && quota.limit > 0 ? 'progressbar' : undefined}
            aria-label={isEn ? 'Database quota usage' : 'Penggunaan kuota basis data'}
            aria-valuenow={quota && quota.limit > 0 ? Math.min(quota.used, quota.limit) : undefined}
            aria-valuemin={0}
            aria-valuemax={quota && quota.limit > 0 ? quota.limit : undefined}
          >
            <div
              className={`h-full transition-all duration-300 rounded-full ${
                quotaPercentage >= 100
                  ? 'bg-rose-500'
                  : quotaPercentage >= 80
                  ? 'bg-amber-500'
                  : 'bg-indigo-600'
              }`}
              style={{ width: `${quotaPercentage}%` }}
            />
          </div>

          {isQuotaFull && (
            <div className="mt-3 flex items-center gap-2 text-xs text-amber-700 dark:text-amber-400 bg-amber-50 dark:bg-amber-950/40 border border-amber-200 dark:border-amber-800/60 p-2.5 rounded-xl">
              <ShieldAlert className="w-4 h-4 flex-shrink-0" />
              <span>
                {isEn
                  ? 'Database plan quota has been reached. Upgrade your plan to add more databases.'
                  : 'Batas kuota database paket telah tercapai. Tingkatkan paket Anda untuk menambah database baru.'}
              </span>
            </div>
          )}
        </div>
      </div>

      {/* Filter and Search Bar */}
      <div className="flex flex-col sm:flex-row items-center justify-between gap-3">
        <div className="relative w-full sm:w-72">
          <Search className="w-4 h-4 text-slate-400 absolute left-3 top-1/2 -translate-y-1/2" />
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={isEn ? 'Search databases...' : 'Cari nama basis data...'}
            className="w-full pl-9 pr-4 py-2 bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl text-sm text-slate-900 dark:text-slate-100 placeholder-slate-400 focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500 transition"
          />
        </div>

        <div className="flex items-center gap-2 w-full sm:w-auto">
          <span className="text-xs font-semibold text-slate-500 dark:text-slate-400">
            {isEn ? 'Engine:' : 'Mesin:'}
          </span>
          <div className="flex items-center bg-slate-100 dark:bg-slate-800/70 p-1 rounded-xl border border-slate-200 dark:border-slate-700/60 text-xs font-medium">
            <button
              onClick={() => setEngineFilter('all')}
              className={`px-3 py-1 rounded-lg transition ${
                engineFilter === 'all'
                  ? 'bg-white dark:bg-slate-700 text-slate-900 dark:text-slate-100 shadow-sm font-semibold'
                  : 'text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-slate-200'
              }`}
            >
              Semua
            </button>
            <button
              onClick={() => setEngineFilter('mariadb')}
              className={`px-3 py-1 rounded-lg transition ${
                engineFilter === 'mariadb'
                  ? 'bg-white dark:bg-slate-700 text-slate-900 dark:text-slate-100 shadow-sm font-semibold'
                  : 'text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-slate-200'
              }`}
            >
              MariaDB
            </button>
            <button
              onClick={() => setEngineFilter('postgresql')}
              className={`px-3 py-1 rounded-lg transition ${
                engineFilter === 'postgresql'
                  ? 'bg-white dark:bg-slate-700 text-slate-900 dark:text-slate-100 shadow-sm font-semibold'
                  : 'text-slate-600 dark:text-slate-400 hover:text-slate-900 dark:hover:text-slate-200'
              }`}
            >
              PostgreSQL
            </button>
          </div>

          <button
            onClick={() => { void loadDatabases(); void loadEngines(); }}
            disabled={isLoading}
            className="p-2 border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900 rounded-xl text-slate-600 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-800 transition"
            title="Muat Ulang"
          >
            <RefreshCw className={`w-4 h-4 ${isLoading ? 'animate-spin' : ''}`} />
          </button>
        </div>
      </div>

      {(enginesLoading || enginesError || engines.length === 0) && <div className="rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] p-5">
        <DatabaseEnginePicker engines={engines} selected={createEngine} loading={enginesLoading} error={enginesError} onSelect={setCreateEngine} onRefresh={() => void loadEngines()} />
      </div>}

      {/* Database List */}
      {isLoading ? (
        <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl p-12 flex flex-col items-center justify-center text-center">
          <Loader2 className="w-8 h-8 text-indigo-600 dark:text-indigo-400 animate-spin mb-3" />
          <p className="text-sm text-slate-500 dark:text-slate-400">
            {isEn ? 'Loading database list...' : 'Memuat daftar basis data...'}
          </p>
        </div>
      ) : loadError ? (
        <div role="alert" className="p-6 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] space-y-3">
          <h3 className="font-semibold">{isEn ? 'Database list unavailable' : 'Daftar basis data belum tersedia'}</h3>
          <p className="text-sm text-[var(--ink-muted)] break-words">{loadError}</p>
          <button type="button" onClick={loadDatabases} className="project-button">{isEn ? 'Try again' : 'Coba lagi'}</button>
        </div>
      ) : filteredDatabases.length === 0 ? (
        <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl p-12 flex flex-col items-center justify-center text-center">
          <Database className="w-12 h-12 text-slate-300 dark:text-slate-600 mb-3" />
          <h3 className="text-base font-semibold text-slate-900 dark:text-slate-100">
            {isEn ? 'No databases found' : 'Belum ada basis data'}
          </h3>
          <p className="text-sm text-slate-500 dark:text-slate-400 max-w-sm mt-1 mb-5">
            {searchQuery
              ? isEn
                ? 'No databases match your search query.'
                : 'Tidak ada basis data yang cocok dengan kueri pencarian.'
              : isEn
              ? 'Create a database to store and organize your application data securely.'
              : 'Buat basis data baru untuk menghubungkan aplikasi Anda ke MySQL/PostgreSQL.'}
          </p>
          {!searchQuery && canCreate && (
            <button
              onClick={openCreate}
              className="inline-flex items-center gap-2 px-4 py-2 bg-indigo-600 hover:bg-indigo-700 text-white rounded-xl text-sm font-medium transition"
            >
              <Plus className="w-4 h-4" />
              <span>{isEn ? 'Create Database' : 'Buat Basis Data Pertama'}</span>
            </button>
          )}
        </div>
      ) : (
        <div className="grid grid-cols-1 gap-4">
          {filteredDatabases.map((db) => {
            const isMaria = db.engine === 'mariadb';
            return (
              <div
                key={db.id}
                className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl p-5 shadow-sm hover:border-slate-300 dark:hover:border-slate-700 transition"
              >
                <div className="flex flex-col lg:flex-row lg:items-center justify-between gap-4">
                  {/* Left Column: Name & Metadata */}
                  <div className="space-y-2">
                    <div className="flex items-center gap-3 flex-wrap">
                      <span className="font-bold text-base text-slate-900 dark:text-slate-100">
                        {db.name}
                      </span>
                      <button
                        onClick={() => handleCopy(db.name, `name-${db.id}`)}
                        className="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 transition"
                        title="Salin Nama Database"
                      >
                        {copiedKey === `name-${db.id}` ? (
                          <Check className="w-4 h-4 text-emerald-500" />
                        ) : (
                          <Copy className="w-4 h-4" />
                        )}
                      </button>

                      {/* Engine Badge */}
                      <span
                        className={`inline-flex items-center gap-1.5 px-2.5 py-0.5 rounded-full text-xs font-semibold ${
                          isMaria
                            ? 'bg-sky-50 dark:bg-sky-950/60 text-sky-700 dark:text-sky-300 border border-sky-200 dark:border-sky-800'
                            : 'bg-indigo-50 dark:bg-indigo-950/60 text-indigo-700 dark:text-indigo-300 border border-indigo-200 dark:border-indigo-800'
                        }`}
                      >
                        <Server className="w-3 h-3" />
                        {isMaria ? 'MariaDB' : 'PostgreSQL'}
                      </span>

                      {/* Remote Access Badge */}
                      {db.remote_access ? (
                        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-amber-50 dark:bg-amber-950/50 text-amber-700 dark:text-amber-300 border border-amber-200 dark:border-amber-800">
                          <Globe className="w-3 h-3" />
                          <span>Remote ({db.allowed_hosts})</span>
                        </span>
                      ) : (
                        <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded-full text-xs font-medium bg-emerald-50 dark:bg-emerald-950/50 text-emerald-700 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800">
                          <ShieldCheck className="w-3 h-3" />
                          <span>Localhost</span>
                        </span>
                      )}
                    </div>

                    {/* Metadata details */}
                    <div className="flex flex-wrap items-center gap-x-5 gap-y-1 text-xs text-slate-500 dark:text-slate-400">
                      <div>
                        <span className="text-slate-400 dark:text-slate-500">
                          Pengguna DB:{' '}
                        </span>
                        <span className="font-mono font-medium text-slate-800 dark:text-slate-200">
                          {db.db_user}
                        </span>
                        <button
                          onClick={() => handleCopy(db.db_user, `user-${db.id}`)}
                          className="ml-1.5 text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 inline-block align-middle"
                          title="Salin Nama Pengguna"
                        >
                          {copiedKey === `user-${db.id}` ? (
                            <Check className="w-3 h-3 text-emerald-500 inline" />
                          ) : (
                            <Copy className="w-3 h-3 inline" />
                          )}
                        </button>
                      </div>

                      <div>
                        <span className="text-slate-400 dark:text-slate-500">
                          Host:{' '}
                        </span>
                        <span className="font-mono font-medium text-slate-800 dark:text-slate-200">
                          127.0.0.1:{isMaria ? '3306' : '5432'}
                        </span>
                      </div>

                      <div>
                        <span className="text-slate-400 dark:text-slate-500">
                          Ukuran:{' '}
                        </span>
                        <span className="font-medium text-slate-700 dark:text-slate-300">
                          {formatBytes(db.size_bytes)}
                        </span>
                      </div>

                      <div>
                        <span className="text-slate-400 dark:text-slate-500">
                          Collation:{' '}
                        </span>
                        <span className="font-mono text-slate-700 dark:text-slate-300">
                          {db.collation}
                        </span>
                      </div>
                    </div>
                  </div>

                  {/* Right Column: Actions */}
                  <div className="flex flex-wrap items-center gap-2 pt-2 lg:pt-0 border-t lg:border-t-0 border-slate-100 dark:border-slate-800">
                    <DatabaseWebAccessButton id={db.id} name={db.name} engine={db.engine} />
                    <button
                      onClick={() => {
                        setRotatingDb(db);
                        setRotateNewPassword('');
                        setAutoRotatePassword(true);
                        setRotatedPasswordResult(null);
                      }}
                      className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-700 dark:text-slate-200 hover:bg-slate-50 dark:hover:bg-slate-750 transition"
                      title="Ubah atau acak kata sandi database"
                    >
                      <KeyRound className="w-3.5 h-3.5 text-slate-400" />
                      <span>Kata Sandi</span>
                    </button>

                    <button
                      onClick={() => {
                        setRemoteDb(db);
                        setRemoteAccessEnabled(db.remote_access);
                        setRemoteAllowedHosts(db.allowed_hosts || 'localhost');
                      }}
                      className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-700 dark:text-slate-200 hover:bg-slate-50 dark:hover:bg-slate-750 transition"
                      title="Atur akses jarak jauh dan daftar host"
                    >
                      <Globe className="w-3.5 h-3.5 text-slate-400" />
                      <span>Akses Remote</span>
                    </button>

                    <button
                      onClick={() => {
                        setExportDb(db);
                        setExportPath(
                          `/home/${db.db_user}/backups/${db.name}_backup_${new Date()
                            .toISOString()
                            .slice(0, 10)}.sql`
                        );
                      }}
                      className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-700 dark:text-slate-200 hover:bg-slate-50 dark:hover:bg-slate-750 transition"
                      title="Ekspor berkas SQL dump"
                    >
                      <Download className="w-3.5 h-3.5 text-slate-400" />
                      <span>Ekspor</span>
                    </button>

                    <button
                      onClick={() => {
                        setImportDb(db);
                        setImportPath(`/home/${db.db_user}/backups/dump.sql`);
                      }}
                      className="inline-flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-xl border border-slate-200 dark:border-slate-700 bg-white dark:bg-slate-800 text-slate-700 dark:text-slate-200 hover:bg-slate-50 dark:hover:bg-slate-750 transition"
                      title="Impor berkas SQL dump"
                    >
                      <Upload className="w-3.5 h-3.5 text-slate-400" />
                      <span>Impor</span>
                    </button>

                    <button
                      onClick={() => setDeletingDb(db)}
                      className="p-1.5 text-slate-400 hover:text-rose-600 dark:hover:text-rose-400 hover:bg-rose-50 dark:hover:bg-rose-950/40 rounded-xl transition"
                      title="Hapus Basis Data"
                    >
                      <Trash2 className="w-4 h-4" />
                    </button>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* ========================================================= */}
      {/* 1. Modal: Buat Basis Data                                 */}
      {/* ========================================================= */}
      {isCreateModalOpen && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-sm animate-in fade-in"
          onDismiss={() => setIsCreateModalOpen(false)}
        >
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl max-w-lg w-full p-6 shadow-2xl space-y-5 dialog-panel">
            <div className="flex items-center justify-between pb-3 border-b border-slate-100 dark:border-slate-800 dialog-header">
              <div className="flex items-center gap-2.5">
                <div className="p-2 bg-indigo-50 dark:bg-indigo-950/60 text-indigo-600 dark:text-indigo-400 rounded-xl">
                  <Database className="w-5 h-5" />
                </div>
                <h3 className="text-lg font-bold text-slate-900 dark:text-slate-100">
                  {isEn ? 'Create New Database' : 'Buat Basis Data Baru'}
                </h3>
              </div>
              <button
                onClick={() => setIsCreateModalOpen(false)}
                className="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 p-1"
              >
                <X className="w-5 h-5" />
              </button>
            </div>

            <form onSubmit={handleCreateDatabase} className="dialog-form">
              <div className="dialog-body space-y-4">
                <DatabaseEnginePicker engines={engines} selected={createEngine} loading={enginesLoading} error={enginesError} busy={isCreating} onSelect={setCreateEngine} onRefresh={() => void loadEngines()} />

                {/* Database Name */}
                <div>
                  <label className="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
                    Nama Basis Data
                  </label>
                  <input
                    type="text"
                    required
                    value={createDbName}
                    onChange={(e) => setCreateDbName(e.target.value)}
                    placeholder="contoh: wordpress"
                    className="w-full px-3.5 py-2.5 bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded-xl text-sm text-slate-900 dark:text-slate-100 placeholder-slate-400 focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500 font-mono transition"
                  />
                  <p className="mt-1 text-xs text-slate-500 dark:text-slate-400">
                    Prefix tenant akan ditambahkan secara otomatis demi keamanan isolasi.
                  </p>
                </div>

                {/* Database User */}
                <div>
                  <label className="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
                    Nama Pengguna DB (Opsional)
                  </label>
                  <input
                    type="text"
                    value={createDbUser}
                    onChange={(e) => setCreateDbUser(e.target.value)}
                    placeholder="Kosongkan untuk sama dengan nama database"
                    className="w-full px-3.5 py-2.5 bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded-xl text-sm text-slate-900 dark:text-slate-100 placeholder-slate-400 focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500 font-mono transition"
                  />
                </div>

                {/* Password generation */}
                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    <label className="text-xs font-semibold text-slate-700 dark:text-slate-300">Kata Sandi</label>
                    <label className="inline-flex items-center gap-1.5 text-xs text-indigo-600 dark:text-indigo-400 cursor-pointer">
                      <input
                        type="checkbox"
                        checked={autoGeneratePassword}
                        onChange={(e) => setAutoGeneratePassword(e.target.checked)}
                        className="rounded border-slate-300 text-indigo-600 focus:ring-indigo-500"
                      />
                      <span>Acak kata sandi kuat otomatis</span>
                    </label>
                  </div>

                  {!autoGeneratePassword && (
                    <input
                      type="password"
                      required
                      value={createPassword}
                      onChange={(e) => setCreatePassword(e.target.value)}
                      placeholder="Minimal 12 karakter aman..."
                      className="w-full px-3.5 py-2.5 bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded-xl text-sm text-slate-900 dark:text-slate-100 placeholder-slate-400 focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500 font-mono transition"
                    />
                  )}
                </div>

                {/* Remote Access Toggle */}
                <div className="pt-2 border-t border-slate-100 dark:border-slate-800 space-y-2">
                  <label className="flex items-center justify-between cursor-pointer">
                    <div>
                      <div className="text-xs font-semibold text-slate-700 dark:text-slate-300">
                        Izinkan Akses Jarak Jauh (Remote Access)
                      </div>
                      <div className="text-xs text-slate-500 dark:text-slate-400">
                        Secara default hanya localhost yang diizinkan demi keamanan.
                      </div>
                    </div>
                    <input
                      type="checkbox"
                      checked={createRemoteAccess}
                      onChange={(e) => setCreateRemoteAccess(e.target.checked)}
                      className="w-4 h-4 rounded border-slate-300 text-indigo-600 focus:ring-indigo-500"
                    />
                  </label>

                  {createRemoteAccess && (
                    <div>
                      <label className="block text-xs font-medium text-slate-600 dark:text-slate-400 mb-1">
                        Host atau IP yang Diizinkan
                      </label>
                      <input
                        type="text"
                        value={createAllowedHosts}
                        onChange={(e) => setCreateAllowedHosts(e.target.value)}
                        placeholder="contoh: % atau 192.168.1.50 atau 10.0.0.0/24"
                        className="w-full px-3 py-2 bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded-xl text-xs text-slate-900 dark:text-slate-100 font-mono focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500"
                      />
                    </div>
                  )}
                </div>

                {/* Action Buttons */}
              </div>
              <div className="pt-3 flex items-center justify-end gap-3 border-t border-slate-100 dark:border-slate-800 dialog-footer">
                <button
                  type="button"
                  onClick={() => setIsCreateModalOpen(false)}
                  className="px-4 py-2 text-sm font-medium text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 rounded-xl transition"
                >
                  Batal
                </button>
                <button
                  type="submit"
                  disabled={isCreating || !canCreate}
                  className="inline-flex items-center gap-2 px-5 py-2 text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 active:bg-indigo-800 rounded-xl shadow-sm transition disabled:opacity-50"
                >
                  {isCreating ? <Loader2 className="w-4 h-4 animate-spin" /> : <Plus className="w-4 h-4" />}
                  <span>Buat Basis Data</span>
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* ========================================================= */}
      {/* 2. Modal: Kata Sandi Dibuat (One-Time Display)            */}
      {/* ========================================================= */}
      {showCreatedPasswordModal && (
        <ModalOverlay className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-sm animate-in fade-in">
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl max-w-md w-full p-6 shadow-2xl space-y-4 dialog-panel">
            <div className="flex items-center gap-3 text-emerald-600 dark:text-emerald-400 dialog-header">
              <CheckCircle2 className="w-6 h-6 flex-shrink-0" />
              <h3 className="text-lg font-bold text-slate-900 dark:text-slate-100">Basis Data Berhasil Dibuat</h3>
            </div>
            <div className="dialog-body space-y-4">
              <p className="text-sm text-slate-600 dark:text-slate-300">
                Simpan kata sandi ini sekarang. Demi keamanan, kata sandi tidak disimpan dalam bentuk teks biasa.
              </p>

              <div className="bg-slate-50 dark:bg-slate-800/80 border border-slate-200 dark:border-slate-700 p-3.5 rounded-xl space-y-2 text-xs">
                <div>
                  <span className="text-slate-500 dark:text-slate-400">Database: </span>
                  <span className="font-mono font-bold text-slate-900 dark:text-slate-100">
                    {showCreatedPasswordModal.dbName}
                  </span>
                </div>
                <div>
                  <span className="text-slate-500 dark:text-slate-400">Pengguna: </span>
                  <span className="font-mono font-bold text-slate-900 dark:text-slate-100">
                    {showCreatedPasswordModal.dbUser}
                  </span>
                </div>
                <div className="pt-2 border-t border-slate-200 dark:border-slate-700 flex items-center justify-between">
                  <span className="font-mono text-sm font-bold text-indigo-600 dark:text-indigo-400 select-all">
                    {showCreatedPasswordModal.password}
                  </span>
                  <button
                    onClick={() => handleCopy(showCreatedPasswordModal.password, 'created-pass')}
                    className="p-1.5 bg-white dark:bg-slate-700 border border-slate-200 dark:border-slate-600 rounded-lg text-slate-600 dark:text-slate-200 hover:bg-slate-50 transition"
                    title="Salin Kata Sandi"
                  >
                    {copiedKey === 'created-pass' ? (
                      <Check className="w-4 h-4 text-emerald-500" />
                    ) : (
                      <Copy className="w-4 h-4" />
                    )}
                  </button>
                </div>
              </div>
            </div>
            <div className="pt-2 flex justify-end dialog-footer">
              <button
                onClick={() => setShowCreatedPasswordModal(null)}
                className="px-5 py-2 text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 rounded-xl transition shadow-sm"
              >
                Saya Sudah Menyimpan
              </button>
            </div>
          </div>
        </ModalOverlay>
      )}

      {/* ========================================================= */}
      {/* 3. Modal: Rotasi Kata Sandi                               */}
      {/* ========================================================= */}
      {rotatingDb && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-sm animate-in fade-in"
          onDismiss={() => setRotatingDb(null)}
        >
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl max-w-md w-full p-6 shadow-2xl space-y-4 dialog-panel">
            <div className="flex items-center justify-between pb-3 border-b border-slate-100 dark:border-slate-800 dialog-header">
              <div className="flex items-center gap-2.5">
                <div className="p-2 bg-indigo-50 dark:bg-indigo-950/60 text-indigo-600 dark:text-indigo-400 rounded-xl">
                  <KeyRound className="w-5 h-5" />
                </div>
                <h3 className="text-lg font-bold text-slate-900 dark:text-slate-100">Rotasi Kata Sandi</h3>
              </div>
              <button
                onClick={() => setRotatingDb(null)}
                className="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200"
              >
                <X className="w-5 h-5" />
              </button>
            </div>
            <div className="dialog-body space-y-4">
              <p className="text-sm text-slate-600 dark:text-slate-300">
                Ubah kata sandi untuk akun pengguna{' '}
                <strong className="font-mono text-slate-900 dark:text-slate-100">{rotatingDb.db_user}</strong> pada
                basis data <strong className="font-mono text-slate-900 dark:text-slate-100">{rotatingDb.name}</strong>
                .
              </p>

              {rotatedPasswordResult ? (
                <div className="p-4 bg-emerald-50 dark:bg-emerald-950/40 border border-emerald-200 dark:border-emerald-800 rounded-xl space-y-2">
                  <div className="text-xs font-semibold text-emerald-800 dark:text-emerald-300">
                    Kata sandi baru berhasil dibuat:
                  </div>
                  <div className="flex items-center justify-between bg-white dark:bg-slate-900 p-2.5 rounded-lg border border-emerald-200 dark:border-emerald-800">
                    <span className="font-mono text-sm font-bold text-slate-900 dark:text-slate-100 select-all">
                      {rotatedPasswordResult}
                    </span>
                    <button
                      onClick={() => handleCopy(rotatedPasswordResult, 'rotated-pass')}
                      className="p-1 hover:text-slate-900 dark:hover:text-slate-100"
                      title="Salin Kata Sandi"
                    >
                      {copiedKey === 'rotated-pass' ? (
                        <Check className="w-4 h-4 text-emerald-500" />
                      ) : (
                        <Copy className="w-4 h-4" />
                      )}
                    </button>
                  </div>
                  <div className="pt-2 flex justify-end">
                    <button
                      onClick={() => setRotatingDb(null)}
                      className="px-4 py-1.5 text-xs font-medium text-white bg-indigo-600 hover:bg-indigo-700 rounded-xl transition"
                    >
                      Selesai
                    </button>
                  </div>
                </div>
              ) : (
                <div className="space-y-4">
                  <label className="flex items-center gap-2 text-xs font-medium text-slate-700 dark:text-slate-300 cursor-pointer">
                    <input
                      type="checkbox"
                      checked={autoRotatePassword}
                      onChange={(e) => setAutoRotatePassword(e.target.checked)}
                      className="rounded border-slate-300 text-indigo-600 focus:ring-indigo-500"
                    />
                    <span>Acak kata sandi kuat secara otomatis (Rekomendasi)</span>
                  </label>

                  {!autoRotatePassword && (
                    <div>
                      <label className="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
                        Kata Sandi Baru
                      </label>
                      <input
                        type="password"
                        value={rotateNewPassword}
                        onChange={(e) => setRotateNewPassword(e.target.value)}
                        placeholder="Minimal 12 karakter aman..."
                        className="w-full px-3.5 py-2.5 bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded-xl text-sm font-mono focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500"
                      />
                    </div>
                  )}

                  <div className="pt-3 flex justify-end gap-3 border-t border-slate-100 dark:border-slate-800">
                    <button
                      onClick={() => setRotatingDb(null)}
                      className="px-4 py-2 text-sm font-medium text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 rounded-xl transition"
                    >
                      Batal
                    </button>
                    <button
                      onClick={handleRotatePassword}
                      disabled={isRotating}
                      className="inline-flex items-center gap-2 px-5 py-2 text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 rounded-xl transition disabled:opacity-50"
                    >
                      {isRotating && <Loader2 className="w-4 h-4 animate-spin" />}
                      <span>Terapkan Kata Sandi</span>
                    </button>
                  </div>
                </div>
              )}
            </div>
          </div>
        </ModalOverlay>
      )}

      {/* ========================================================= */}
      {/* 4. Modal: Akses Jarak Jauh                                */}
      {/* ========================================================= */}
      {remoteDb && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-sm animate-in fade-in"
          onDismiss={() => setRemoteDb(null)}
        >
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl max-w-md w-full p-6 shadow-2xl space-y-4 dialog-panel">
            <div className="flex items-center justify-between pb-3 border-b border-slate-100 dark:border-slate-800 dialog-header">
              <div className="flex items-center gap-2.5">
                <div className="p-2 bg-indigo-50 dark:bg-indigo-950/60 text-indigo-600 dark:text-indigo-400 rounded-xl">
                  <Globe className="w-5 h-5" />
                </div>
                <h3 className="text-lg font-bold text-slate-900 dark:text-slate-100">Konfigurasi Akses Jarak Jauh</h3>
              </div>
              <button
                onClick={() => setRemoteDb(null)}
                className="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200"
              >
                <X className="w-5 h-5" />
              </button>
            </div>
            <div className="dialog-body space-y-4">
              <p className="text-sm text-slate-600 dark:text-slate-300">
                Atur hak akses jaringan untuk basis data{' '}
                <strong className="font-mono text-slate-900 dark:text-slate-100">{remoteDb.name}</strong>.
              </p>
            </div>
            <div className="space-y-4 dialog-footer">
              <label className="flex items-center justify-between p-3.5 bg-slate-50 dark:bg-slate-800/60 border border-slate-200 dark:border-slate-700 rounded-xl cursor-pointer">
                <div>
                  <div className="text-sm font-semibold text-slate-900 dark:text-slate-100">
                    Akses Remote Diaktifkan
                  </div>
                  <div className="text-xs text-slate-500 dark:text-slate-400">
                    Bila nonaktif, hanya koneksi dari server lokal yang diizinkan.
                  </div>
                </div>
                <input
                  type="checkbox"
                  checked={remoteAccessEnabled}
                  onChange={(e) => setRemoteAccessEnabled(e.target.checked)}
                  className="w-5 h-5 rounded border-slate-300 text-indigo-600 focus:ring-indigo-500"
                />
              </label>

              {remoteAccessEnabled && (
                <div>
                  <label className="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
                    Host atau Alamat IP yang Diizinkan
                  </label>
                  <input
                    type="text"
                    value={remoteAllowedHosts}
                    onChange={(e) => setRemoteAllowedHosts(e.target.value)}
                    placeholder="contoh: % atau 192.168.1.100 atau 10.0.0.0/24"
                    className="w-full px-3.5 py-2.5 bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded-xl text-sm font-mono text-slate-900 dark:text-slate-100 focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500"
                  />
                  <p className="mt-1.5 text-xs text-slate-500 dark:text-slate-400">
                    Gunakan <code className="font-mono">%</code> untuk mengizinkan semua IP, atau pisahkan dengan
                    tanda koma untuk beberapa IP/subnet.
                  </p>
                </div>
              )}

              <div className="pt-3 flex justify-end gap-3 border-t border-slate-100 dark:border-slate-800">
                <button
                  onClick={() => setRemoteDb(null)}
                  className="px-4 py-2 text-sm font-medium text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 rounded-xl transition"
                >
                  Batal
                </button>
                <button
                  onClick={handleSaveRemoteAccess}
                  disabled={isSavingRemote}
                  className="inline-flex items-center gap-2 px-5 py-2 text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 rounded-xl transition disabled:opacity-50"
                >
                  {isSavingRemote && <Loader2 className="w-4 h-4 animate-spin" />}
                  <span>Simpan Pengaturan</span>
                </button>
              </div>
            </div>
          </div>
        </ModalOverlay>
      )}

      {/* ========================================================= */}
      {/* 5. Modal: Ekspor Dump SQL                                 */}
      {/* ========================================================= */}
      {exportDb && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-sm animate-in fade-in"
          onDismiss={() => setExportDb(null)}
        >
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl max-w-md w-full p-6 shadow-2xl space-y-4 dialog-panel">
            <div className="flex items-center justify-between pb-3 border-b border-slate-100 dark:border-slate-800 dialog-header">
              <div className="flex items-center gap-2.5">
                <div className="p-2 bg-indigo-50 dark:bg-indigo-950/60 text-indigo-600 dark:text-indigo-400 rounded-xl">
                  <Download className="w-5 h-5" />
                </div>
                <h3 className="text-lg font-bold text-slate-900 dark:text-slate-100">Ekspor Basis Data</h3>
              </div>
              <button
                onClick={() => setExportDb(null)}
                className="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200"
              >
                <X className="w-5 h-5" />
              </button>
            </div>
            <div className="dialog-body space-y-4">
              <p className="text-sm text-slate-600 dark:text-slate-300">
                Proses ekspor akan dijalankan di latar belakang melalui Job Runner dan menghasilkan salinan dump SQL.
              </p>

              <div>
                <label className="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
                  Lokasi Berkas Ekspor (.sql)
                </label>
                <input
                  type="text"
                  value={exportPath}
                  onChange={(e) => setExportPath(e.target.value)}
                  className="w-full px-3.5 py-2.5 bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded-xl text-xs font-mono text-slate-900 dark:text-slate-100 focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500"
                />
              </div>
            </div>
            <div className="pt-3 flex justify-end gap-3 border-t border-slate-100 dark:border-slate-800 dialog-footer">
              <button
                onClick={() => setExportDb(null)}
                className="px-4 py-2 text-sm font-medium text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 rounded-xl transition"
              >
                Batal
              </button>
              <button
                onClick={handleExport}
                disabled={isExporting}
                className="inline-flex items-center gap-2 px-5 py-2 text-sm font-medium text-white bg-indigo-600 hover:bg-indigo-700 rounded-xl transition disabled:opacity-50"
              >
                {isExporting && <Loader2 className="w-4 h-4 animate-spin" />}
                <span>Mulai Ekspor</span>
              </button>
            </div>
          </div>
        </ModalOverlay>
      )}

      {/* ========================================================= */}
      {/* 6. Modal: Impor Berkas SQL                                */}
      {/* ========================================================= */}
      {importDb && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-sm animate-in fade-in"
          onDismiss={() => setImportDb(null)}
        >
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl max-w-md w-full p-6 shadow-2xl space-y-4 dialog-panel">
            <div className="flex items-center justify-between pb-3 border-b border-slate-100 dark:border-slate-800 dialog-header">
              <div className="flex items-center gap-2.5">
                <div className="p-2 bg-indigo-50 dark:bg-indigo-950/60 text-indigo-600 dark:text-indigo-400 rounded-xl">
                  <Upload className="w-5 h-5" />
                </div>
                <h3 className="text-lg font-bold text-slate-900 dark:text-slate-100">Impor Berkas SQL</h3>
              </div>
              <button
                onClick={() => setImportDb(null)}
                className="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200"
              >
                <X className="w-5 h-5" />
              </button>
            </div>
            <div className="dialog-body space-y-4">
              <div className="p-3 bg-amber-50 dark:bg-amber-950/40 border border-amber-200 dark:border-amber-800/60 rounded-xl text-xs text-amber-800 dark:text-amber-300 flex items-start gap-2">
                <AlertCircle className="w-4 h-4 flex-shrink-0 mt-0.5" />
                <span>
                  Peringatan: Impor dump SQL akan menimpa dan memodifikasi tabel pada basis data{' '}
                  <strong className="font-mono">{importDb.name}</strong>. Pastikan Anda telah membuat cadangan
                  terlebih dahulu.
                </span>
              </div>

              <div>
                <label className="block text-xs font-semibold text-slate-700 dark:text-slate-300 mb-1">
                  Jalur Berkas SQL di Direktori Anda
                </label>
                <input
                  type="text"
                  required
                  value={importPath}
                  onChange={(e) => setImportPath(e.target.value)}
                  placeholder="/home/username/backups/dump.sql"
                  className="w-full px-3.5 py-2.5 bg-white dark:bg-slate-800 border border-slate-200 dark:border-slate-700 rounded-xl text-xs font-mono text-slate-900 dark:text-slate-100 focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500"
                />
              </div>
            </div>
            <div className="pt-3 flex justify-end gap-3 border-t border-slate-100 dark:border-slate-800 dialog-footer">
              <button
                onClick={() => setImportDb(null)}
                className="px-4 py-2 text-sm font-medium text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 rounded-xl transition"
              >
                Batal
              </button>
              <button
                onClick={handleImport}
                disabled={isImporting || !importPath.trim()}
                className="inline-flex items-center gap-2 px-5 py-2 text-sm font-medium text-white bg-amber-600 hover:bg-amber-700 rounded-xl transition disabled:opacity-50"
              >
                {isImporting && <Loader2 className="w-4 h-4 animate-spin" />}
                <span>Jadwalkan Impor</span>
              </button>
            </div>
          </div>
        </ModalOverlay>
      )}

      {/* ========================================================= */}
      {/* 7. Dialog Konfirmasi Hapus                                */}
      {/* ========================================================= */}
      {deletingDb && (
        <ConfirmDangerDialog
          isOpen={!!deletingDb}
          title={isEn ? 'Delete Database' : 'Hapus Basis Data'}
          description={
            isEn
              ? `Are you sure you want to delete database '${deletingDb.name}' and user '${deletingDb.db_user}'? All data will be permanently destroyed.`
              : `Apakah Anda yakin ingin menghapus basis data '${deletingDb.name}' beserta pengguna '${deletingDb.db_user}'? Seluruh data tabel akan dihapus secara permanen.`
          }
          confirmMatchText={deletingDb.name}
          confirmButtonLabel={isEn ? 'Delete Database' : 'Hapus Basis Data'}
          onConfirm={handleDelete}
          onClose={() => setDeletingDb(null)}
          isLoading={isDeleting}
        />
      )}
    </div>
  );
};
