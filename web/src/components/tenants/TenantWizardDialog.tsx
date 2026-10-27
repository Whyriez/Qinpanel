import { ModalOverlay } from '../ui/ModalOverlay';
import React, { useState, useEffect, useRef } from 'react';
import {
  X,
  User,
  Shield,
  CheckCircle2,
  Cpu,
  HardDrive,
  Activity,
  ArrowRight,
  ArrowLeft,
  Loader2,
  AlertCircle,
} from 'lucide-react';
import { Plan, CreateTenantRequest, TenantDetailResponse } from '../../types/api';
import { fetchPlans, createTenant } from '../../api/client';

export interface TenantWizardDialogProps {
  isOpen: boolean;
  onClose: () => void;
  onSuccess: (tenant: TenantDetailResponse) => void;
}

export const TenantWizardDialog: React.FC<TenantWizardDialogProps> = ({
  isOpen,
  onClose,
  onSuccess,
}) => {
  const [step, setStep] = useState<1 | 2 | 3>(1);
  const [slug, setSlug] = useState('');
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [selectedPlanId, setSelectedPlanId] = useState<number | null>(null);

  const [plans, setPlans] = useState<Plan[]>([]);
  const [isLoadingPlans, setIsLoadingPlans] = useState(false);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [errorMessage, setErrorMessage] = useState<string | null>(null);
  const stepContent = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (isOpen) stepContent.current?.focus({ preventScroll: true });
  }, [isOpen, step]);

  useEffect(() => {
    if (isOpen) {
      setStep(1);
      setSlug('');
      setEmail('');
      setPassword('');
      setSelectedPlanId(null);
      setErrorMessage(null);

      // Load plans
      setIsLoadingPlans(true);
      fetchPlans()
        .then((data) => {
          setPlans(data);
          if (data.length > 0) {
            setSelectedPlanId(data[0].id);
          }
        })
        .catch((err) => {
          setErrorMessage('Gagal memuat daftar paket hosting: ' + (err.message || 'Error'));
        })
        .finally(() => {
          setIsLoadingPlans(false);
        });
    }
  }, [isOpen]);

  if (!isOpen) return null;

  const linuxUsername = slug ? `p_${slug.toLowerCase().replace(/[^a-z0-9_-]/g, '')}` : 'p_<slug>';
  const selectedPlan = plans.find((p) => p.id === selectedPlanId);

  const formatBytes = (bytes: number): string => {
    if (bytes >= 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024 * 1024)).toFixed(0)} GB`;
    if (bytes >= 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(0)} MB`;
    return `${bytes} B`;
  };

  const handleNextStep1 = (e: React.FormEvent) => {
    e.preventDefault();
    if (!slug.trim()) {
      setErrorMessage('Nama slug tenant wajib diisi.');
      return;
    }
    if (slug.length < 3) {
      setErrorMessage('Slug minimal 3 karakter.');
      return;
    }
    if (!password || password.length < 8) {
      setErrorMessage('Kata sandi minimal 8 karakter.');
      return;
    }
    setErrorMessage(null);
    setStep(2);
  };

  const handleNextStep2 = () => {
    if (!selectedPlanId) {
      setErrorMessage('Pilih salah satu paket hosting untuk melanjutkan.');
      return;
    }
    setErrorMessage(null);
    setStep(3);
  };

  const handleConfirmCreate = async () => {
    if (!selectedPlanId) return;
    setIsSubmitting(true);
    setErrorMessage(null);

    const payload: CreateTenantRequest = {
      username_slug: slug.trim(),
      email: email.trim() || undefined,
      password,
      plan_id: selectedPlanId,
    };

    try {
      const res = await createTenant(payload);
      onSuccess(res);
      onClose();
    } catch (err: unknown) {
      const e = err as Error;
      setErrorMessage(e.message || 'Gagal menambahkan tenant');
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <ModalOverlay
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-xs"
      role="dialog"
      aria-modal="true"
      aria-labelledby="wizard-title"
      onDismiss={onClose}
      dismissDisabled={isSubmitting}
    >
      <div className="tenant-wizard w-full max-w-xl rounded-[var(--radius-lg)] border border-[var(--line)] bg-[var(--surface)] p-6 shadow-[var(--shadow-floating)] animate-in fade-in zoom-in-95 duration-150 flex flex-col max-h-[90vh] dialog-panel">
        {/* Header */}
        <div className="flex items-center justify-between pb-3 border-b border-[var(--line)] dialog-header">
          <div>
            <h2 id="wizard-title" className="text-sm font-bold text-[var(--ink)]">
              Tambah Tenant Baru
            </h2>
            <p className="text-xs text-[var(--ink-muted)]">
              Konfigurasi akun Linux mandiri dengan batasan isolasi cgroup v2 & kuota disk.
            </p>
          </div>
          <button
            onClick={onClose}
            disabled={isSubmitting}
            className="p-1 rounded-[var(--radius-sm)] text-[var(--ink-muted)] hover:text-[var(--ink)] hover:bg-[var(--raised)] cursor-pointer disabled:opacity-50"
            title="Batal"
          >
            <X className="w-4 h-4" />
          </button>
        </div>

        {/* Stepper Wizard Bar */}
        <ol className="dialog-stepper">
          {[
            { num: 1, title: 'Identitas' },
            { num: 2, title: 'Paket Hosting' },
            { num: 3, title: 'Konfirmasi' },
          ].map((item) => (
            <li key={item.num} aria-current={step === item.num ? 'step' : undefined}>
              <div className="flex items-center gap-2">
                <div
                  className={`w-6 h-6 rounded-full flex items-center justify-center text-xs font-bold transition-colors ${
                    step === item.num
                      ? 'bg-[var(--primary)] text-white'
                      : step > item.num
                        ? 'bg-[var(--success)] text-white'
                        : 'bg-[var(--line)] text-[var(--ink-muted)]'
                  }`}
                >
                  {step > item.num ? <CheckCircle2 className="w-3.5 h-3.5" /> : item.num}
                </div>
                <span
                  className={`text-xs font-medium ${
                    step === item.num ? 'text-[var(--primary)] font-semibold' : 'text-[var(--ink-muted)]'
                  }`}
                >
                  {item.title}
                </span>
              </div>
            </li>
          ))}
        </ol>

        {/* Step Content */}
        <div key={step} ref={stepContent} tabIndex={-1} role="group" aria-label={`Langkah ${step} dari 3`} className="flex-1 overflow-y-auto p-1.5 dialog-body dialog-step">
          {/* Error notification banner */}
          {errorMessage && (
            <div className="mb-4 p-3 rounded-[var(--radius-sm)] bg-[var(--danger)]/10 border border-[var(--danger)]/30 text-[var(--danger)] text-xs flex items-center gap-2">
              <AlertCircle className="w-4 h-4 shrink-0" />
              <span>{errorMessage}</span>
            </div>
          )}

          {/* STEP 1: Identitas Tenant */}
          {step === 1 && (
            <form id="wizard-step-1" onSubmit={handleNextStep1} className="space-y-4">
              <div>
                <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                  Nama Slug Tenant <span className="text-[var(--danger)]">*</span>
                </label>
                <input
                  type="text"
                  required
                  placeholder="contoh: tokokami atau klien-a"
                  value={slug}
                  onChange={(e) => setSlug(e.target.value.toLowerCase().replace(/[^a-z0-9_-]/g, ''))}
                  className="w-full px-3 py-2 text-xs rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                />
                <div className="mt-1.5 flex flex-wrap items-center gap-1.5 break-all text-[11px] text-[var(--ink-muted)] font-mono">
                  <span>Nama Linux:</span>
                  <span className="font-semibold text-[var(--primary)]">{linuxUsername}</span>
                  <span className="text-[10px] text-[var(--ink-muted)] opacity-70">
                    (UID rentang 10000–59999)
                  </span>
                </div>
              </div>

              <div>
                <label className="block text-xs font-semibold text-[var(--ink)] mb-1">Email (Opsional)</label>
                <input
                  type="email"
                  placeholder="kontak@klien.com"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  className="w-full px-3 py-2 text-xs rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                />
              </div>

              <div>
                <label className="block text-xs font-semibold text-[var(--ink)] mb-1">
                  Kata Sandi Akun Panel <span className="text-[var(--danger)]">*</span>
                </label>
                <input
                  type="password"
                  required
                  placeholder="Minimal 8 karakter kuat"
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  className="w-full px-3 py-2 text-xs rounded-[var(--radius-sm)] border border-[var(--line)] bg-[var(--bg)] text-[var(--ink)] focus:outline-none focus:border-[var(--primary)]"
                />
                <span className="text-[11px] text-[var(--ink-muted)] mt-1 block">
                  User Linux sistem dibuat tanpa password interaktif. Kata sandi ini khusus untuk login Web Panel.
                </span>
              </div>
            </form>
          )}

          {/* STEP 2: Pilih Paket Hosting */}
          {step === 2 && (
            <div className="space-y-3">
              <div className="text-xs text-[var(--ink-muted)] mb-2">
                Pilih paket hosting untuk mengonfigurasi batas CPU, RAM, Disk, dan izin akses terminal.
              </div>

              {isLoadingPlans ? (
                <div className="p-8 text-center text-xs text-[var(--ink-muted)] flex items-center justify-center gap-2">
                  <Loader2 className="w-4 h-4 animate-spin text-[var(--primary)]" />
                  <span>Memuat pilihan paket...</span>
                </div>
              ) : plans.length === 0 ? (
                <div className="p-6 text-center text-xs text-[var(--ink-muted)] bg-[var(--raised)] border border-[var(--line)] rounded-[var(--radius-md)]">
                  Belum ada paket hosting yang dibuat. Buat paket terlebih dahulu di menu Paket Hosting.
                </div>
              ) : (
                <div className="grid grid-cols-1 gap-2.5">
                  {plans.map((p) => {
                    const isSelected = selectedPlanId === p.id;
                    return (
                      <div
                        key={p.id}
                        onClick={() => setSelectedPlanId(p.id)}
                        className={`p-3.5 rounded-[var(--radius-md)] border cursor-pointer transition-all ${
                          isSelected
                            ? 'border-[var(--primary)] bg-[var(--primary)]/5 shadow-xs'
                            : 'border-[var(--line)] bg-[var(--raised)] hover:border-[var(--line)]/80'
                        }`}
                      >
                        <div className="flex flex-wrap items-center justify-between gap-2 mb-2">
                          <div className="flex items-center gap-2">
                            <input
                              type="radio"
                              name="plan_selection"
                              checked={isSelected}
                              onChange={() => setSelectedPlanId(p.id)}
                              className="text-[var(--primary)]"
                            />
                            <span className="text-xs font-bold text-[var(--ink)]">{p.name}</span>
                          </div>
                          <span className="text-[11px] font-mono px-2 py-0.5 rounded-[var(--radius-sm)] bg-[var(--bg)] border border-[var(--line)] text-[var(--ink-muted)]">
                            Shell: {p.shell_policy}
                          </span>
                        </div>

                        {p.description && (
                          <p className="text-[11px] text-[var(--ink-muted)] mb-3">{p.description}</p>
                        )}

                        <div className="grid grid-cols-1 sm:grid-cols-3 gap-2 text-[11px] text-[var(--ink-muted)] pt-2 border-t border-[var(--line)]/60 font-mono">
                          <div className="flex items-center gap-1.5">
                            <Cpu className="w-3.5 h-3.5 text-[var(--primary)]" />
                            <span>{p.cpu_quota_percent}% CPU</span>
                          </div>
                          <div className="flex items-center gap-1.5">
                            <Activity className="w-3.5 h-3.5 text-[var(--primary)]" />
                            <span>{formatBytes(p.memory_limit_bytes)} RAM</span>
                          </div>
                          <div className="flex items-center gap-1.5">
                            <HardDrive className="w-3.5 h-3.5 text-[var(--primary)]" />
                            <span>{formatBytes(p.disk_limit_bytes)} Disk</span>
                          </div>
                        </div>
                      </div>
                    );
                  })}
                </div>
              )}
            </div>
          )}

          {/* STEP 3: Tinjau & Konfirmasi */}
          {step === 3 && selectedPlan && (
            <div className="space-y-4">
              <div className="p-3.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] space-y-2.5">
                <h3 className="text-xs font-bold text-[var(--ink)] border-b border-[var(--line)] pb-2 flex items-center gap-2">
                  <User className="w-4 h-4 text-[var(--primary)]" />
                  <span>Ringkasan Identitas Akun</span>
                </h3>
                <div className="tenant-wizard-summary grid grid-cols-1 sm:grid-cols-2 gap-x-3 gap-y-2 text-xs">
                  <span className="text-[var(--ink-muted)]">Nama Login Panel:</span>
                  <span className="font-semibold text-[var(--ink)] font-mono">{slug}</span>

                  <span className="text-[var(--ink-muted)]">User Linux Host:</span>
                  <span className="font-semibold text-[var(--primary)] font-mono">{linuxUsername}</span>

                  <span className="text-[var(--ink-muted)]">Email:</span>
                  <span className="text-[var(--ink)]">{email || '—'}</span>

                  <span className="text-[var(--ink-muted)]">Shell Policy:</span>
                  <span className="font-mono text-[var(--ink)]">{selectedPlan.shell_policy}</span>
                </div>
              </div>

              <div className="p-3.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--raised)] space-y-2.5">
                <h3 className="text-xs font-bold text-[var(--ink)] border-b border-[var(--line)] pb-2 flex items-center gap-2">
                  <Shield className="w-4 h-4 text-[var(--primary)]" />
                  <span>Limit & Isolasi Paket ({selectedPlan.name})</span>
                </h3>
                <div className="tenant-wizard-summary grid grid-cols-1 sm:grid-cols-2 gap-x-3 gap-y-2 text-xs">
                  <span className="text-[var(--ink-muted)]">Cgroup CPU Quota:</span>
                  <span className="font-semibold text-[var(--ink)] font-mono">
                    {selectedPlan.cpu_quota_percent}% (per core)
                  </span>

                  <span className="text-[var(--ink-muted)]">Cgroup Memory Limit:</span>
                  <span className="font-semibold text-[var(--ink)] font-mono">
                    {formatBytes(selectedPlan.memory_limit_bytes)}
                  </span>

                  <span className="text-[var(--ink-muted)]">Maksimal Tasks (PIDs):</span>
                  <span className="font-semibold text-[var(--ink)] font-mono">
                    {selectedPlan.tasks_max} proses
                  </span>

                  <span className="text-[var(--ink-muted)]">Kuota Kuota Disk:</span>
                  <span className="font-semibold text-[var(--ink)] font-mono">
                    {formatBytes(selectedPlan.disk_limit_bytes)}
                  </span>

                  <span className="text-[var(--ink-muted)]">Maksimal Inode:</span>
                  <span className="font-semibold text-[var(--ink)] font-mono">
                    {selectedPlan.inode_limit.toLocaleString()} files
                  </span>
                </div>
              </div>
            </div>
          )}
        </div>

        {/* Footer Actions */}
        <div className="flex items-center justify-between pt-4 mt-2 border-t border-[var(--line)] dialog-footer">
          {step > 1 ? (
            <button
              type="button"
              onClick={() => setStep((s) => (s - 1) as 1 | 2)}
              disabled={isSubmitting}
              className="flex items-center gap-1.5 px-3 py-1.5 rounded-[var(--radius-sm)] border border-[var(--line)] text-xs font-medium text-[var(--ink)] hover:bg-[var(--raised)] cursor-pointer disabled:opacity-50"
            >
              <ArrowLeft className="w-3.5 h-3.5" />
              <span>Sebelumnya</span>
            </button>
          ) : (
            <div />
          )}

          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={onClose}
              disabled={isSubmitting}
              className="px-3.5 py-1.5 rounded-[var(--radius-sm)] border border-[var(--line)] text-xs font-medium text-[var(--ink)] hover:bg-[var(--raised)] cursor-pointer disabled:opacity-50"
            >
              Batal
            </button>

            {step === 1 && (
              <button
                type="submit"
                form="wizard-step-1"
                className="flex items-center gap-1.5 px-3.5 py-1.5 rounded-[var(--radius-sm)] bg-[var(--primary)] text-white text-xs font-semibold hover:opacity-90 cursor-pointer"
              >
                <span>Lanjut Pilih Paket</span>
                <ArrowRight className="w-3.5 h-3.5" />
              </button>
            )}

            {step === 2 && (
              <button
                type="button"
                onClick={handleNextStep2}
                disabled={!selectedPlanId}
                className="flex items-center gap-1.5 px-3.5 py-1.5 rounded-[var(--radius-sm)] bg-[var(--primary)] text-white text-xs font-semibold hover:opacity-90 cursor-pointer disabled:opacity-50"
              >
                <span>Tinjau Ringkasan</span>
                <ArrowRight className="w-3.5 h-3.5" />
              </button>
            )}

            {step === 3 && (
              <button
                type="button"
                onClick={handleConfirmCreate}
                disabled={isSubmitting}
                className="flex items-center gap-1.5 px-4 py-1.5 rounded-[var(--radius-sm)] bg-[var(--success)] text-white text-xs font-semibold hover:opacity-90 cursor-pointer disabled:opacity-50"
              >
                {isSubmitting ? (
                  <>
                    <Loader2 className="w-3.5 h-3.5 animate-spin" />
                    <span>Mempersiapkan Akun...</span>
                  </>
                ) : (
                  <>
                    <CheckCircle2 className="w-3.5 h-3.5" />
                    <span>Konfirmasi & Buat Tenant</span>
                  </>
                )}
              </button>
            )}
          </div>
        </div>
      </div>
    </ModalOverlay>
  );
};
