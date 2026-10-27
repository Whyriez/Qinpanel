import { describe, it, expect } from 'vitest';
import { renderToString } from 'react-dom/server';
import { AnalyticsRetentionCard } from '../AnalyticsRetentionCard';
import { AnalyticsRetentionResponse } from '../../../types/api';

describe('AnalyticsRetentionCard Component', () => {
  const mockRetentionData: AnalyticsRetentionResponse = {
    config: {
      raw_retention_secs: 7200, // 2 jam
      rollup_1m_retention_secs: 1209600, // 14 hari
      rollup_1h_retention_secs: 34560000, // 400 hari
      rollup_1d_retention_secs: null,
      security_events_retention_secs: 604800, // 7 hari
      max_db_size_bytes: 104857600, // 100 MB
      auto_vacuum: true,
    },
    storage: {
      main_db_size_bytes: 15728640, // 15 MB
      wal_size_bytes: 2097152, // 2 MB
      total_size_bytes: 17825792,
      page_size: 4096,
      page_count: 3840,
      freelist_count: 384,
      free_bytes: 1572864,
      fragmentation_ratio: 0.1, // 10%
      tables: [
        { table_name: 'server_metrics_raw', row_count: 50000 },
        { table_name: 'site_metrics_raw', row_count: 120000 },
        { table_name: 'access_log_security_events', row_count: 850 },
      ],
    },
    last_report: {
      started_at: 1700000000,
      completed_at: 1700000005,
      pruned_raw_records: 12500,
      pruned_rollup_1m_records: 300,
      pruned_rollup_1h_records: 50,
      pruned_rollup_1d_records: 0,
      pruned_security_events: 120,
      pruned_stale_tailer_records: 15,
      initial_db_size_bytes: 20971520,
      final_db_size_bytes: 17825792,
      bytes_reclaimed: 3145728, // ~3 MB
      vacuum_performed: true,
      wal_checkpoint_performed: true,
      is_emergency_pruning: false,
    },
    integrity: {
      is_healthy: true,
      checked_at: 1700000010,
      errors: [],
    },
  };

  it('renders retention overview, database size, and fragmentation ratio', () => {
    const html = renderToString(<AnalyticsRetentionCard initialData={mockRetentionData} />);

    expect(html).toContain('Siklus Data &amp; Retensi Basis Data');
    expect(html).toContain('15.00 MB'); // main db size
    expect(html).toContain('10.0'); // fragmentation ratio
    expect(html).toContain('Optimal (≤ 20%)');
    expect(html).toContain('Sehat (Valid)');
    expect(html).toContain('Konfigurasi Kebijakan Retensi');
  });

  it('renders last maintenance report summary with reclaimed space', () => {
    const html = renderToString(<AnalyticsRetentionCard initialData={mockRetentionData} />);

    expect(html).toContain('Ringkasan Pembersihan Terakhir');
    expect(html).toContain('3.00 MB'); // bytes reclaimed
    expect(html).toContain('12.500'); // formatted raw records count
    expect(html).toContain('Jalankan Pembersihan &amp; Optimasi');
    expect(html).toContain('Uji Integritas Basis Data');
  });

  it('renders emergency pruning badge when triggered under disk limit', () => {
    const emergencyData: AnalyticsRetentionResponse = {
      ...mockRetentionData,
      last_report: {
        ...mockRetentionData.last_report!,
        is_emergency_pruning: true,
      },
    };

    const html = renderToString(<AnalyticsRetentionCard initialData={emergencyData} />);
    expect(html).toContain('DARURAT (Batas Disk)');
  });

  it('displays corrupted integrity alert when database check has errors', () => {
    const corruptedData: AnalyticsRetentionResponse = {
      ...mockRetentionData,
      integrity: {
        is_healthy: false,
        checked_at: 1700000020,
        errors: ['Page 42 is corrupted', 'Index idx_site_metrics missing entries'],
      },
    };

    const html = renderToString(<AnalyticsRetentionCard initialData={corruptedData} />);
    expect(html).toContain('Korup');
    expect(html).toContain('Isu');
  });
});
