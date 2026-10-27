import React, { useState, useEffect, useCallback, useRef } from 'react';
import {
  Database,
  Server,
  RefreshCw,
  CheckCircle2,
  Download,
  Power,
  RotateCw,
  ShieldCheck,
  Loader2,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  fetchDatabaseServers,
  installDatabaseServer,
  actionDatabaseServer,
  fetchJobDetail,
} from '../../api/client';
import { DatabaseServer, DatabaseEngine, JobItem } from '../../types/api';
import { toast } from 'sonner';
import { useRuntimeJobs } from './useRuntimeJobs';
import { DatabaseWebToolsCard } from './DatabaseWebToolsCard';
import { RuntimeJobProgress } from './RuntimeJobProgress';

const databaseJobs = new Set(['database.server.install']);
const jobEngine = (payload: string): string | undefined => {
  try { const value: unknown = JSON.parse(payload); return value && typeof value === 'object' && 'engine' in value && typeof value.engine === 'string' ? value.engine : undefined; }
  catch { return undefined; }
};

export const DatabaseServersTab: React.FC = () => {
  const { i18n } = useTranslation();
  const isEn = i18n.language.startsWith('en');

  const [servers, setServers] = useState<DatabaseServer[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [processingEngine, setProcessingEngine] = useState<DatabaseEngine | null>(null);
  const submitting = useRef(new Set<DatabaseEngine>());
  const [submittingEngines, setSubmittingEngines] = useState(new Set<DatabaseEngine>());
  const [serverError, setServerError] = useState('');

  const loadServers = useCallback(async (quiet = false) => {
    if (!quiet) setIsLoading(true);
    try {
      const data = await fetchDatabaseServers();
      setServers(data);
      setServerError('');
      return data;
    } catch (err: any) {
      if (!quiet) toast.error(err.message || 'Gagal memuat status server basis data');
      setServerError(err.message || 'Gagal memuat status server basis data');
      return null;
    } finally {
      if (!quiet) setIsLoading(false);
    }
  }, []);

  const onFinished = useCallback(async (job: JobItem) => {
    const inventory = await loadServers(true);
    if (!inventory) return false;
    return job.status !== 'succeeded' || inventory.some(server => server.engine === jobEngine(job.payload_json) && server.is_installed);
  }, [loadServers]);
  const installs = useRuntimeJobs(onFinished, databaseJobs);
  const jobsUnavailable = installs.loading || Boolean(installs.error) || Boolean(serverError);

  useEffect(() => {
    loadServers();
  }, [loadServers]);

  const handleInstall = async (engine: DatabaseEngine) => {
    if (submitting.current.has(engine) || jobsUnavailable || installs.jobs.some(job => jobEngine(job.payload_json) === engine)) return;
    submitting.current.add(engine);
    setSubmittingEngines(new Set(submitting.current));
    try {
      const res = await installDatabaseServer(engine);
      if (!res.job_id) throw new Error('Server tidak mengembalikan ID tugas instalasi. Muat ulang daftar tugas sebelum mencoba kembali.');
      const now = Math.floor(Date.now() / 1000);
      installs.track({
        id: res.job_id, user_id: null, name: 'database.server.install', status: 'queued', progress: 0,
        payload_json: JSON.stringify({ engine }), result_json: null, error_message: null,
        retry_count: 0, max_retries: 1, created_at: now, updated_at: now,
      });
      void fetchJobDetail(res.job_id).then(installs.track).catch(() => { /* Keep the accepted job blocked; the tracker retries. */ });
      toast.success(
        res.message || `Instalasi ${engine === 'mariadb' ? 'MariaDB' : 'PostgreSQL'} dijadwalkan`
      );
    } catch (err: any) {
      toast.error(err.message || `Gagal menjadwalkan instalasi ${engine}`);
    } finally {
      submitting.current.delete(engine);
      setSubmittingEngines(new Set(submitting.current));
    }
  };

  const handleAction = async (
    engine: DatabaseEngine,
    action: 'start' | 'stop' | 'restart'
  ) => {
    setProcessingEngine(engine);
    try {
      const res = await actionDatabaseServer(engine, action);
      toast.success(res.message || `Aksi ${action} pada ${engine} berhasil`);
      await loadServers();
    } catch (err: any) {
      toast.error(err.message || `Gagal menjalankan aksi ${action} pada ${engine}`);
    } finally {
      setProcessingEngine(null);
    }
  };

  return (
    <div className="space-y-6">
      <DatabaseWebToolsCard />
      {/* Overview Notice */}
      <div className="p-4 rounded-xl border border-indigo-500/20 bg-indigo-500/5 dark:bg-indigo-950/20 flex items-start gap-3">
        <ShieldCheck className="w-5 h-5 text-indigo-600 dark:text-indigo-400 shrink-0 mt-0.5" />
        <div className="text-xs space-y-1">
          <p className="font-semibold text-slate-900 dark:text-slate-100">
            {isEn
              ? 'Multi-Tenant Database Engine Architecture'
              : 'Arsitektur Mesin Basis Data Multi-Tenant'}
          </p>
          <p className="text-slate-600 dark:text-slate-400 leading-relaxed">
            {isEn
              ? 'MariaDB and PostgreSQL servers are provisioned on demand. Each tenant database and user are isolated with strict quotas, prefix validation, and localhost-only access by default.'
              : 'Server MariaDB dan PostgreSQL dapat dipasang sesuai kebutuhan (on-demand). Seluruh database dan user tenant terisolasi dengan prefix otomatis, kuota paket, dan default hanya localhost demi keamanan.'}
          </p>
        </div>
      </div>

      {/* Header bar */}
      <div className="flex items-center justify-between">
        <div>
          <h3 className="text-base font-bold text-slate-900 dark:text-slate-100 flex items-center gap-2">
            <Server className="w-5 h-5 text-indigo-600 dark:text-indigo-400" />
            <span>{isEn ? 'Database Engine Servers' : 'Server Mesin Basis Data'}</span>
          </h3>
          <p className="text-xs text-slate-500 dark:text-slate-400 mt-0.5">
            {isEn
              ? 'Manage database service lifecycles and on-demand installation'
              : 'Kelola siklus hidup service database dan instalasi paket on-demand'}
          </p>
        </div>

        <button
          onClick={() => { void loadServers(); }}
          disabled={isLoading}
          className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-xl border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900 text-xs font-medium text-slate-700 dark:text-slate-300 hover:bg-slate-50 dark:hover:bg-slate-800 transition"
        >
          <RefreshCw className={`w-3.5 h-3.5 ${isLoading ? 'animate-spin' : ''}`} />
          <span>{isEn ? 'Refresh' : 'Muat Ulang'}</span>
        </button>
      </div>

      {(installs.error || serverError) && <p role="alert" className="text-xs text-[var(--danger)]">{installs.error || serverError} Tombol instalasi dinonaktifkan sampai status tugas dapat diperiksa.</p>}

      {/* Server Cards */}
      {isLoading ? (
        <div className="p-12 text-center text-xs text-slate-500 flex items-center justify-center gap-2 bg-white dark:bg-slate-900 rounded-2xl border border-slate-200 dark:border-slate-800">
          <Loader2 className="w-4 h-4 animate-spin text-indigo-600" />
          <span>Memuat status service database...</span>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
          {/* MariaDB Card */}
          {(() => {
            const mariadb = servers.find((s) => s.engine === 'mariadb') || {
              engine: 'mariadb' as DatabaseEngine,
              is_installed: false,
              is_running: false,
              version: null,
              port: 3306,
              socket_path: '/run/mysqld/mysqld.sock',
              updated_at: 0,
            };
            const installJob = installs.jobs.find(job => jobEngine(job.payload_json) === 'mariadb');
            const isBusy = processingEngine === 'mariadb' || submittingEngines.has('mariadb') || Boolean(installJob) || jobsUnavailable;

            return (
              <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl p-6 shadow-sm space-y-5">
                <div className="flex items-start justify-between">
                  <div className="flex items-center gap-3">
                    <div className="p-3 bg-sky-50 dark:bg-sky-950/50 text-sky-600 dark:text-sky-400 rounded-2xl border border-sky-100 dark:border-sky-800/60">
                      <Database className="w-6 h-6" />
                    </div>
                    <div>
                      <h4 className="text-lg font-bold text-slate-900 dark:text-slate-100">
                        MariaDB (MySQL)
                      </h4>
                      <p className="text-xs text-slate-500 dark:text-slate-400 font-mono">
                        Port {mariadb.port}
                      </p>
                    </div>
                  </div>

                  <div className="flex flex-col items-end gap-1.5">
                    {mariadb.is_installed ? (
                      <span className="inline-flex items-center gap-1 px-2.5 py-0.5 rounded-full text-xs font-semibold bg-emerald-50 dark:bg-emerald-950/50 text-emerald-700 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800">
                        <CheckCircle2 className="w-3 h-3" />
                        Terpasang
                      </span>
                    ) : (
                      <span className="inline-flex items-center gap-1 px-2.5 py-0.5 rounded-full text-xs font-semibold bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-400 border border-slate-200 dark:border-slate-700">
                        Belum Terpasang
                      </span>
                    )}

                    {mariadb.is_installed && (
                      <span
                        className={`inline-flex items-center gap-1 px-2.5 py-0.5 rounded-full text-xs font-medium ${
                          mariadb.is_running
                            ? 'bg-emerald-50 dark:bg-emerald-950/50 text-emerald-700 dark:text-emerald-300'
                            : 'bg-rose-50 dark:bg-rose-950/50 text-rose-700 dark:text-rose-300'
                        }`}
                      >
                        <span
                          className={`w-1.5 h-1.5 rounded-full ${
                            mariadb.is_running ? 'bg-emerald-500' : 'bg-rose-500'
                          }`}
                        />
                        {mariadb.is_running ? 'Berjalan' : 'Berhenti'}
                      </span>
                    )}
                  </div>
                </div>

                {/* Server Info Details */}
                <div className="p-3.5 bg-slate-50 dark:bg-slate-850/60 rounded-xl border border-slate-100 dark:border-slate-800 space-y-2 text-xs">
                  <div className="flex items-center justify-between">
                    <span className="text-slate-500 dark:text-slate-400">Versi:</span>
                    <span className="font-mono text-slate-800 dark:text-slate-200 font-medium">
                      {mariadb.version || '—'}
                    </span>
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-slate-500 dark:text-slate-400">Socket:</span>
                    <span className="font-mono text-slate-800 dark:text-slate-200 truncate max-w-[200px]">
                      {mariadb.socket_path || '/run/mysqld/mysqld.sock'}
                    </span>
                  </div>
                </div>

                {installJob && <RuntimeJobProgress job={installJob} runtime="MariaDB" />}
                <div className="pt-2 border-t border-slate-100 dark:border-slate-800 flex items-center justify-end gap-2">
                  {!mariadb.is_installed ? (
                    <button
                      onClick={() => handleInstall('mariadb')}
                      disabled={isBusy}
                      className="inline-flex items-center gap-2 px-4 py-2 bg-indigo-600 hover:bg-indigo-700 active:bg-indigo-800 text-white rounded-xl text-xs font-semibold shadow-sm transition disabled:opacity-50"
                    >
                      {isBusy ? (
                        <Loader2 className="w-3.5 h-3.5 animate-spin" />
                      ) : (
                        <Download className="w-3.5 h-3.5" />
                      )}
                      <span>{installJob ? installJob.status === 'queued' ? 'Instalasi MariaDB dalam antrean' : installJob.status === 'succeeded' ? 'Memeriksa hasil instalasi MariaDB' : 'Memasang MariaDB…' : submittingEngines.has('mariadb') ? 'Menjadwalkan instalasi…' : jobsUnavailable ? 'Memeriksa tugas instalasi…' : 'Pasang Server MariaDB'}</span>
                    </button>
                  ) : (
                    <>
                      {mariadb.is_running ? (
                        <button
                          onClick={() => handleAction('mariadb', 'stop')}
                          disabled={isBusy}
                          className="inline-flex items-center gap-1.5 px-3 py-1.5 bg-rose-50 dark:bg-rose-950/40 text-rose-700 dark:text-rose-300 border border-rose-200 dark:border-rose-800 hover:bg-rose-100 rounded-xl text-xs font-medium transition disabled:opacity-50"
                        >
                          <Power className="w-3.5 h-3.5" />
                          <span>Hentikan</span>
                        </button>
                      ) : (
                        <button
                          onClick={() => handleAction('mariadb', 'start')}
                          disabled={isBusy}
                          className="inline-flex items-center gap-1.5 px-3 py-1.5 bg-emerald-50 dark:bg-emerald-950/40 text-emerald-700 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800 hover:bg-emerald-100 rounded-xl text-xs font-medium transition disabled:opacity-50"
                        >
                          <Power className="w-3.5 h-3.5" />
                          <span>Mulai</span>
                        </button>
                      )}

                      <button
                        onClick={() => handleAction('mariadb', 'restart')}
                        disabled={isBusy}
                        className="inline-flex items-center gap-1.5 px-3 py-1.5 bg-slate-100 dark:bg-slate-800 text-slate-700 dark:text-slate-200 border border-slate-200 dark:border-slate-700 hover:bg-slate-200 dark:hover:bg-slate-700 rounded-xl text-xs font-medium transition disabled:opacity-50"
                      >
                        <RotateCw className={`w-3.5 h-3.5 ${isBusy ? 'animate-spin' : ''}`} />
                        <span>Muat Ulang</span>
                      </button>
                    </>
                  )}
                </div>
              </div>
            );
          })()}

          {/* PostgreSQL Card */}
          {(() => {
            const pg = servers.find((s) => s.engine === 'postgresql') || {
              engine: 'postgresql' as DatabaseEngine,
              is_installed: false,
              is_running: false,
              version: null,
              port: 5432,
              socket_path: '/var/run/postgresql/.s.PGSQL.5432',
              updated_at: 0,
            };
            const installJob = installs.jobs.find(job => jobEngine(job.payload_json) === 'postgresql');
            const isBusy = processingEngine === 'postgresql' || submittingEngines.has('postgresql') || Boolean(installJob) || jobsUnavailable;

            return (
              <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl p-6 shadow-sm space-y-5">
                <div className="flex items-start justify-between">
                  <div className="flex items-center gap-3">
                    <div className="p-3 bg-indigo-50 dark:bg-indigo-950/50 text-indigo-600 dark:text-indigo-400 rounded-2xl border border-indigo-100 dark:border-indigo-800/60">
                      <Database className="w-6 h-6" />
                    </div>
                    <div>
                      <h4 className="text-lg font-bold text-slate-900 dark:text-slate-100">
                        PostgreSQL
                      </h4>
                      <p className="text-xs text-slate-500 dark:text-slate-400 font-mono">
                        Port {pg.port}
                      </p>
                    </div>
                  </div>

                  <div className="flex flex-col items-end gap-1.5">
                    {pg.is_installed ? (
                      <span className="inline-flex items-center gap-1 px-2.5 py-0.5 rounded-full text-xs font-semibold bg-emerald-50 dark:bg-emerald-950/50 text-emerald-700 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800">
                        <CheckCircle2 className="w-3 h-3" />
                        Terpasang
                      </span>
                    ) : (
                      <span className="inline-flex items-center gap-1 px-2.5 py-0.5 rounded-full text-xs font-semibold bg-slate-100 dark:bg-slate-800 text-slate-600 dark:text-slate-400 border border-slate-200 dark:border-slate-700">
                        Belum Terpasang
                      </span>
                    )}

                    {pg.is_installed && (
                      <span
                        className={`inline-flex items-center gap-1 px-2.5 py-0.5 rounded-full text-xs font-medium ${
                          pg.is_running
                            ? 'bg-emerald-50 dark:bg-emerald-950/50 text-emerald-700 dark:text-emerald-300'
                            : 'bg-rose-50 dark:bg-rose-950/50 text-rose-700 dark:text-rose-300'
                        }`}
                      >
                        <span
                          className={`w-1.5 h-1.5 rounded-full ${
                            pg.is_running ? 'bg-emerald-500' : 'bg-rose-500'
                          }`}
                        />
                        {pg.is_running ? 'Berjalan' : 'Berhenti'}
                      </span>
                    )}
                  </div>
                </div>

                {/* Server Info Details */}
                <div className="p-3.5 bg-slate-50 dark:bg-slate-850/60 rounded-xl border border-slate-100 dark:border-slate-800 space-y-2 text-xs">
                  <div className="flex items-center justify-between">
                    <span className="text-slate-500 dark:text-slate-400">Versi:</span>
                    <span className="font-mono text-slate-800 dark:text-slate-200 font-medium">
                      {pg.version || '—'}
                    </span>
                  </div>
                  <div className="flex items-center justify-between">
                    <span className="text-slate-500 dark:text-slate-400">Socket:</span>
                    <span className="font-mono text-slate-800 dark:text-slate-200 truncate max-w-[200px]">
                      {pg.socket_path || '/var/run/postgresql/.s.PGSQL.5432'}
                    </span>
                  </div>
                </div>

                {installJob && <RuntimeJobProgress job={installJob} runtime="PostgreSQL" />}
                <div className="pt-2 border-t border-slate-100 dark:border-slate-800 flex items-center justify-end gap-2">
                  {!pg.is_installed ? (
                    <button
                      onClick={() => handleInstall('postgresql')}
                      disabled={isBusy}
                      className="inline-flex items-center gap-2 px-4 py-2 bg-indigo-600 hover:bg-indigo-700 active:bg-indigo-800 text-white rounded-xl text-xs font-semibold shadow-sm transition disabled:opacity-50"
                    >
                      {isBusy ? (
                        <Loader2 className="w-3.5 h-3.5 animate-spin" />
                      ) : (
                        <Download className="w-3.5 h-3.5" />
                      )}
                      <span>{installJob ? installJob.status === 'queued' ? 'Instalasi PostgreSQL dalam antrean' : installJob.status === 'succeeded' ? 'Memeriksa hasil instalasi PostgreSQL' : 'Memasang PostgreSQL…' : submittingEngines.has('postgresql') ? 'Menjadwalkan instalasi…' : jobsUnavailable ? 'Memeriksa tugas instalasi…' : 'Pasang Server PostgreSQL'}</span>
                    </button>
                  ) : (
                    <>
                      {pg.is_running ? (
                        <button
                          onClick={() => handleAction('postgresql', 'stop')}
                          disabled={isBusy}
                          className="inline-flex items-center gap-1.5 px-3 py-1.5 bg-rose-50 dark:bg-rose-950/40 text-rose-700 dark:text-rose-300 border border-rose-200 dark:border-rose-800 hover:bg-rose-100 rounded-xl text-xs font-medium transition disabled:opacity-50"
                        >
                          <Power className="w-3.5 h-3.5" />
                          <span>Hentikan</span>
                        </button>
                      ) : (
                        <button
                          onClick={() => handleAction('postgresql', 'start')}
                          disabled={isBusy}
                          className="inline-flex items-center gap-1.5 px-3 py-1.5 bg-emerald-50 dark:bg-emerald-950/40 text-emerald-700 dark:text-emerald-300 border border-emerald-200 dark:border-emerald-800 hover:bg-emerald-100 rounded-xl text-xs font-medium transition disabled:opacity-50"
                        >
                          <Power className="w-3.5 h-3.5" />
                          <span>Mulai</span>
                        </button>
                      )}

                      <button
                        onClick={() => handleAction('postgresql', 'restart')}
                        disabled={isBusy}
                        className="inline-flex items-center gap-1.5 px-3 py-1.5 bg-slate-100 dark:bg-slate-800 text-slate-700 dark:text-slate-200 border border-slate-200 dark:border-slate-700 hover:bg-slate-200 dark:hover:bg-slate-700 rounded-xl text-xs font-medium transition disabled:opacity-50"
                      >
                        <RotateCw className={`w-3.5 h-3.5 ${isBusy ? 'animate-spin' : ''}`} />
                        <span>Muat Ulang</span>
                      </button>
                    </>
                  )}
                </div>
              </div>
            );
          })()}
        </div>
      )}
    </div>
  );
};
