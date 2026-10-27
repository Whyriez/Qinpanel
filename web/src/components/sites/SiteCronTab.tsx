import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect, useCallback, useMemo } from 'react';
import {
  Clock,
  Play,
  RotateCw,
  Plus,
  Trash2,
  Edit3,
  Search,
  CheckCircle2,
  AlertCircle,
  FileText,
  Calendar,
  X,
  Loader2,
  Terminal,
  Layers,
} from 'lucide-react';
import { useTranslation } from 'react-i18next';
import {
  fetchSiteCronJobs,
  createSiteCronJob,
  updateSiteCronJob,
  deleteSiteCronJob,
  runSiteCronJob,
  fetchSiteCronRuns,
  fetchSiteCronLogs,
  validateCronExpression,
} from '../../api/client';
import {
  CronJob,
  CronQuota,
  CronRun,
  CreateCronJobRequest,
  UpdateCronJobRequest,
} from '../../types/api';

export interface SiteCronTabProps {
  siteId: number;
}

interface CronPreset {
  id: string;
  labelKey: string;
  expression: string;
}

const CRON_PRESETS: CronPreset[] = [
  { id: 'every_5m', labelKey: 'preset_every_5m', expression: '*/5 * * * *' },
  { id: 'every_15m', labelKey: 'preset_every_15m', expression: '*/15 * * * *' },
  { id: 'hourly', labelKey: 'preset_hourly', expression: '0 * * * *' },
  { id: 'daily_midnight', labelKey: 'preset_daily_midnight', expression: '0 0 * * *' },
  { id: 'daily_2am', labelKey: 'preset_daily_2am', expression: '0 2 * * *' },
  { id: 'weekly', labelKey: 'preset_weekly', expression: '0 0 * * 0' },
  { id: 'monthly', labelKey: 'preset_monthly', expression: '0 0 1 * *' },
  { id: 'custom', labelKey: 'preset_custom', expression: '* * * * *' },
];

