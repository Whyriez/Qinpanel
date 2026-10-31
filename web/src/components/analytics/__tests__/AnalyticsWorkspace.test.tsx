import { beforeEach, describe, expect, it, vi } from 'vitest';
import { renderToStaticMarkup } from 'react-dom/server';
import { AnalyticsPage } from '../../../pages/AnalyticsPage';
import { AnalyticsPlot, analyticsX } from '../AnalyticsPlot';
import { AnalyticsMetric, analyticsBytes } from '../AnalyticsUi';
import { ServerSystemMetrics } from '../ServerSystemMetrics';
import { SiteAnalyticsTab } from '../../sites/SiteAnalyticsTab';
import type { ServerTimeseriesPointDto } from '../../../types/api';
import i18n from '../../../i18n';
const auth = vi.hoisted(() => ({ role: 'user' }));
vi.mock('../../../context/AuthContext', () => ({ useAuth: () => ({ user: { id: 2, role: auth.role } }) }));
beforeEach(async () => { auth.role = 'user'; await i18n.changeLanguage('id'); });

describe('Analytics workspace presentation', () => {
  it('has clear scope controls for admins without server controls for tenants', () => {
    const tenant = renderToStaticMarkup(<AnalyticsPage />);
    expect(tenant).toContain('Situs yang dianalisis');
    expect(tenant).toContain('Memuat analitik');
    expect(tenant).not.toContain('Lingkup analitik');
    auth.role = 'admin';
    expect(renderToStaticMarkup(<AnalyticsPage />)).toContain('Lingkup analitik');
  });
  it('uses compact accessible range controls and hides options progressively', () => {
    const html = renderToStaticMarkup(<SiteAnalyticsTab siteId={7} siteDomain="project.test" />);
    expect(html).toContain('aria-label="Periode analitik"');
    expect(html).toContain('aria-pressed="true"');
    expect(html).toContain('Opsi tampilan');
    expect(html).not.toContain('Bandingkan periode lalu');
    expect(html).toContain('aria-disabled="true"');
  });
  it('keeps request growth neutral and never labels missing data as zero', () => {
    const html = renderToStaticMarkup(<AnalyticsMetric label="Requests" value="12,345" change={20} />);
    expect(html).toContain('+20.0%');
    expect(html).not.toMatch(/is-danger|text-emerald|text-rose/);
    expect(analyticsBytes(Number.NaN)).toBe('—');
    expect(analyticsBytes(0)).toBe('0 B');
    expect(analyticsBytes(1048576)).toBe('1 MiB');
  });
  it('positions points using timestamps and centers single samples', () => {
    expect(analyticsX(100, 100, 100, 600)).toBe(300);
    expect(analyticsX(110, 100, 200, 600)).toBe(60);
  });
  it('renders a single zero sample as a point, with keyboard controls and correct readout', () => {
    const html = renderToStaticMarkup(<AnalyticsPlot title="Sample chart" timestamps={[1700000000]} series={[{ label: 'Requests', values: [0], color: 'var(--primary)' }]} area emptyMessage="Empty" />);
    expect(html).toContain('<circle');
    expect(html).toContain('tabindex="0"');
    expect(html).toContain('panah kiri dan kanan');
    expect(html).toContain('Requests<strong>0</strong>');
    expect(html).not.toContain('NaN');
    expect(html).not.toContain('Infinity');
  });
  it('keeps chart title and explanation when there are no samples', () => {
    const html = renderToStaticMarkup(<AnalyticsPlot title="Request traffic" timestamps={[]} series={[]} emptyMessage="No samples" />);
    expect(html).toContain('Request traffic');
    expect(html).toContain('No samples');
    expect(html).not.toContain('<svg viewBox');
  });
  it('shows actual server resource samples and keeps memory, network and I/O discoverable', () => {
    const point = { timestamp: 1700000000, cpu_avg: 23.5, cpu_max: 50, memory_used_avg_bytes: 1073741824, memory_used_max_bytes: 2147483648, swap_used_avg_bytes: 0, load1_avg: 1.2, load1_max: 2, disk_used_avg_bytes: 10737418240, net_rx_avg_bytes_sec: 1000, net_tx_avg_bytes_sec: 2000, io_read_avg_bytes_sec: 100, io_write_avg_bytes_sec: 200 } satisfies ServerTimeseriesPointDto;
    const html = renderToStaticMarkup(<ServerSystemMetrics points={[point]} />);
    expect(html).toContain('23.5%'); expect(html).toContain('1 GiB');
    expect(html).toContain('Jenis metrik server'); expect(html).toContain('I/O disk');
    expect(html).not.toContain('RAM %');
  });
  it('supports English labels for the analytics workspace and charts', async () => {
    await i18n.changeLanguage('en');
    expect(renderToStaticMarkup(<AnalyticsPage />)).toContain('Understand your site traffic');
    expect(renderToStaticMarkup(<ServerSystemMetrics points={[]} />)).toContain('No server metric samples');
  });
});
