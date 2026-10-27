import { describe, it, expect, vi } from 'vitest';
import { renderToString } from 'react-dom/server';
import { TerminalTabs } from '../TerminalTabs';
import { TerminalDrawer } from '../TerminalDrawer';
import { TerminalPage } from '../../../pages/TerminalPage';
import { TerminalTabItem } from '../../../types/terminal';
import '../../../i18n';

vi.mock('../../../context/AuthContext', () => ({ useAuth: () => ({ user:{id:42,role:'user'},isLoading:false }) }));

// Mock API client
vi.mock('../../../api/client', () => ({
  createTerminalSession: vi.fn().mockResolvedValue({
    session_id: 'term-test-123',
    tenant_id: 1,
    cwd: '/var/www/test',
    cols: 80,
    rows: 24,
    created_at: 1700000000,
  }),
  closeTerminalSession: vi.fn().mockResolvedValue({ success: true }),
  fetchTerminalSessions: vi.fn().mockResolvedValue([]),
  getTerminalSession: vi.fn().mockResolvedValue(null),
  resizeTerminalSession: vi.fn().mockResolvedValue({ success: true }),
  fetchTenantShellPolicy: vi.fn(),
}));

describe('TerminalTabs Component', () => {
  const sampleTabs: TerminalTabItem[] = [
    {
      id: 'tab-1',
      title: 'bash #1',
      cwd: '/var/www/site1',
      status: 'connected',
    },
    {
      id: 'tab-2',
      title: 'bash #2',
      cwd: '/var/www/site2',
      status: 'connecting',
    },
    {
      id: 'tab-3',
      title: 'bash #3',
      cwd: '/var/www/site3',
      status: 'disconnected',
    },
  ];

  it('renders all tab titles and active indicators', () => {
    const html = renderToString(
      <TerminalTabs
        tabs={sampleTabs}
        activeTabId="tab-1"
        onSelectTab={vi.fn()}
        onNewTab={vi.fn()}
        onCloseTab={vi.fn()}
      />
    );

    expect(html).toContain('bash #1');
    expect(html).toContain('bash #2');
    expect(html).toContain('bash #3');
    // Active tab has primary border/bold styling
    expect(html).toContain('border-[var(--primary)]');
    // Connected tab status indicator
    expect(html).toContain('bg-emerald-500');
    // Connecting tab status indicator
    expect(html).toContain('bg-amber-500');
    // Disconnected tab status indicator
    expect(html).toContain('bg-rose-500');
  });

  it('renders control buttons: new tab, clear, search, maximize', () => {
    const html = renderToString(
      <TerminalTabs
        tabs={sampleTabs}
        activeTabId="tab-1"
        onSelectTab={vi.fn()}
        onNewTab={vi.fn()}
        onCloseTab={vi.fn()}
        onClearActiveTab={vi.fn()}
        onSearchToggle={vi.fn()}
        onToggleMaximize={vi.fn()}
        isMaximized={false}
      />
    );

    expect(html).toContain('Tab Baru');
    expect(html).toContain('Bersihkan layar (Ctrl+L)');
    expect(html).toContain('Cari teks (Ctrl+F)');
    expect(html).toContain('Perbesar layar penuh');
  });

  it('renders minimize button when maximized', () => {
    const html = renderToString(
      <TerminalTabs
        tabs={sampleTabs}
        activeTabId="tab-1"
        onSelectTab={vi.fn()}
        onNewTab={vi.fn()}
        onCloseTab={vi.fn()}
        isMaximized={true}
        onToggleMaximize={vi.fn()}
      />
    );

    expect(html).toContain('Perkecil jendela');
  });
});

describe('TerminalDrawer Component', () => {
  it('does not render content when isOpen is false', () => {
    const html = renderToString(
      <TerminalDrawer isOpen={false} onClose={vi.fn()} initialCwd="/var/www/test" />
    );

    expect(html).toBe('');
  });

  it('renders floating file terminal controls when isOpen is true', () => {
    const html = renderToString(
      <TerminalDrawer isOpen={true} onClose={vi.fn()} initialCwd="/var/www/test" />
    );

    expect(html).toContain('Tab Baru');
    expect(html).toContain('Tutup terminal');
    expect(html).toContain('Minimalkan terminal');
    expect(html).toContain('Maksimalkan terminal');
    expect(html).toContain('aria-modal="false"');
  });
});

describe('TerminalPage Component', () => {
  it('checks tenant policy before offering the initial terminal session', () => {
    const html = renderToString(<TerminalPage />);

    expect(html).toContain('Memeriksa kebijakan akses terminal...');
    expect(html).not.toContain('Buka Sesi Terminal');
  });
});