export const SiteCronTab: React.FC<SiteCronTabProps> = ({ siteId }) => {
  const { t, i18n } = useTranslation();
  const isEn = i18n.language.startsWith('en');

  const [jobs, setJobs] = useState<CronJob[]>([]);
  const [quota, setQuota] = useState<CronQuota>({ used: 0, limit: 10 });
  const [isLoading, setIsLoading] = useState(true);
  const [searchQuery, setSearchQuery] = useState('');
  const [toastMessage, setToastMessage] = useState<{ text: string; type: 'success' | 'error' } | null>(null);

  // Running state per job ID
  const [runningJobIds, setRunningJobIds] = useState<Set<number>>(new Set());

  // Create / Edit modal state
  const [isModalOpen, setIsModalOpen] = useState(false);
  const [editingJob, setEditingJob] = useState<CronJob | null>(null);
  const [formName, setFormName] = useState('');
  const [formCommand, setFormCommand] = useState('');
  const [formDescription, setFormDescription] = useState('');
  const [formWorkDir, setFormWorkDir] = useState('');
  const [formTimeoutSecs, setFormTimeoutSecs] = useState<number>(3600);
  const [formEnabled, setFormEnabled] = useState(true);
  const [formSchedule, setFormSchedule] = useState('0 2 * * *');
  const [formMinute, setFormMinute] = useState('0');
  const [formHour, setFormHour] = useState('2');
  const [formDom, setFormDom] = useState('*');
  const [formMonth, setFormMonth] = useState('*');
  const [formDow, setFormDow] = useState('*');
  const [selectedPreset, setSelectedPreset] = useState('daily_2am');
  const [humanTranslation, setHumanTranslation] = useState('');
  const [validationError, setValidationError] = useState<string | null>(null);
  const [isSaving, setIsSaving] = useState(false);

  // History & Logs Modal state
  const [historyJob, setHistoryJob] = useState<CronJob | null>(null);
  const [historyActiveTab, setHistoryActiveTab] = useState<'runs' | 'logs'>('runs');
  const [jobRuns, setJobRuns] = useState<CronRun[]>([]);
  const [jobLogs, setJobLogs] = useState<string[]>([]);
  const [isLoadingHistory, setIsLoadingHistory] = useState(false);

  // Delete modal state
  const [deletingJob, setDeletingJob] = useState<CronJob | null>(null);
  const [isDeleting, setIsDeleting] = useState(false);

  const showToast = useCallback((text: string, type: 'success' | 'error' = 'success') => {
    setToastMessage({ text, type });
    setTimeout(() => setToastMessage(null), 4000);
  }, []);

  const loadJobs = useCallback(async () => {
    setIsLoading(true);
    try {
      const res = await fetchSiteCronJobs(siteId);
      setJobs(res.jobs);
      setQuota(res.quota);
    } catch (err: unknown) {
      showToast(err instanceof Error ? err.message : 'Gagal memuat cron job', 'error');
    } finally {
      setIsLoading(false);
    }
  }, [siteId, showToast]);

  useEffect(() => {
    loadJobs();
  }, [loadJobs]);

  // Sync parts to formSchedule
  const updateScheduleFromParts = useCallback(
    async (m: string, h: string, dom: string, mon: string, dow: string) => {
      const expr = `${m.trim() || '*'} ${h.trim() || '*'} ${dom.trim() || '*'} ${mon.trim() || '*'} ${dow.trim() || '*'}`;
      setFormSchedule(expr);
      try {
        const val = await validateCronExpression(expr);
        if (val.valid) {
          setValidationError(null);
          setHumanTranslation(isEn ? val.human_en || '' : val.human_id || '');
        } else {
          setValidationError(val.error || 'Ekspresi tidak valid');
          setHumanTranslation('');
        }
      } catch {
        // Fallback
      }
    },
    [isEn]
  );

  const handlePresetChange = (presetId: string) => {
    setSelectedPreset(presetId);
    const found = CRON_PRESETS.find((p) => p.id === presetId);
    if (found) {
      const parts = found.expression.split(' ');
      if (parts.length === 5) {
        setFormMinute(parts[0]);
        setFormHour(parts[1]);
        setFormDom(parts[2]);
        setFormMonth(parts[3]);
        setFormDow(parts[4]);
        updateScheduleFromParts(parts[0], parts[1], parts[2], parts[3], parts[4]);
      }
    }
  };

  const openCreateModal = () => {
    setEditingJob(null);
    setFormName('');
    setFormCommand('');
    setFormDescription('');
    setFormWorkDir('');
    setFormTimeoutSecs(3600);
    setFormEnabled(true);
    setSelectedPreset('daily_2am');
    setFormMinute('0');
    setFormHour('2');
    setFormDom('*');
    setFormMonth('*');
    setFormDow('*');
    setValidationError(null);
    updateScheduleFromParts('0', '2', '*', '*', '*');
    setIsModalOpen(true);
  };

  const openEditModal = (job: CronJob) => {
    setEditingJob(job);
    setFormName(job.name);
    setFormCommand(job.command);
    setFormDescription(job.description || '');
    setFormWorkDir(job.work_dir || '');
    setFormTimeoutSecs(job.timeout_secs);
    setFormEnabled(job.enabled);
    setValidationError(null);

    const parts = job.schedule.split(' ');
    if (parts.length === 5) {
      setFormMinute(parts[0]);
      setFormHour(parts[1]);
      setFormDom(parts[2]);
      setFormMonth(parts[3]);
      setFormDow(parts[4]);
      const matchedPreset = CRON_PRESETS.find((p) => p.expression === job.schedule);
      setSelectedPreset(matchedPreset ? matchedPreset.id : 'custom');
      updateScheduleFromParts(parts[0], parts[1], parts[2], parts[3], parts[4]);
    }
    setIsModalOpen(true);
  };

  const handleSaveJob = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!formName.trim() || !formCommand.trim() || !formSchedule.trim()) {
      showToast('Mohon lengkapi field nama, perintah, dan jadwal', 'error');
      return;
    }

    setIsSaving(true);
    try {
      if (editingJob) {
        const updateReq: UpdateCronJobRequest = {
          name: formName.trim(),
          command: formCommand.trim(),
          schedule: formSchedule.trim(),
          description: formDescription.trim() || undefined,
          work_dir: formWorkDir.trim() || undefined,
          timeout_secs: formTimeoutSecs,
          enabled: formEnabled,
        };
        await updateSiteCronJob(siteId, editingJob.id, updateReq);
        showToast('Cron job berhasil diperbarui');
      } else {
        const createReq: CreateCronJobRequest = {
          name: formName.trim(),
          command: formCommand.trim(),
          schedule: formSchedule.trim(),
          description: formDescription.trim() || undefined,
          work_dir: formWorkDir.trim() || undefined,
          timeout_secs: formTimeoutSecs,
          enabled: formEnabled,
        };
        await createSiteCronJob(siteId, createReq);
        showToast('Cron job berhasil ditambahkan');
      }
      setIsModalOpen(false);
      loadJobs();
    } catch (err: unknown) {
      showToast(err instanceof Error ? err.message : 'Gagal menyimpan cron job', 'error');
    } finally {
      setIsSaving(false);
    }
  };

  const handleToggleEnable = async (job: CronJob) => {
    try {
      await updateSiteCronJob(siteId, job.id, { enabled: !job.enabled });
      showToast(`Cron job ${job.name} ${!job.enabled ? 'diaktifkan' : 'dinonaktifkan'}`);
      loadJobs();
    } catch (err: unknown) {
      showToast(err instanceof Error ? err.message : 'Gagal mengubah status cron job', 'error');
    }
  };

  const handleRunNow = async (job: CronJob) => {
    setRunningJobIds((prev) => new Set(prev).add(job.id));
    try {
      const res = await runSiteCronJob(siteId, job.id);
      if (res.status === 'success') {
        showToast(`Job ${job.name} selesai dalam ${res.duration_ms || 0} ms`);
      } else {
        showToast(`Job ${job.name} gagal dengan exit code ${res.exit_code}`, 'error');
      }
      loadJobs();
    } catch (err: unknown) {
      showToast(err instanceof Error ? err.message : 'Gagal menjalankan cron job', 'error');
    } finally {
      setRunningJobIds((prev) => {
        const next = new Set(prev);
        next.delete(job.id);
        return next;
      });
    }
  };

  const openHistoryModal = async (job: CronJob) => {
    setHistoryJob(job);
    setHistoryActiveTab('runs');
    setIsLoadingHistory(true);
    try {
      const [runs, logsRes] = await Promise.all([
        fetchSiteCronRuns(siteId, job.id, 20),
        fetchSiteCronLogs(siteId, job.id, 100),
      ]);
      setJobRuns(runs);
      setJobLogs(logsRes.logs);
    } catch (err: unknown) {
      showToast(err instanceof Error ? err.message : 'Gagal memuat riwayat eksekusi', 'error');
    } finally {
      setIsLoadingHistory(false);
    }
  };

  const refreshLogsOnly = async () => {
    if (!historyJob) return;
    setIsLoadingHistory(true);
    try {
      const logsRes = await fetchSiteCronLogs(siteId, historyJob.id, 100);
      setJobLogs(logsRes.logs);
    } catch (err: unknown) {
      showToast(err instanceof Error ? err.message : 'Gagal memuat log', 'error');
    } finally {
      setIsLoadingHistory(false);
    }
  };

  const handleDeleteConfirm = async () => {
    if (!deletingJob) return;
    setIsDeleting(true);
    try {
      await deleteSiteCronJob(siteId, deletingJob.id);
      showToast(`Cron job ${deletingJob.name} berhasil dihapus`);
      setDeletingJob(null);
      loadJobs();
    } catch (err: unknown) {
      showToast(err instanceof Error ? err.message : 'Gagal menghapus cron job', 'error');
    } finally {
      setIsDeleting(false);
    }
  };

  const filteredJobs = useMemo(() => {
    if (!searchQuery.trim()) return jobs;
    const q = searchQuery.toLowerCase();
    return jobs.filter(
      (j) =>
        j.name.toLowerCase().includes(q) ||
        j.command.toLowerCase().includes(q) ||
        j.schedule.toLowerCase().includes(q) ||
        (j.description && j.description.toLowerCase().includes(q))
    );
  }, [jobs, searchQuery]);

  return (
    <div className="space-y-6">
      {/* Toast Notification */}
      {toastMessage && (
        <div
          role="status"
          className={`fixed bottom-6 right-6 z-50 flex items-center gap-3 px-4 py-3 rounded-xl shadow-xl text-sm font-medium transition-all animate-in fade-in slide-in-from-bottom-2 ${
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
        </div>
      )}

      {/* Header & Quota Card */}
      <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl p-6 shadow-sm">
        <div className="flex flex-col lg:flex-row lg:items-center justify-between gap-4">
          <div>
            <div className="flex items-center gap-3">
              <div className="p-2.5 rounded-xl bg-indigo-50 dark:bg-indigo-950/60 text-indigo-600 dark:text-indigo-400">
                <Clock className="w-6 h-6" />
              </div>
              <div>
                <h2 className="text-xl font-bold text-slate-900 dark:text-white">
                  {t('cron.title')}
                </h2>
                <p className="text-sm text-slate-500 dark:text-slate-400 mt-0.5">
                  {t('cron.subtitle')}
                </p>
              </div>
            </div>
          </div>

          <div className="flex items-center gap-3 flex-wrap">
            {/* Quota Badge */}
            <div className="flex items-center gap-2 px-3 py-1.5 rounded-xl bg-slate-100 dark:bg-slate-800/70 border border-slate-200 dark:border-slate-700 text-xs font-medium text-slate-700 dark:text-slate-300">
              <Layers className="w-4 h-4 text-slate-500" />
              <span>{t('cron.quota_badge')}</span>
              <span
                className={`font-semibold px-2 py-0.5 rounded-full ${
                  quota.used >= quota.limit
                    ? 'bg-rose-100 text-rose-700 dark:bg-rose-950 dark:text-rose-300'
                    : 'bg-indigo-100 text-indigo-700 dark:bg-indigo-950 dark:text-indigo-300'
                }`}
              >
                {quota.used} / {quota.limit}
              </span>
            </div>

            <button
              onClick={openCreateModal}
              disabled={quota.used >= quota.limit}
              className="inline-flex items-center gap-2 px-4 py-2 rounded-xl text-sm font-semibold text-white bg-indigo-600 hover:bg-indigo-500 active:scale-[0.98] transition-all disabled:opacity-50 disabled:cursor-not-allowed shadow-sm shadow-indigo-600/20"
            >
              <Plus className="w-4 h-4" />
              <span>{t('cron.add_job')}</span>
            </button>
          </div>
        </div>
      </div>

      {/* Filter and Search */}
      <div className="flex items-center justify-between gap-4 flex-wrap">
        <div className="relative flex-1 min-w-[280px] max-w-md">
          <Search className="w-4 h-4 absolute left-3.5 top-1/2 -translate-y-1/2 text-slate-400" />
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder={t('cron.search_placeholder')}
            className="w-full pl-10 pr-4 py-2 text-sm rounded-xl border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900 text-slate-900 dark:text-white placeholder:text-slate-400 focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500 transition-all"
          />
        </div>

        <button
          onClick={loadJobs}
          disabled={isLoading}
          aria-label={t('common.refresh', 'Segarkan')}
          className="inline-flex items-center gap-2 px-3 py-2 rounded-xl text-xs font-semibold text-slate-700 dark:text-slate-300 bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 hover:bg-slate-50 dark:hover:bg-slate-800/50 transition-all"
        >
          <RotateCw className={`w-3.5 h-3.5 ${isLoading ? 'animate-spin' : ''}`} />
          <span>{t('common.refresh', 'Segarkan')}</span>
        </button>
      </div>

      {/* Main Jobs Table */}
      <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl overflow-hidden shadow-sm">
        {isLoading && jobs.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-16 text-slate-400">
            <Loader2 className="w-8 h-8 animate-spin text-indigo-500 mb-3" />
            <p className="text-sm">{t('common.loading', 'Memuat data...')}</p>
          </div>
        ) : filteredJobs.length === 0 ? (
          <div className="text-center py-16 px-4">
            <div className="w-16 h-16 rounded-2xl bg-indigo-50 dark:bg-indigo-950/40 text-indigo-500 mx-auto flex items-center justify-center mb-4">
              <Calendar className="w-8 h-8" />
            </div>
            <h3 className="text-base font-semibold text-slate-900 dark:text-white">
              {t('cron.empty_title')}
            </h3>
            <p className="text-sm text-slate-500 dark:text-slate-400 max-w-md mx-auto mt-1 mb-5">
              {t('cron.empty_desc')}
            </p>
            <button
              onClick={openCreateModal}
              disabled={quota.used >= quota.limit}
              className="inline-flex items-center gap-2 px-4 py-2 rounded-xl text-sm font-semibold text-white bg-indigo-600 hover:bg-indigo-500 transition-all"
            >
              <Plus className="w-4 h-4" />
              <span>{t('cron.add_job')}</span>
            </button>
          </div>
        ) : (
          <div className="overflow-x-auto">
            <table className="w-full text-left text-sm">
              <thead className="bg-slate-50/80 dark:bg-slate-800/40 text-xs font-semibold text-slate-500 dark:text-slate-400 border-b border-slate-200 dark:border-slate-800">
                <tr>
                  <th className="py-3.5 px-4">{t('cron.col_name')}</th>
                  <th className="py-3.5 px-4">{t('cron.col_schedule')}</th>
                  <th className="py-3.5 px-4">{t('cron.col_command')}</th>
                  <th className="py-3.5 px-4">{t('cron.col_last_run')}</th>
                  <th className="py-3.5 px-4">{t('cron.col_status')}</th>
                  <th className="py-3.5 px-4 text-right">{t('cron.col_actions')}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-slate-200 dark:divide-slate-800/60 font-medium">
                {filteredJobs.map((job) => {
                  const isJobRunning = runningJobIds.has(job.id);
                  return (
                    <tr
                      key={job.id}
                      className="hover:bg-slate-50/50 dark:hover:bg-slate-800/20 transition-colors"
                    >
                      {/* Name & Description */}
                      <td className="py-4 px-4">
                        <div className="flex flex-col">
                          <span className="font-semibold text-slate-900 dark:text-white">
                            {job.name}
                          </span>
                          {job.description && (
                            <span className="text-xs text-slate-500 dark:text-slate-400 line-clamp-1 mt-0.5">
                              {job.description}
                            </span>
                          )}
                          <span className="text-[11px] text-slate-400 font-mono mt-0.5">
                            unit: site-{siteId}-cron-{job.id}.timer
                          </span>
                        </div>
                      </td>

                      {/* Schedule */}
                      <td className="py-4 px-4">
                        <div className="flex flex-col gap-1">
                          <div className="inline-flex items-center gap-1.5 font-mono text-xs font-semibold px-2 py-0.5 rounded-md bg-slate-100 dark:bg-slate-800 text-indigo-600 dark:text-indigo-400 w-fit">
                            <span>{job.schedule}</span>
                          </div>
                          <span className="text-xs text-slate-600 dark:text-slate-300">
                            {isEn ? job.human_description_en : job.human_description}
                          </span>
                        </div>
                      </td>

                      {/* Command */}
                      <td className="py-4 px-4 max-w-xs">
                        <div className="font-mono text-xs bg-slate-100 dark:bg-slate-800 px-2.5 py-1 rounded-md text-slate-800 dark:text-slate-200 truncate" title={job.command}>
                          {job.command}
                        </div>
                      </td>

                      {/* Last Run */}
                      <td className="py-4 px-4">
                        {job.last_run_at ? (
                          <div className="flex flex-col text-xs text-slate-600 dark:text-slate-300">
                            <span>{new Date(job.last_run_at * 1000).toLocaleString()}</span>
                            <div className="flex items-center gap-2 text-[11px] text-slate-400 mt-0.5">
                              {job.last_duration_ms !== null && job.last_duration_ms !== undefined && (
                                <span>{job.last_duration_ms} ms</span>
                              )}
                              {job.last_exit_code !== null && job.last_exit_code !== undefined && (
                                <span
                                  className={`px-1.5 py-0.2 rounded font-mono ${
                                    job.last_exit_code === 0
                                      ? 'text-emerald-600 bg-emerald-50 dark:bg-emerald-950/60'
                                      : 'text-rose-600 bg-rose-50 dark:bg-rose-950/60'
                                  }`}
                                >
                                  exit {job.last_exit_code}
                                </span>
                              )}
                            </div>
                          </div>
                        ) : (
                          <span className="text-xs text-slate-400">-</span>
                        )}
                      </td>

                      {/* Status & Enable Toggle */}
                      <td className="py-4 px-4">
                        <div className="flex items-center gap-3">
                          {/* Status Badge */}
                          <span
                            className={`inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full text-xs font-semibold ${
                              job.status === 'running' || isJobRunning
                                ? 'bg-blue-100 text-blue-700 dark:bg-blue-950 dark:text-blue-300 animate-pulse'
                                : job.status === 'success'
                                ? 'bg-emerald-100 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300'
                                : job.status === 'failed'
                                ? 'bg-rose-100 text-rose-700 dark:bg-rose-950 dark:text-rose-300'
                                : 'bg-slate-100 text-slate-600 dark:bg-slate-800 dark:text-slate-400'
                            }`}
                          >
                            <span
                              className={`w-1.5 h-1.5 rounded-full ${
                                job.status === 'running' || isJobRunning
                                  ? 'bg-blue-500'
                                  : job.status === 'success'
                                  ? 'bg-emerald-500'
                                  : job.status === 'failed'
                                  ? 'bg-rose-500'
                                  : 'bg-slate-400'
                              }`}
                            />
                            <span>
                              {job.status === 'running' || isJobRunning
                                ? t('cron.status_running')
                                : job.status === 'success'
                                ? t('cron.status_success')
                                : job.status === 'failed'
                                ? t('cron.status_failed')
                                : t('cron.status_idle')}
                            </span>
                          </span>

                          {/* Enabled Switch */}
                          <button
                            onClick={() => handleToggleEnable(job)}
                            role="switch"
                            aria-checked={job.enabled}
                            className={`relative inline-flex h-5 w-9 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
                              job.enabled ? 'bg-indigo-600' : 'bg-slate-300 dark:bg-slate-700'
                            }`}
                          >
                            <span
                              className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow-sm ring-0 transition duration-200 ease-in-out ${
                                job.enabled ? 'translate-x-4' : 'translate-x-0'
                              }`}
                            />
                          </button>
                        </div>
                      </td>

                      {/* Actions */}
                      <td className="py-4 px-4 text-right">
                        <div className="inline-flex items-center gap-1.5">
                          {/* Run Now Button */}
                          <button
                            onClick={() => handleRunNow(job)}
                            disabled={isJobRunning}
                            title={t('cron.run_now')}
                            className="p-1.5 rounded-lg text-indigo-600 dark:text-indigo-400 hover:bg-indigo-50 dark:hover:bg-indigo-950/60 transition-all disabled:opacity-50"
                          >
                            {isJobRunning ? (
                              <Loader2 className="w-4 h-4 animate-spin text-indigo-500" />
                            ) : (
                              <Play className="w-4 h-4 fill-current" />
                            )}
                          </button>

                          {/* History & Logs Button */}
                          <button
                            onClick={() => openHistoryModal(job)}
                            title={t('cron.logs_and_history')}
                            className="p-1.5 rounded-lg text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 transition-all"
                          >
                            <FileText className="w-4 h-4" />
                          </button>

                          {/* Edit Button */}
                          <button
                            onClick={() => openEditModal(job)}
                            title={t('cron.edit')}
                            className="p-1.5 rounded-lg text-slate-600 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 transition-all"
                          >
                            <Edit3 className="w-4 h-4" />
                          </button>

                          {/* Delete Button */}
                          <button
                            onClick={() => setDeletingJob(job)}
                            title={t('cron.delete')}
                            className="p-1.5 rounded-lg text-rose-600 dark:text-rose-400 hover:bg-rose-50 dark:hover:bg-rose-950/60 transition-all"
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

      {/* Modal: Create or Edit Cron Job */}
      {isModalOpen && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-sm animate-in fade-in"
          onDismiss={() => setIsModalOpen(false)}
        >
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl w-full max-w-xl shadow-2xl overflow-hidden flex flex-col max-h-[90vh] dialog-panel">
            <div className="flex items-center justify-between px-6 py-4 border-b border-slate-200 dark:border-slate-800 dialog-header">
              <h3 className="text-lg font-bold text-slate-900 dark:text-white">
                {editingJob ? t('cron.modal_edit_title') : t('cron.modal_create_title')}
              </h3>
              <button
                onClick={() => setIsModalOpen(false)}
                className="p-1 rounded-lg text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 transition-colors"
              >
                <X className="w-5 h-5" />
              </button>
            </div>

            <form onSubmit={handleSaveJob} className="p-6 overflow-y-auto flex-1 dialog-form">
              <div className="dialog-body space-y-4">
                {/* Name */}
                <div>
                  <label className="block text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-1.5">
                    {t('cron.form_name_label')} *
                  </label>
                  <input
                    type="text"
                    required
                    value={formName}
                    onChange={(e) => setFormName(e.target.value)}
                    placeholder={t('cron.form_name_placeholder')}
                    className="w-full px-3.5 py-2 text-sm rounded-xl border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-950 text-slate-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500"
                  />
                </div>

                {/* Command */}
                <div>
                  <label className="block text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-1.5">
                    {t('cron.form_command_label')} *
                  </label>
                  <input
                    type="text"
                    required
                    value={formCommand}
                    onChange={(e) => setFormCommand(e.target.value)}
                    placeholder={t('cron.form_command_placeholder')}
                    className="w-full px-3.5 py-2 text-sm font-mono rounded-xl border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-950 text-slate-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500"
                  />
                  <p className="text-[11px] text-slate-400 mt-1">{t('cron.form_command_hint')}</p>
                </div>

                {/* Presets Selection */}
                <div>
                  <label className="block text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-1.5">
                    {t('cron.preset_label')}
                  </label>
                  <div className="grid grid-cols-2 sm:grid-cols-4 gap-2">
                    {CRON_PRESETS.map((p) => (
                      <button
                        key={p.id}
                        type="button"
                        onClick={() => handlePresetChange(p.id)}
                        className={`px-2.5 py-1.5 text-xs rounded-xl font-medium border text-center transition-all ${
                          selectedPreset === p.id
                            ? 'bg-indigo-50 border-indigo-500 text-indigo-700 dark:bg-indigo-950/60 dark:border-indigo-400 dark:text-indigo-300 font-semibold shadow-sm'
                            : 'border-slate-200 dark:border-slate-800 hover:bg-slate-50 dark:hover:bg-slate-800/40 text-slate-600 dark:text-slate-300'
                        }`}
                      >
                        {t(`cron.${p.labelKey}`)}
                      </button>
                    ))}
                  </div>
                </div>

                {/* 5-part cron builder inputs */}
                <div className="p-4 rounded-xl bg-slate-50 dark:bg-slate-950/50 border border-slate-200 dark:border-slate-800 space-y-3">
                  <div className="flex items-center justify-between">
                    <span className="text-xs font-bold text-slate-800 dark:text-slate-200">
                      {t('cron.schedule_builder_title')}
                    </span>
                    <span className="font-mono text-xs font-bold text-indigo-600 dark:text-indigo-400 bg-indigo-50 dark:bg-indigo-950/60 px-2.5 py-0.5 rounded-md">
                      {formSchedule}
                    </span>
                  </div>

                  <div className="grid grid-cols-5 gap-2">
                    <div>
                      <label className="block text-[11px] font-semibold text-slate-500 mb-1">
                        {t('cron.cron_minute')}
                      </label>
                      <input
                        type="text"
                        value={formMinute}
                        onChange={(e) => {
                          setFormMinute(e.target.value);
                          setSelectedPreset('custom');
                          updateScheduleFromParts(e.target.value, formHour, formDom, formMonth, formDow);
                        }}
                        className="w-full text-center px-2 py-1.5 text-xs font-mono font-bold rounded-lg border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900 text-slate-900 dark:text-white"
                      />
                    </div>
                    <div>
                      <label className="block text-[11px] font-semibold text-slate-500 mb-1">
                        {t('cron.cron_hour')}
                      </label>
                      <input
                        type="text"
                        value={formHour}
                        onChange={(e) => {
                          setFormHour(e.target.value);
                          setSelectedPreset('custom');
                          updateScheduleFromParts(formMinute, e.target.value, formDom, formMonth, formDow);
                        }}
                        className="w-full text-center px-2 py-1.5 text-xs font-mono font-bold rounded-lg border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900 text-slate-900 dark:text-white"
                      />
                    </div>
                    <div>
                      <label className="block text-[11px] font-semibold text-slate-500 mb-1">
                        {t('cron.cron_dom')}
                      </label>
                      <input
                        type="text"
                        value={formDom}
                        onChange={(e) => {
                          setFormDom(e.target.value);
                          setSelectedPreset('custom');
                          updateScheduleFromParts(formMinute, formHour, e.target.value, formMonth, formDow);
                        }}
                        className="w-full text-center px-2 py-1.5 text-xs font-mono font-bold rounded-lg border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900 text-slate-900 dark:text-white"
                      />
                    </div>
                    <div>
                      <label className="block text-[11px] font-semibold text-slate-500 mb-1">
                        {t('cron.cron_month')}
                      </label>
                      <input
                        type="text"
                        value={formMonth}
                        onChange={(e) => {
                          setFormMonth(e.target.value);
                          setSelectedPreset('custom');
                          updateScheduleFromParts(formMinute, formHour, formDom, e.target.value, formDow);
                        }}
                        className="w-full text-center px-2 py-1.5 text-xs font-mono font-bold rounded-lg border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900 text-slate-900 dark:text-white"
                      />
                    </div>
                    <div>
                      <label className="block text-[11px] font-semibold text-slate-500 mb-1">
                        {t('cron.cron_dow')}
                      </label>
                      <input
                        type="text"
                        value={formDow}
                        onChange={(e) => {
                          setFormDow(e.target.value);
                          setSelectedPreset('custom');
                          updateScheduleFromParts(formMinute, formHour, formDom, formMonth, e.target.value);
                        }}
                        className="w-full text-center px-2 py-1.5 text-xs font-mono font-bold rounded-lg border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-900 text-slate-900 dark:text-white"
                      />
                    </div>
                  </div>

                  {/* Human preview */}
                  <div className="pt-2 border-t border-slate-200 dark:border-slate-800/80">
                    <div className="flex items-center gap-2">
                      <span className="text-[11px] text-slate-500">{t('cron.human_preview_label')}</span>
                      {validationError ? (
                        <span className="text-xs font-semibold text-rose-600 dark:text-rose-400 flex items-center gap-1">
                          <AlertCircle className="w-3.5 h-3.5" />
                          {validationError}
                        </span>
                      ) : (
                        <span className="text-xs font-semibold text-emerald-600 dark:text-emerald-400">
                          {humanTranslation || 'Setiap menit'}
                        </span>
                      )}
                    </div>
                  </div>
                </div>

                {/* Working Directory & Timeout */}
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-4">
                  <div>
                    <label className="block text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-1.5">
                      {t('cron.form_workdir_label')}
                    </label>
                    <input
                      type="text"
                      value={formWorkDir}
                      onChange={(e) => setFormWorkDir(e.target.value)}
                      placeholder={t('cron.form_workdir_placeholder')}
                      className="w-full px-3.5 py-2 text-sm font-mono rounded-xl border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-950 text-slate-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500"
                    />
                  </div>
                  <div>
                    <label className="block text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-1.5">
                      {t('cron.form_timeout_label')}
                    </label>
                    <input
                      type="number"
                      min="10"
                      max="86400"
                      value={formTimeoutSecs}
                      onChange={(e) => setFormTimeoutSecs(Number(e.target.value))}
                      className="w-full px-3.5 py-2 text-sm rounded-xl border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-950 text-slate-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500"
                    />
                  </div>
                </div>

                {/* Optional Description */}
                <div>
                  <label className="block text-xs font-bold uppercase tracking-wider text-slate-700 dark:text-slate-300 mb-1.5">
                    {t('cron.col_name')} (Keterangan)
                  </label>
                  <input
                    type="text"
                    value={formDescription}
                    onChange={(e) => setFormDescription(e.target.value)}
                    placeholder="Keterangan singkat fungsi tugas..."
                    className="w-full px-3.5 py-2 text-sm rounded-xl border border-slate-200 dark:border-slate-800 bg-white dark:bg-slate-950 text-slate-900 dark:text-white focus:outline-none focus:ring-2 focus:ring-indigo-500/20 focus:border-indigo-500"
                  />
                </div>

                {/* Enable Switch */}
                <div className="flex items-center justify-between pt-2">
                  <div>
                    <span className="text-sm font-semibold text-slate-800 dark:text-slate-200">
                      {t('cron.form_enabled_label')}
                    </span>
                    <p className="text-xs text-slate-400">
                      Timer systemd akan langsung diaktifkan di server setelah disimpan
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={() => setFormEnabled(!formEnabled)}
                    role="switch"
                    aria-checked={formEnabled}
                    className={`relative inline-flex h-6 w-11 flex-shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none ${
                      formEnabled ? 'bg-indigo-600' : 'bg-slate-300 dark:bg-slate-700'
                    }`}
                  >
                    <span
                      className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow-sm ring-0 transition duration-200 ease-in-out ${
                        formEnabled ? 'translate-x-5' : 'translate-x-0'
                      }`}
                    />
                  </button>
                </div>

                {/* Footer Buttons */}
              </div>
              <div className="flex items-center justify-end gap-3 pt-4 border-t border-slate-200 dark:border-slate-800 dialog-footer">
                <button
                  type="button"
                  onClick={() => setIsModalOpen(false)}
                  className="px-4 py-2 rounded-xl text-sm font-semibold text-slate-700 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors"
                >
                  {t('common.cancel', 'Batal')}
                </button>
                <button
                  type="submit"
                  disabled={isSaving || !!validationError}
                  className="inline-flex items-center gap-2 px-5 py-2 rounded-xl text-sm font-semibold text-white bg-indigo-600 hover:bg-indigo-500 active:scale-[0.98] transition-all disabled:opacity-50"
                >
                  {isSaving && <Loader2 className="w-4 h-4 animate-spin" />}
                  <span>{editingJob ? t('common.save', 'Simpan Perubahan') : t('cron.add_job')}</span>
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* Modal: Execution History & Logs */}
      {historyJob && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-sm animate-in fade-in"
          onDismiss={() => setHistoryJob(null)}
        >
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl w-full max-w-3xl shadow-2xl overflow-hidden flex flex-col max-h-[85vh] dialog-panel">
            <div className="flex items-center justify-between px-6 py-4 border-b border-slate-200 dark:border-slate-800 dialog-header">
              <div className="flex items-center gap-2">
                <Terminal className="w-5 h-5 text-indigo-500" />
                <h3 className="text-lg font-bold text-slate-900 dark:text-white">
                  {historyJob.name} — {t('cron.history_modal_title')}
                </h3>
              </div>
              <button
                onClick={() => setHistoryJob(null)}
                className="p-1 rounded-lg text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 transition-colors"
              >
                <X className="w-5 h-5" />
              </button>
            </div>

            {/* Tab navigation */}
            <div className="flex border-b border-slate-200 dark:border-slate-800 px-6">
              <button
                onClick={() => setHistoryActiveTab('runs')}
                className={`py-3 px-4 text-sm font-semibold border-b-2 transition-all ${
                  historyActiveTab === 'runs'
                    ? 'border-indigo-600 text-indigo-600 dark:text-indigo-400'
                    : 'border-transparent text-slate-500 hover:text-slate-700 dark:hover:text-slate-300'
                }`}
              >
                {t('cron.tab_runs')}
              </button>
              <button
                onClick={() => setHistoryActiveTab('logs')}
                className={`py-3 px-4 text-sm font-semibold border-b-2 transition-all ${
                  historyActiveTab === 'logs'
                    ? 'border-indigo-600 text-indigo-600 dark:text-indigo-400'
                    : 'border-transparent text-slate-500 hover:text-slate-700 dark:hover:text-slate-300'
                }`}
              >
                {t('cron.tab_logs')}
              </button>
            </div>

            <div className="p-6 overflow-y-auto flex-1 dialog-body">
              {isLoadingHistory ? (
                <div className="flex items-center justify-center py-12 text-slate-400">
                  <Loader2 className="w-6 h-6 animate-spin text-indigo-500 mr-2" />
                  <span>{t('common.loading', 'Memuat...')}</span>
                </div>
              ) : historyActiveTab === 'runs' ? (
                jobRuns.length === 0 ? (
                  <div className="text-center py-12 text-slate-400">
                    <AlertCircle className="w-8 h-8 mx-auto mb-2 opacity-50" />
                    <p className="text-sm">{t('cron.no_runs_yet')}</p>
                  </div>
                ) : (
                  <div className="overflow-x-auto">
                    <table className="w-full text-left text-xs">
                      <thead className="bg-slate-50 dark:bg-slate-800/50 text-slate-500 border-b border-slate-200 dark:border-slate-800">
                        <tr>
                          <th className="py-2.5 px-3">{t('cron.col_started_at')}</th>
                          <th className="py-2.5 px-3">{t('cron.col_duration')}</th>
                          <th className="py-2.5 px-3">{t('cron.col_status')}</th>
                          <th className="py-2.5 px-3">{t('cron.col_exit_code')}</th>
                          <th className="py-2.5 px-3">{t('cron.col_output')}</th>
                        </tr>
                      </thead>
                      <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                        {jobRuns.map((r) => (
                          <tr key={r.id}>
                            <td className="py-2.5 px-3 font-mono">
                              {new Date(r.started_at * 1000).toLocaleString()}
                            </td>
                            <td className="py-2.5 px-3">{r.duration_ms ? `${r.duration_ms} ms` : '-'}</td>
                            <td className="py-2.5 px-3">
                              <span
                                className={`px-2 py-0.5 rounded-full font-semibold ${
                                  r.status === 'success'
                                    ? 'bg-emerald-100 text-emerald-700 dark:bg-emerald-950 dark:text-emerald-300'
                                    : 'bg-rose-100 text-rose-700 dark:bg-rose-950 dark:text-rose-300'
                                }`}
                              >
                                {r.status}
                              </span>
                            </td>
                            <td className="py-2.5 px-3 font-mono">{r.exit_code !== null ? r.exit_code : '-'}</td>
                            <td
                              className="py-2.5 px-3 max-w-xs font-mono truncate"
                              title={r.stdout || r.stderr || ''}
                            >
                              {r.stdout || r.stderr || '-'}
                            </td>
                          </tr>
                        ))}
                      </tbody>
                    </table>
                  </div>
                )
              ) : (
                <div className="space-y-3">
                  <div className="flex justify-end">
                    <button
                      onClick={refreshLogsOnly}
                      className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-lg text-xs font-medium text-slate-700 dark:text-slate-300 bg-slate-100 dark:bg-slate-800 hover:bg-slate-200 dark:hover:bg-slate-700 transition-colors"
                    >
                      <RotateCw className="w-3.5 h-3.5" />
                      <span>{t('cron.refresh_logs')}</span>
                    </button>
                  </div>
                  <div className="p-4 rounded-xl bg-slate-950 text-slate-200 font-mono text-xs overflow-x-auto max-h-96 whitespace-pre-wrap select-all">
                    {jobLogs.length > 0 ? (
                      jobLogs.join('\n')
                    ) : (
                      <span className="text-slate-500">Tidak ada log systemd tercatat untuk unit ini.</span>
                    )}
                  </div>
                </div>
              )}
            </div>
          </div>
        </ModalOverlay>
      )}

      {/* Modal: Delete Confirmation */}
      {deletingJob && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-slate-900/60 backdrop-blur-sm animate-in fade-in"
          onDismiss={() => setDeletingJob(null)}
        >
          <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-2xl w-full max-w-md p-6 shadow-2xl dialog-panel">
            <div className="flex items-center gap-3 text-rose-600 dark:text-rose-400 mb-3 dialog-header">
              <div className="p-2.5 rounded-xl bg-rose-50 dark:bg-rose-950/60">
                <Trash2 className="w-6 h-6" />
              </div>
              <h3 className="text-lg font-bold text-slate-900 dark:text-white">{t('cron.delete_confirm_title')}</h3>
            </div>
            <div className="dialog-body ">
              <p className="text-sm text-slate-600 dark:text-slate-400 mb-6">
                {t('cron.delete_confirm_desc')}{' '}
                <strong className="text-slate-900 dark:text-white font-semibold">{deletingJob.name}</strong>.
              </p>
            </div>
            <div className="flex items-center justify-end gap-3 dialog-footer">
              <button
                type="button"
                onClick={() => setDeletingJob(null)}
                className="px-4 py-2 rounded-xl text-sm font-semibold text-slate-700 dark:text-slate-300 hover:bg-slate-100 dark:hover:bg-slate-800 transition-colors"
              >
                {t('common.cancel', 'Batal')}
              </button>
              <button
                type="button"
                onClick={handleDeleteConfirm}
                disabled={isDeleting}
                className="inline-flex items-center gap-2 px-4 py-2 rounded-xl text-sm font-semibold text-white bg-rose-600 hover:bg-rose-500 active:scale-[0.98] transition-all disabled:opacity-50"
              >
                {isDeleting && <Loader2 className="w-4 h-4 animate-spin" />}
                <span>{t('cron.delete')}</span>
              </button>
            </div>
          </div>
        </ModalOverlay>
      )}
    </div>
  );
};
