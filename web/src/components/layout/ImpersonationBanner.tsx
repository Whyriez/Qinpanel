import React, { useState } from 'react';
import { UserCheck, ArrowLeft, Loader2 } from 'lucide-react';
import { stopImpersonating } from '../../api/client';

export interface ImpersonationBannerProps {
  impersonatedBy: string;
  username: string;
  onStopped?: () => void;
}

export const ImpersonationBanner: React.FC<ImpersonationBannerProps> = ({
  impersonatedBy,
  username,
  onStopped,
}) => {
  const [isStopping, setIsStopping] = useState(false);

  const handleStop = async () => {
    setIsStopping(true);
    try {
      await stopImpersonating();
      if (onStopped) {
        onStopped();
      } else {
        window.location.reload();
      }
    } catch (err) {
      console.error('Failed to stop impersonation:', err);
      window.location.reload();
    } finally {
      setIsStopping(false);
    }
  };

  return (
    <aside
      aria-label="Notifikasi impersonasi"
      className="w-full bg-[var(--accent)] text-[var(--accent-foreground)] px-4 py-2 border-b border-[var(--line)] shadow-xs transition-all z-30"
    >
      <div className="max-w-[1280px] mx-auto flex items-center justify-between gap-4">
        <div className="flex items-center gap-2.5 text-xs font-medium">
          <UserCheck className="w-4 h-4 shrink-0" />
          <span>
            Mode Impersonasi Aktif: Anda sedang mengelola akun tenant{' '}
            <strong className="font-semibold underline decoration-2">{username}</strong> (diinisiasi oleh{' '}
            <strong className="font-semibold">{impersonatedBy}</strong>).
          </span>
        </div>

        <button
          onClick={handleStop}
          disabled={isStopping}
          className="flex items-center gap-1.5 px-3 py-1 bg-black/15 hover:bg-black/25 active:bg-black/35 text-xs font-semibold rounded-[var(--radius-sm)] border border-black/20 transition-colors cursor-pointer disabled:opacity-50"
          title="Kembali ke Akun Administrator"
        >
          {isStopping ? (
            <Loader2 className="w-3.5 h-3.5 animate-spin" />
          ) : (
            <ArrowLeft className="w-3.5 h-3.5" />
          )}
          <span>Kembali ke Admin</span>
        </button>
      </div>
    </aside>
  );
};
