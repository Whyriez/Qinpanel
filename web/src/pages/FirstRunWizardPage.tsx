import React from 'react';
import { Brand } from '../components/ui/Brand';

export interface FirstRunWizardPageProps {
  onFinish: () => void;
  onBackToLogin: () => void;
}

// Keep /setup bookmarks useful while retiring public token-based account creation.
export const FirstRunWizardPage: React.FC<FirstRunWizardPageProps> = ({ onBackToLogin }) => (
  <main className="auth-wizard min-h-screen flex items-center justify-center p-4 bg-[var(--bg)] text-[var(--ink)]">
    <section aria-labelledby="admin-access-title" className="w-full max-w-xl space-y-5 border border-[var(--line)] rounded-[var(--radius-md)] bg-[var(--surface)] p-6 sm:p-8">
      <Brand />
      <div className="space-y-2">
        <h1 id="admin-access-title" className="text-lg font-semibold">Akses administrator</h1>
        <p className="text-sm text-[var(--ink-muted)]">Installer membuat username dan password otomatis. Gunakan kredensial yang ditampilkan setelah instalasi berhasil untuk masuk ke panel.</p>
      </div>
      <div className="space-y-2 text-sm">
        <h2 className="font-medium">Lupa kredensial?</h2>
        <p>Masuk ke server melalui SSH, lalu jalankan:</p>
        <pre className="whitespace-pre-wrap break-words rounded-[var(--radius-sm)] bg-[var(--raised)] p-3 text-xs"><code>{'sudo panelctl admin list\nsudo panelctl admin reset-password --username NAMA'}</code></pre>
        <p className="text-[var(--ink-muted)]">Ganti NAMA dengan username dari daftar. Password baru ditampilkan sekali. Seluruh sesi dan API token akun dicabut; 2FA tetap aktif.</p>
      </div>
      <details className="text-sm space-y-2">
        <summary className="cursor-pointer font-medium">Administrator belum tersedia?</summary>
        <p>Jalankan <code>sudo panelctl admin bootstrap</code> melalui SSH. Perintah ini mempertahankan akun yang sudah ada.</p>
        <p>Bantuan lengkap: <code>panelctl --help</code> dan <code>panelctl admin --help</code>.</p>
      </details>
      <p className="text-sm text-[var(--ink-muted)]">Username dan password dapat diubah melalui Profil &amp; keamanan setelah masuk.</p>
      <button type="button" onClick={onBackToLogin} className="rounded-[var(--radius-sm)] bg-[var(--primary)] px-4 py-2 text-sm font-medium text-white focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[var(--primary)]">Kembali ke login</button>
    </section>
  </main>
);
