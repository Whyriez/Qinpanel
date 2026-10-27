import { ModalOverlay } from '../components/ui/ModalOverlay';
import React, { useState, useEffect, useCallback } from 'react';
import { useTranslation } from 'react-i18next';
import {
  Layers,
  Plus,
  Edit2,
  Trash2,
  RefreshCw,
  Cpu,
  HardDrive,
  Globe,
  Terminal,
  Database,
  CheckCircle2,
  AlertTriangle,
  X,
  Loader2,
  Sliders,
  Shield,
} from 'lucide-react';
import { Plan, CreatePlanRequest, UpdatePlanRequest, ShellPolicy } from '../types/api';
import { fetchPlans, createPlan, updatePlan, deletePlan } from '../api/client';
import { ConfirmDangerDialog } from '../components/ui/ConfirmDangerDialog';
import { AiPackageAssistant } from '../components/plans/AiPackageAssistant';
import { aiDraftFields, type AiRecommendation } from '../api/ai';

export const PlansPage: React.FC = () => {
  const { i18n } = useTranslation();
  const en = i18n.language.startsWith('en');
  const [plans, setPlans] = useState<Plan[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const [successMessage, setSuccessMessage] = useState<string | null>(null);

  // Dialog state
  const [isCreateOpen, setIsCreateOpen] = useState(false);
  const [isAiOpen, setIsAiOpen] = useState(false);
  const [aiSource, setAiSource] = useState<string | null>(null);
  const [editingPlan, setEditingPlan] = useState<Plan | null>(null);
  const [deletingPlan, setDeletingPlan] = useState<Plan | null>(null);

  // Form state
  const [formData, setFormData] = useState({
    name: '',
    description: '',
    cpu_quota_percent: 100,
    memory_limit_mb: 512,
    memory_high_mb: 400,
    tasks_max: 100,
    io_weight: 100,
    disk_limit_mb: 2048,
    inode_limit: 100000,
    max_sites: 5,
    max_domains: 10,
    max_databases: 5,
    max_cron_jobs: 10,
    max_terminal_sessions: 2,
    max_upload_mb: 50,
    shell_policy: 'none' as ShellPolicy,
    allow_external_upstream: false,
  });
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [formError, setFormError] = useState<string | null>(null);

  const loadPlans = useCallback(async () => {
    setIsLoading(true);
    setErrorMessage(null);
    try {
      const data = await fetchPlans();
      setPlans(data);
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal memuat daftar paket hosting';
      setErrorMessage(msg);
    } finally {
      setIsLoading(false);
    }
  }, []);

  useEffect(() => {
    loadPlans();
  }, [loadPlans]);

  const openCreateDialog = () => {
    setAiSource(null);
    setFormData({
      name: '',
      description: '',
      cpu_quota_percent: 100,
      memory_limit_mb: 512,
      memory_high_mb: 400,
      tasks_max: 100,
      io_weight: 100,
      disk_limit_mb: 2048,
      inode_limit: 100000,
      max_sites: 5,
      max_domains: 10,
      max_databases: 5,
      max_cron_jobs: 10,
      max_terminal_sessions: 2,
      max_upload_mb: 50,
      shell_policy: 'none',
      allow_external_upstream: false,
    });
    setFormError(null);
    setIsCreateOpen(true);
  };

  const useAiDraft = (result: AiRecommendation) => {
    const fields = aiDraftFields(result);
    openCreateDialog();
    setFormData(previous => ({ ...previous, ...fields,
      name: result.preset.demand.usage === 'personal' ? (en ? 'Personal plan' : 'Paket pribadi') : (en ? 'Application plan' : 'Paket aplikasi'),
      description: result.summary }));
    setAiSource(`${result.provider === 'openai' ? 'OpenAI' : 'Gemini'} · ${result.model}`);
    setIsAiOpen(false);
  };

  const openEditDialog = (plan: Plan) => {
    setAiSource(null);
    setEditingPlan(plan);
    setFormData({
      name: plan.name,
      description: plan.description || '',
      cpu_quota_percent: plan.cpu_quota_percent,
      memory_limit_mb: Math.round(plan.memory_limit_bytes / (1024 * 1024)),
      memory_high_mb: plan.memory_high_bytes ? Math.round(plan.memory_high_bytes / (1024 * 1024)) : 0,
      tasks_max: plan.tasks_max,
      io_weight: plan.io_weight,
      disk_limit_mb: Math.round(plan.disk_limit_bytes / (1024 * 1024)),
      inode_limit: plan.inode_limit,
      max_sites: plan.max_sites,
      max_domains: plan.max_domains,
      max_databases: plan.max_databases,
      max_cron_jobs: plan.max_cron_jobs,
      max_terminal_sessions: plan.max_terminal_sessions,
      max_upload_mb: Math.round(plan.max_upload_bytes / (1024 * 1024)),
      shell_policy: plan.shell_policy,
      allow_external_upstream: plan.allow_external_upstream,
    });
    setFormError(null);
  };

  const handleFormSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!formData.name.trim()) {
      setFormError('Nama paket wajib diisi');
      return;
    }
    setIsSubmitting(true);
    setFormError(null);

    const payload: CreatePlanRequest = {
      name: formData.name.trim(),
      description: formData.description.trim() || undefined,
      cpu_quota_percent: Number(formData.cpu_quota_percent),
      memory_limit_bytes: Number(formData.memory_limit_mb) * 1024 * 1024,
      memory_high_bytes: formData.memory_high_mb > 0 ? Number(formData.memory_high_mb) * 1024 * 1024 : undefined,
      tasks_max: Number(formData.tasks_max),
      io_weight: Number(formData.io_weight),
      disk_limit_bytes: Number(formData.disk_limit_mb) * 1024 * 1024,
      inode_limit: Number(formData.inode_limit),
      max_sites: Number(formData.max_sites),
      max_domains: Number(formData.max_domains),
      max_databases: Number(formData.max_databases),
      max_cron_jobs: Number(formData.max_cron_jobs),
      max_terminal_sessions: Number(formData.max_terminal_sessions),
      max_upload_bytes: Number(formData.max_upload_mb) * 1024 * 1024,
      shell_policy: formData.shell_policy,
      allow_external_upstream: formData.allow_external_upstream,
    };

    try {
      if (editingPlan) {
        await updatePlan(editingPlan.id, payload as UpdatePlanRequest);
        setSuccessMessage(`Paket "${editingPlan.name}" berhasil diperbarui. Perubahan limit langsung diterapkan ke tenant aktif.`);
        setEditingPlan(null);
      } else {
        await createPlan(payload);
        setSuccessMessage(`Paket baru "${formData.name}" berhasil dibuat.`);
        setIsCreateOpen(false);
      }
      await loadPlans();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal menyimpan paket';
      setFormError(msg);
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleDeletePlan = async () => {
    if (!deletingPlan) return;
    try {
      await deletePlan(deletingPlan.id);
      setSuccessMessage(`Paket "${deletingPlan.name}" berhasil dihapus.`);
      setDeletingPlan(null);
      await loadPlans();
    } catch (err: unknown) {
      const msg = err instanceof Error ? err.message : 'Gagal menghapus paket';
      setErrorMessage(msg);
      setDeletingPlan(null);
    }
  };

  const formatSize = (bytes: number): string => {
    if (bytes >= 1024 * 1024 * 1024) {
      return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
    }
    return `${Math.round(bytes / (1024 * 1024))} MB`;
  };

  const renderShellBadge = (policy: ShellPolicy) => {
    switch (policy) {
      case 'full':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono bg-emerald-500/10 text-emerald-600 dark:text-emerald-400 border border-emerald-500/20">
            <Terminal className="w-3 h-3" /> Shell interaktif
          </span>
        );
      case 'sftp':
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono bg-amber-500/10 text-amber-600 dark:text-amber-400 border border-amber-500/20">
            <Shield className="w-3 h-3" /> SFTP saja
          </span>
        );
      default:
        return (
          <span className="inline-flex items-center gap-1 px-2 py-0.5 rounded text-[11px] font-mono bg-[var(--raised)] text-[var(--ink-muted)] border border-[var(--line)]">
            Tanpa akses shell
          </span>
        );
    }
  };

  return (
    <div className="space-y-6">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <h1 className="text-xl font-bold tracking-tight text-[var(--ink)] flex items-center gap-2">
            <Layers className="w-6 h-6 text-[var(--primary)]" />
            Paket Hosting (Hosting Plans)
          </h1>
          <p className="text-xs text-[var(--ink-muted)] mt-1">
            Kelola template limit sumber daya cgroup v2, kuota disk, dan batasan operasional tenant.
          </p>
        </div>

        <div className="flex flex-wrap items-center gap-2">
          <button type="button" className="button-secondary ai-package-launcher" onClick={() => setIsAiOpen(true)}>{en ? 'Help size with AI' : 'Bantu atur dengan AI'}</button>
          <button
            onClick={loadPlans}
            disabled={isLoading}
            className="p-2 border border-[var(--line)] rounded-[var(--radius-sm)] bg-[var(--surface)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--raised)] transition-colors cursor-pointer disabled:opacity-50"
            title="Segarkan daftar paket"
          >
            <RefreshCw className={`w-4 h-4 ${isLoading ? 'animate-spin' : ''}`} />
          </button>
          <button
            onClick={openCreateDialog}
            className="flex items-center gap-2 px-3.5 py-2 bg-[var(--primary)] text-white text-xs font-semibold rounded-[var(--radius-sm)] hover:opacity-90 active:scale-95 transition-all shadow-xs cursor-pointer"
          >
            <Plus className="w-4 h-4" />
            <span>Buat Paket Baru</span>
          </button>
        </div>
      </div>

      {/* Alerts */}
      {successMessage && (
        <div className="p-3.5 bg-emerald-500/10 border border-emerald-500/20 rounded-[var(--radius-sm)] flex items-center justify-between text-xs text-emerald-700 dark:text-emerald-300">
          <div className="flex items-center gap-2">
            <CheckCircle2 className="w-4 h-4 shrink-0 text-emerald-600 dark:text-emerald-400" />
            <span>{successMessage}</span>
          </div>
          <button
            onClick={() => setSuccessMessage(null)}
            className="text-emerald-600 dark:text-emerald-400 hover:opacity-75 cursor-pointer"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      )}

      {errorMessage && (
        <div className="p-3.5 bg-[var(--danger)]/10 border border-[var(--danger)]/20 rounded-[var(--radius-sm)] flex items-center justify-between text-xs text-[var(--danger)]">
          <div className="flex items-center gap-2">
            <AlertTriangle className="w-4 h-4 shrink-0" />
            <span>{errorMessage}</span>
          </div>
          <button
            onClick={() => setErrorMessage(null)}
            className="hover:opacity-75 cursor-pointer"
          >
            <X className="w-3.5 h-3.5" />
          </button>
        </div>
      )}

      {/* Grid of Plans */}
      {isLoading ? (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
          {[1, 2, 3].map((i) => (
            <div
              key={i}
              className="p-5 border border-[var(--line)] rounded-[var(--radius-md)] bg-[var(--surface)] animate-pulse space-y-3"
            >
              <div className="h-5 bg-[var(--raised)] rounded w-1/2" />
              <div className="h-3 bg-[var(--raised)] rounded w-3/4" />
              <div className="h-20 bg-[var(--raised)] rounded" />
            </div>
          ))}
        </div>
      ) : plans.length === 0 ? (
        <div className="p-12 text-center border border-[var(--line)] rounded-[var(--radius-md)] bg-[var(--surface)]">
          <Layers className="w-10 h-10 mx-auto text-[var(--ink-muted)] mb-3 opacity-40" />
          <h3 className="text-sm font-semibold text-[var(--ink)]">Belum ada paket hosting</h3>
          <p className="text-xs text-[var(--ink-muted)] mt-1 max-w-sm mx-auto">
            Buat paket hosting pertama untuk mulai mendefinisikan isolasi resource dan limit operasional tenant.
          </p>
          <button
            onClick={openCreateDialog}
            className="mt-4 inline-flex items-center gap-2 px-3 py-1.5 bg-[var(--primary)] text-white text-xs font-medium rounded-[var(--radius-sm)] cursor-pointer"
          >
            <Plus className="w-4 h-4" />
            <span>Buat Paket Sekarang</span>
          </button>
        </div>
      ) : (
        <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-5">
          {plans.map((plan) => {
            const isDefault = plan.name.toLowerCase() === 'default';

            return (
              <div
                key={plan.id}
                className="flex flex-col justify-between p-5 border border-[var(--line)] rounded-[var(--radius-md)] bg-[var(--surface)] hover:border-[var(--line-strong)] transition-all shadow-xs relative overflow-hidden"
              >
                {/* Default badge strip */}
                {isDefault && (
                  <div className="absolute top-0 right-0">
                    <span className="bg-[var(--primary)] text-white text-[10px] font-bold uppercase tracking-wider px-3 py-0.5 rounded-bl shadow-xs">
                      Default
                    </span>
                  </div>
                )}

                <div>
                  {/* Title & Description */}
                  <div className="pr-12">
                    <h3 className="text-base font-bold text-[var(--ink)] flex items-center gap-2">
                      {plan.name}
                    </h3>
                    <p className="text-xs text-[var(--ink-muted)] mt-1 line-clamp-2 min-h-[32px]">
                      {plan.description || 'Tidak ada deskripsi paket.'}
                    </p>
                  </div>

                  {/* Badges */}
                  <div className="flex flex-wrap items-center gap-1.5 mt-3 pt-3 border-t border-[var(--line)]">
                    {renderShellBadge(plan.shell_policy)}
                    {plan.allow_external_upstream && (
                      <span className="px-2 py-0.5 rounded text-[11px] font-mono bg-blue-500/10 text-blue-600 dark:text-blue-400 border border-blue-500/20">
                        Upstream Eksternal
                      </span>
                    )}
                  </div>

                  {/* Resource Limits List */}
                  <div className="mt-4 space-y-2 text-xs">
                    <div className="flex items-center justify-between p-2 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]/50">
                      <span className="text-[var(--ink-muted)] flex items-center gap-1.5">
                        <Cpu className="w-3.5 h-3.5 text-[var(--primary)]" />
                        CPU Quota:
                      </span>
                      <span className="font-mono font-semibold text-[var(--ink)]">
                        {plan.cpu_quota_percent > 0 ? `${plan.cpu_quota_percent}% (quota)` : 'Tidak terbatas'}
                      </span>
                    </div>

                    <div className="flex items-center justify-between p-2 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]/50">
                      <span className="text-[var(--ink-muted)] flex items-center gap-1.5">
                        <Sliders className="w-3.5 h-3.5 text-purple-500" />
                        Memori RAM:
                      </span>
                      <span className="font-mono font-semibold text-[var(--ink)]">
                        {formatSize(plan.memory_limit_bytes)}
                        {plan.memory_high_bytes ? (
                          <span className="text-[10px] text-[var(--ink-muted)] ml-1 font-normal">
                            (throttled {formatSize(plan.memory_high_bytes)})
                          </span>
                        ) : null}
                      </span>
                    </div>

                    <div className="flex items-center justify-between p-2 rounded-[var(--radius-sm)] bg-[var(--raised)] border border-[var(--line)]/50">
                      <span className="text-[var(--ink-muted)] flex items-center gap-1.5">
                        <HardDrive className="w-3.5 h-3.5 text-amber-500" />
                        Penyimpanan Disk:
                      </span>
                      <span className="font-mono font-semibold text-[var(--ink)]">
                        {formatSize(plan.disk_limit_bytes)}
                        <span className="text-[10px] text-[var(--ink-muted)] ml-1 font-normal">
                          ({plan.inode_limit.toLocaleString()} file)
                        </span>
                      </span>
                    </div>

                    <div className="grid grid-cols-2 gap-2 pt-1 text-[11px]">
                      <div className="flex items-center justify-between px-2 py-1 rounded bg-[var(--raised)]/50">
                        <span className="text-[var(--ink-muted)] flex items-center gap-1">
                          <Globe className="w-3 h-3" /> Situs Max:
                        </span>
                        <span className="font-mono font-medium text-[var(--ink)]">{plan.max_sites}</span>
                      </div>
                      <div className="flex items-center justify-between px-2 py-1 rounded bg-[var(--raised)]/50">
                        <span className="text-[var(--ink-muted)] flex items-center gap-1">
                          <Database className="w-3 h-3" /> Database Max:
                        </span>
                        <span className="font-mono font-medium text-[var(--ink)]">{plan.max_databases}</span>
                      </div>
                    </div>
                  </div>
                </div>

                {/* Card Actions */}
                <div className="flex items-center justify-between gap-2 mt-5 pt-3 border-t border-[var(--line)]">
                  <span className="text-[10px] font-mono text-[var(--ink-muted)]">
                    pids.max: {plan.tasks_max}
                  </span>

                  <div className="flex items-center gap-1.5">
                    <button
                      onClick={() => openEditDialog(plan)}
                      className="flex items-center gap-1 px-2.5 py-1 text-xs font-medium border border-[var(--line)] rounded-[var(--radius-sm)] bg-[var(--surface)] hover:bg-[var(--raised)] text-[var(--ink)] transition-colors cursor-pointer"
                    >
                      <Edit2 className="w-3 h-3" />
                      <span>Edit</span>
                    </button>
                    <button
                      onClick={() => setDeletingPlan(plan)}
                      disabled={isDefault}
                      className="p-1 text-xs text-[var(--danger)] hover:bg-[var(--danger)]/10 rounded-[var(--radius-sm)] transition-colors cursor-pointer disabled:opacity-30 disabled:cursor-not-allowed"
                      title={isDefault ? 'Paket default tidak dapat dihapus' : 'Hapus paket'}
                    >
                      <Trash2 className="w-3.5 h-3.5" />
                    </button>
                  </div>
                </div>
              </div>
            );
          })}
        </div>
      )}

      {/* Modal Dialog: Create / Edit Plan */}
      {isAiOpen && <AiPackageAssistant onClose={() => setIsAiOpen(false)} onUse={useAiDraft} />}
      {(isCreateOpen || editingPlan) && (
        <ModalOverlay
          className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs overflow-y-auto"
          onDismiss={() => {
            setIsCreateOpen(false);
            setEditingPlan(null);
          }}
        >
          <div className="w-full max-w-2xl bg-[var(--surface)] border border-[var(--line)] rounded-[var(--radius-lg)] shadow-2xl p-6 text-left my-8 dialog-panel">
            <div className="flex items-center justify-between pb-4 border-b border-[var(--line)] dialog-header">
              <div>
                <h2 className="text-base font-bold text-[var(--ink)] flex items-center gap-2">
                  <Sliders className="w-5 h-5 text-[var(--primary)]" />
                  {editingPlan ? `Edit Paket: ${editingPlan.name}` : 'Buat Paket Hosting Baru'}
                </h2>
                <p className="text-xs text-[var(--ink-muted)] mt-0.5">
                  {editingPlan
                    ? 'Perubahan batas resource cgroup v2 akan otomatis disinkronkan ke tenant yang memakai paket ini.'
                    : 'Tentukan limit isolasi kernel, kuota file, dan fitur untuk paket ini.'}
                </p>
              </div>
              <button
                onClick={() => {
                  setIsCreateOpen(false);
                  setEditingPlan(null);
                }}
                className="p-1 rounded text-[var(--ink-muted)] hover:text-[var(--ink)] cursor-pointer"
              >
                <X className="w-5 h-5" />
              </button>
            </div>
            <form onSubmit={handleFormSubmit} className="mt-4 dialog-form">
              <div className="dialog-body space-y-5">
                {aiSource && <p role="status" className="rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--surface-sunken)] p-3 text-sm">{en ? 'Draft from' : 'Draft dari'} {aiSource}. {en ? 'Review and edit before saving. These figures are not reserved capacity; existing tenants are unchanged.' : 'Periksa dan edit sebelum menyimpan. Angka ini bukan reservasi kapasitas; tenant existing tidak berubah.'}</p>}
                {formError && (
                  <div className="mt-4 p-3 bg-[var(--danger)]/10 border border-[var(--danger)]/20 rounded-[var(--radius-sm)] flex items-center gap-2 text-xs text-[var(--danger)]">
                    <AlertTriangle className="w-4 h-4 shrink-0" />
                    <span>{formError}</span>
                  </div>
                )}

                {/* Seksi 1: Identitas Paket */}
                <div>
                  <h3 className="text-xs font-semibold text-[var(--ink-muted)] uppercase tracking-wider mb-2">
                    1. Informasi Paket
                  </h3>
                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">
                        Nama Paket <span className="text-[var(--danger)]">*</span>
                      </label>
                      <input
                        type="text"
                        required
                        value={formData.name}
                        onChange={(e) => setFormData({ ...formData, name: e.target.value })}
                        placeholder="e.g. starter, business, pro"
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                      />
                    </div>

                    <div>
                      <label htmlFor="plan-shell-policy" className="block text-xs font-medium text-[var(--ink)] mb-1">
                        Akses shell
                      </label>
                      <select
                        id="plan-shell-policy"
                        value={formData.shell_policy}
                        onChange={(e) => {
                          const policy = e.target.value;
                          if (policy === 'none' || policy === 'sftp' || policy === 'full') {
                            setFormData({ ...formData, shell_policy: policy });
                          }
                        }}
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                      >
                        <option value="none">Tanpa akses shell</option>
                        <option value="sftp">SFTP saja (transfer file)</option>
                        <option value="full">Shell interaktif (SSH dan terminal)</option>
                      </select>
                    </div>

                    <div className="sm:col-span-2">
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">Deskripsi</label>
                      <input
                        type="text"
                        value={formData.description}
                        onChange={(e) => setFormData({ ...formData, description: e.target.value })}
                        placeholder="Deskripsi singkat paket untuk administrator"
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                      />
                    </div>
                  </div>
                </div>

                {/* Seksi 2: Cgroup v2 Resource Slice */}
                <div>
                  <h3 className="text-xs font-semibold text-[var(--ink-muted)] uppercase tracking-wider mb-2 flex items-center gap-1.5">
                    <Cpu className="w-3.5 h-3.5 text-[var(--primary)]" />
                    2. Batasan Kernel & Resource Slice (cgroup v2)
                  </h3>
                  <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">CPU Quota (%)</label>
                      <input
                        type="number"
                        min="0"
                        max="1600"
                        step="10"
                        value={formData.cpu_quota_percent}
                        onChange={(e) => setFormData({ ...formData, cpu_quota_percent: Number(e.target.value) })}
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)] font-mono"
                      />
                      <span className="text-[10px] text-[var(--ink-muted)]">100 = 1 core penuh. 0 = unmetered.</span>
                    </div>

                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">
                        Memory Max (MB) <span className="text-[var(--danger)]">*</span>
                      </label>
                      <input
                        type="number"
                        min="64"
                        required
                        value={formData.memory_limit_mb}
                        onChange={(e) => setFormData({ ...formData, memory_limit_mb: Number(e.target.value) })}
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)] font-mono"
                      />
                      <span className="text-[10px] text-[var(--ink-muted)]">OOM killer aktif jika terlampaui.</span>
                    </div>

                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">Memory High (MB)</label>
                      <input
                        type="number"
                        min="0"
                        value={formData.memory_high_mb}
                        onChange={(e) => setFormData({ ...formData, memory_high_mb: Number(e.target.value) })}
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)] font-mono"
                      />
                      <span className="text-[10px] text-[var(--ink-muted)]">Reclaim throttling watermark.</span>
                    </div>

                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">Tasks Max (pids.max)</label>
                      <input
                        type="number"
                        min="10"
                        max="10000"
                        value={formData.tasks_max}
                        onChange={(e) => setFormData({ ...formData, tasks_max: Number(e.target.value) })}
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)] font-mono"
                      />
                      <span className="text-[10px] text-[var(--ink-muted)]">Mencegah serangan fork-bomb.</span>
                    </div>

                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">IO Weight (1-10000)</label>
                      <input
                        type="number"
                        min="1"
                        max="10000"
                        value={formData.io_weight}
                        onChange={(e) => setFormData({ ...formData, io_weight: Number(e.target.value) })}
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)] font-mono"
                      />
                      <span className="text-[10px] text-[var(--ink-muted)]">Prioritas IO disk (default 100).</span>
                    </div>
                  </div>
                </div>

                {/* Seksi 3: Kuota Disk Linux */}
                <div>
                  <h3 className="text-xs font-semibold text-[var(--ink-muted)] uppercase tracking-wider mb-2 flex items-center gap-1.5">
                    <HardDrive className="w-3.5 h-3.5 text-amber-500" />
                    3. Kuota Disk Linux (ext4 project/user quota)
                  </h3>
                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">
                        Kapasitas Disk (MB) <span className="text-[var(--danger)]">*</span>
                      </label>
                      <input
                        type="number"
                        min="100"
                        required
                        value={formData.disk_limit_mb}
                        onChange={(e) => setFormData({ ...formData, disk_limit_mb: Number(e.target.value) })}
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)] font-mono"
                      />
                      <span className="text-[10px] text-[var(--ink-muted)]">
                        Contoh: 1024 (1 GB), 5120 (5 GB), 10240 (10 GB)
                      </span>
                    </div>

                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">
                        Batas Inode (Jumlah File) <span className="text-[var(--danger)]">*</span>
                      </label>
                      <input
                        type="number"
                        min="1000"
                        required
                        value={formData.inode_limit}
                        onChange={(e) => setFormData({ ...formData, inode_limit: Number(e.target.value) })}
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)] font-mono"
                      />
                      <span className="text-[10px] text-[var(--ink-muted)]">
                        Maksimal berkas file/folder yang boleh dibuat.
                      </span>
                    </div>
                  </div>
                </div>

                {/* Seksi 4: Batasan Aplikasi */}
                <div>
                  <h3 className="text-xs font-semibold text-[var(--ink-muted)] uppercase tracking-wider mb-2">
                    4. Batasan Layanan & Fitur
                  </h3>
                  <div className="grid grid-cols-2 sm:grid-cols-4 gap-3">
                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">Max Situs</label>
                      <input
                        type="number"
                        min="1"
                        value={formData.max_sites}
                        onChange={(e) => setFormData({ ...formData, max_sites: Number(e.target.value) })}
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)] font-mono"
                      />
                    </div>

                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">Max Domain</label>
                      <input
                        type="number"
                        min="1"
                        value={formData.max_domains}
                        onChange={(e) => setFormData({ ...formData, max_domains: Number(e.target.value) })}
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)] font-mono"
                      />
                    </div>

                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">Max Database</label>
                      <input
                        type="number"
                        min="0"
                        value={formData.max_databases}
                        onChange={(e) => setFormData({ ...formData, max_databases: Number(e.target.value) })}
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)] font-mono"
                      />
                    </div>

                    <div>
                      <label className="block text-xs font-medium text-[var(--ink)] mb-1">Max Cron Jobs</label>
                      <input
                        type="number"
                        min="0"
                        value={formData.max_cron_jobs}
                        onChange={(e) => setFormData({ ...formData, max_cron_jobs: Number(e.target.value) })}
                        className="w-full px-3 py-1.5 text-xs bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-sm)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)] font-mono"
                      />
                    </div>
                  </div>

                  <div className="mt-3 flex items-center gap-2">
                    <input
                      type="checkbox"
                      id="allow_external_upstream"
                      checked={formData.allow_external_upstream}
                      onChange={(e) => setFormData({ ...formData, allow_external_upstream: e.target.checked })}
                      className="rounded border-[var(--line)] text-[var(--primary)] focus:ring-[var(--primary)] cursor-pointer"
                    />
                    <label htmlFor="allow_external_upstream" className="text-xs text-[var(--ink)] cursor-pointer">
                      Izinkan Reverse Proxy ke Upstream Eksternal (IP luar server)
                    </label>
                  </div>
                </div>

                {/* Form Buttons */}
              </div>
              <div className="pt-4 border-t border-[var(--line)] flex items-center justify-end gap-2 dialog-footer">
                <button
                  type="button"
                  onClick={() => {
                    setIsCreateOpen(false);
                    setEditingPlan(null);
                  }}
                  className="px-4 py-2 border border-[var(--line)] rounded-[var(--radius-sm)] text-xs font-medium text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--raised)] cursor-pointer"
                >
                  Batal
                </button>
                <button
                  type="submit"
                  disabled={isSubmitting}
                  className="flex items-center gap-1.5 px-4 py-2 bg-[var(--primary)] text-white text-xs font-semibold rounded-[var(--radius-sm)] hover:opacity-90 active:scale-95 transition-all shadow-xs cursor-pointer disabled:opacity-50"
                >
                  {isSubmitting && <Loader2 className="w-3.5 h-3.5 animate-spin" />}
                  <span>{editingPlan ? 'Perbarui Paket' : 'Simpan Paket'}</span>
                </button>
              </div>
            </form>
          </div>
        </ModalOverlay>
      )}

      {/* Danger Confirm Dialog for Delete */}
      <ConfirmDangerDialog
        isOpen={!!deletingPlan}
        title={`Hapus Paket ${deletingPlan?.name}`}
        description={`Apakah Anda yakin ingin menghapus paket "${deletingPlan?.name}"? Paket yang sedang digunakan oleh tenant aktif tidak dapat dihapus.`}
        confirmMatchText={deletingPlan?.name || ''}
        confirmButtonLabel="Hapus Paket"
        onConfirm={handleDeletePlan}
        onClose={() => setDeletingPlan(null)}
      />
    </div>
  );
};
