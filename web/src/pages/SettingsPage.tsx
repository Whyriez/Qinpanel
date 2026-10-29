import React from 'react';
import { useTranslation } from 'react-i18next';
import { navigate, useBrowserRoute } from '../lib/router';
import { ConnectivitySettings } from '../components/system/ConnectivitySettings';
import { Sliders, Cpu, Activity, ShieldCheck, Wrench, Globe, Archive, RefreshCw, Network } from 'lucide-react';
import '../styles/settings.css';
import { NetworkSettings } from '../components/system/NetworkSettings';
import { ServerOptimizerCard } from '../components/system/ServerOptimizerCard';
import { PanelMemoryProfileCard } from '../components/system/PanelMemoryProfileCard';
import { SecurityHardeningCard } from '../components/system/SecurityHardeningCard';
import { SystemReliabilityCard } from '../components/system/SystemReliabilityCard';
import { ServerBackupCard } from '../components/system/ServerBackupCard';
import { PanelUpdateCard } from '../components/system/PanelUpdateCard';
import { AiSettings } from '../components/system/AiSettings';

const settingGroups = [
  { id: 'server', label: 'Server', labelEn: 'Server', items: [
    { id: 'optimizer', label: 'Pengoptimal server', labelEn: 'Server optimizer', icon: Sliders },
    { id: 'memory_profile', label: 'Penggunaan memori', labelEn: 'Memory usage', icon: Activity },
    { id: 'info', label: 'Pedoman profil', labelEn: 'Profile guidelines', icon: Cpu },
    { id: 'ai', label: 'Integrasi AI', labelEn: 'AI integration', icon: Sliders },
  ] },
  { id: 'security', label: 'Keamanan dan jaringan', labelEn: 'Security and networking', items: [
    { id: 'security_hardening', label: 'Hardening & Firewall', labelEn: 'Hardening & Firewall', icon: ShieldCheck },
    { id: 'network', label: 'DNS dan Cloudflare', labelEn: 'DNS and Cloudflare', icon: Globe },
    { id: 'connectivity', label: 'Konektivitas', labelEn: 'Connectivity', icon: Network },
  ] },
  { id: 'maintenance', label: 'Pemeliharaan', labelEn: 'Maintenance', items: [
    { id: 'server_backup', label: 'Backup dan migrasi', labelEn: 'Backup and migration', icon: Archive },
    { id: 'updates', label: 'Pembaruan', labelEn: 'Updates', icon: RefreshCw },
    { id: 'reliability', label: 'Keandalan dan diagnostik', labelEn: 'Reliability and diagnostics', icon: Wrench },
  ] },
] as const;
type Section = typeof settingGroups[number]['items'][number]['id'];
const settingSections = settingGroups.flatMap(group => [...group.items]);

