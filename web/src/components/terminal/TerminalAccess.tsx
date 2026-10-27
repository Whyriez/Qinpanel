import { useCallback, useEffect, useState } from 'react';
import { fetchTenantShellPolicy } from '../../api/client';
import { useAuth } from '../../context/AuthContext';
import { RouteLink } from '../../lib/router';
import type { TenantShellPolicyResponse } from '../../types/api';

export interface TerminalAccessState {
  status: 'loading' | 'allowed' | 'blocked' | 'error';
  policy?: TenantShellPolicyResponse;
  error?: string;
}

export function useTerminalAccess(tenant?: string, enabled = true) {
  const { user } = useAuth();
  const targetId = tenant === undefined ? user?.id : Number(tenant);
  const [revision, setRevision] = useState(0);
  const [snapshot, setSnapshot] = useState<
    TerminalAccessState & { targetId?: number; revision: number }
  >({ status: 'loading', revision: -1 });
  const refresh = useCallback(() => setRevision((value) => value + 1), []);

  useEffect(() => {
    if (!enabled) {
      setSnapshot({ targetId, revision, status: 'loading' });
      return;
    }
    let active = true;
    if (!targetId || !Number.isSafeInteger(targetId) || targetId < 1) {
      setSnapshot({
        targetId,
        revision,
        status: 'error',
        error: 'Pilih tenant yang valid untuk membuka terminal.',
      });
      return;
    }
    setSnapshot({ targetId, revision, status: 'loading' });
    fetchTenantShellPolicy(targetId)
      .then((policy) => {
        if (
          policy.user_id !== targetId ||
          !['none', 'sftp', 'full'].includes(policy.effective_policy)
        ) {
          throw new Error('Kebijakan terminal tidak sesuai dengan tenant yang dipilih.');
        }
        if (active)
          setSnapshot({
            targetId,
            revision,
            policy,
            status: policy.effective_policy === 'full' ? 'allowed' : 'blocked',
          });
      })
      .catch((error: unknown) => {
        if (active)
          setSnapshot({
            targetId,
            revision,
            status: 'error',
            error: error instanceof Error ? error.message : 'Gagal memeriksa akses terminal.',
          });
      });
    return () => {
      active = false;
    };
  }, [enabled, targetId, revision]);

  const state: TerminalAccessState =
    snapshot.targetId === targetId && snapshot.revision === revision
      ? snapshot
      : { status: 'loading' };
  return { state, refresh, canOpenTerminal: enabled && state.status === 'allowed' };
}

export function TerminalAccessNotice({
  state,
  onRefresh,
}: {
  state: TerminalAccessState;
  onRefresh: () => void;
}) {
  const { user } = useAuth();
  const policy = state.policy;
  if (state.status === 'allowed') return null;
  return (
    <div className="flex h-full min-h-48 flex-col items-center justify-center gap-3 overflow-y-auto p-6 text-center text-sm">
      {state.status === 'loading' ? (
        <p role="status">Memeriksa kebijakan akses terminal...</p>
      ) : (
        <>
          <h2 className="font-semibold text-[var(--ink)]">
            {state.status === 'error'
              ? 'Akses terminal belum dapat diperiksa'
              : 'Terminal belum diaktifkan'}
          </h2>
          {state.status === 'error' ? (
            <p role="alert" className="max-w-lg text-[var(--ink-muted)]">
              {state.error}
            </p>
          ) : (
            <div role="status" className="max-w-lg space-y-2 text-[var(--ink-muted)]">
              <p>
                {policy?.effective_policy === 'sftp'
                  ? 'Tenant ini memakai mode SFTP saja. Transfer berkas diizinkan, tetapi shell interaktif belum diaktifkan.'
                  : 'Kebijakan tenant ini belum mengizinkan akses shell interaktif.'}
              </p>
              <p>
                Kebijakan aktif:{' '}
                <strong className="font-mono text-[var(--ink)]">{policy?.effective_policy}</strong>{' '}
                ·{' '}
                {policy?.inherited_from_plan
                  ? policy.plan_policy
                    ? 'Mengikuti paket'
                    : 'Default tanpa paket'
                  : 'Pengaturan khusus tenant'}
              </p>
              <p>
                {user?.role === 'admin'
                  ? 'Pilih full di pengaturan shell tenant untuk mengaktifkan Web Terminal dan SSH interaktif.'
                  : 'Hubungi administrator untuk mengaktifkan kebijakan full pada akun ini.'}
              </p>
            </div>
          )}
          <div className="flex flex-wrap justify-center gap-3">
            {user?.role === 'admin' && state.status === 'blocked' && policy && (
              <RouteLink
                href={`/tenants/${policy.user_id}/overview?section=shell`}
                className="rounded border border-[var(--line)] px-3 py-2 font-medium text-[var(--primary)]"
              >
                Atur akses terminal
              </RouteLink>
            )}
            <button
              type="button"
              onClick={onRefresh}
              className="rounded border border-[var(--line)] px-3 py-2 text-[var(--ink)]"
            >
              Periksa ulang akses
            </button>
          </div>
        </>
      )}
    </div>
  );
}