export const SettingsPage: React.FC = () => {
  const { i18n } = useTranslation();
  const en = i18n.language.startsWith('en');
  const route = useBrowserRoute();
  const currentSection = settingSections.find(item => item.id === route.search.get('section')) ?? settingGroups[0].items[0];
  const activeSection = currentSection.id;
  const currentLabel = en ? currentSection.labelEn : currentSection.label;
  const setActiveSection = (next: Section) => {
    const params = new URLSearchParams(route.search); params.set('section', next);
    navigate(`/settings?${params}`);
  };

  return (
    <div className="server-settings-page space-y-6" data-testid="settings-page">
      {/* Page Header */}
      <div>
        <h1 className="text-xl font-bold text-[var(--ink)]">
          Pengaturan server
        </h1>
        <p className="text-xs text-[var(--ink-muted)] mt-1">
          Kelola performa, keamanan, pemeliharaan, dan penggunaan sumber daya server.
        </p>
      </div>

      <div className="settings-mobile-navigation">
        <label htmlFor="server-settings-section" className="block text-xs font-semibold text-[var(--ink-muted)] mb-2">
          {en ? 'Settings section' : 'Bagian pengaturan'}
        </label>
        <select id="server-settings-section" value={activeSection} onChange={event => setActiveSection(event.target.value as Section)} aria-controls="server-settings-content">
          {settingGroups.map(group => (
            <optgroup key={group.id} label={en ? group.labelEn : group.label}>
              {group.items.map(item => <option key={item.id} value={item.id}>{en ? item.labelEn : item.label}</option>)}
            </optgroup>
          ))}
        </select>
      </div>

      <nav className="settings-desktop-navigation" aria-label={en ? 'Server settings sections' : 'Bagian pengaturan server'}>
        {settingGroups.map(group => (
          <div key={group.id}>
            <h2 className="text-xs font-semibold text-[var(--ink-muted)] mb-2">{en ? group.labelEn : group.label}</h2>
            <div className="space-y-1">
              {group.items.map(item => {
                const Icon = item.icon;
                return <button key={item.id} type="button" onClick={() => setActiveSection(item.id)} aria-pressed={activeSection === item.id} aria-controls="server-settings-content" data-testid={item.id === 'reliability' ? 'tab-reliability' : undefined}>
                  <Icon size={16} aria-hidden="true" />
                  <span>{en ? item.labelEn : item.label}</span>
                </button>;
              })}
            </div>
          </div>
        ))}
      </nav>

      {/* Section Content */}
      <section id="server-settings-content" className="server-settings-content" aria-label={currentLabel}>
      {activeSection === 'server_backup' && <ServerBackupCard />}
      {activeSection === 'ai' && <AiSettings />}
      {activeSection === 'updates' && <PanelUpdateCard />}
      {activeSection === 'connectivity' && <ConnectivitySettings />}
      {activeSection === 'network' && <NetworkSettings key={route.search.get('network_tab') ?? 'public'} initialTab={route.search.get('network_tab') === 'certificates' ? 'certificates' : 'public'} />}
      {activeSection === 'optimizer' && (
        <ServerOptimizerCard />
      )}

      {activeSection === 'security_hardening' && (
        <SecurityHardeningCard />
      )}

      {activeSection === 'reliability' && (
        <SystemReliabilityCard />
      )}

      {activeSection === 'memory_profile' && (
        <PanelMemoryProfileCard />
      )}

      {activeSection === 'info' && (
        <div className="border border-[var(--line)] bg-[var(--surface)] rounded-[var(--radius-md)] p-6 space-y-4">
          <div className="flex items-center gap-2">
            <Cpu className="w-5 h-5 text-[var(--primary)]" />
            <h2 className="text-base font-semibold text-[var(--ink)]">
              Profil performa server
            </h2>
          </div>
          <p className="text-xs text-[var(--ink-muted)] leading-relaxed">
            Sistem pengoptimal secara otomatis menghitung metrik perangkat keras host dan mengkategorikan ke dalam 4 tingkatan profil:
          </p>

          <div className="grid grid-cols-1 md:grid-cols-2 gap-4 text-xs">
            <div className="p-3.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface-sunken)] space-y-1.5">
              <span className="font-semibold text-[var(--ink)]">1. Micro (&lt; 2 GB RAM)</span>
              <p className="text-[var(--ink-muted)]">
                Didesain untuk VPS hemat resource (1-2 vCPU, RAM 512MB-1GB). Menggunakan alokasi buffer minimal, worker Nginx auto-capped, dan PHP-FPM ondemand dengan max_children rendah untuk mencegah Out-Of-Memory (OOM).
              </p>
            </div>

            <div className="p-3.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface-sunken)] space-y-1.5">
              <span className="font-semibold text-[var(--ink)]">2. Standard (2 - 4 GB RAM)</span>
              <p className="text-[var(--ink-muted)]">
                Profil seimbang untuk VPS hosting umum. Menggunakan koneksi worker menengah (2.048 koneksi), somaxconn 4.096, dan buffer fastcgi 64k.
              </p>
            </div>

            <div className="p-3.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface-sunken)] space-y-1.5">
              <span className="font-semibold text-[var(--ink)]">3. High Performance (4 - 16 GB RAM)</span>
              <p className="text-[var(--ink-muted)]">
                Untuk server production dengan lalu lintas tinggi. Mendukung hingga 4.096 koneksi per worker, somaxconn 16.384, tcp_tw_reuse aktif, dan cluster runtime multi-core.
              </p>
            </div>

            <div className="p-3.5 rounded-[var(--radius-md)] border border-[var(--line)] bg-[var(--surface-sunken)] space-y-1.5">
              <span className="font-semibold text-[var(--ink)]">4. Dedicated Large (&gt; 16 GB RAM)</span>
              <p className="text-[var(--ink-muted)]">
                Konfigurasi ultra-throughput untuk server bare-metal/dedicated. File descriptor 1.000.000, somaxconn 65.535, worker connections 8.192, dan alokasi buffer bervolume besar.
              </p>
            </div>
          </div>
        </div>
      )}
      </section>
    </div>
  );
};

export default SettingsPage;
