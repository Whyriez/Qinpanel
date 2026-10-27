import { afterEach, describe, expect, it, vi } from 'vitest';
import { parseSystemInsights, parseRetention, parseMaintenance, parseIntegrity } from './analyticsContracts';
import { fetchSystemInsights, fetchAnalyticsRetention, updateAnalyticsRetention, triggerAnalyticsMaintenance, setCsrfToken } from './client';
import { summarizeSystemInsights } from '../components/analytics/systemInsights';
import type { Site } from '../types/api';

// Names and units match panel-api/src/analytics.rs and retention.rs DTOs.
const retention = {
  retention: { raw_retention_secs: 7200, one_minute_retention_secs: 1209600, one_hour_retention_secs: 34560000, one_day_retention_secs: null, security_events_retention_secs: 604800, max_db_size_bytes: 104857600, auto_vacuum: true },
  stats: { db_size_bytes: 15728640, wal_size_bytes: 2097152, total_file_size_bytes: 17825792, page_size: 4096, page_count: 3840, freelist_count: 384, free_bytes: 1572864, fragmentation_ratio: 0.1, max_db_size_bytes: 104857600, is_healthy: true, integrity_message: 'ok', table_stats: [{ table_name: 'server_metrics_raw', row_count: 10 }] },
  last_maintenance: null,
};
const maintenance = { timestamp: 1700000000, raw_records_pruned: 20, one_minute_records_pruned: 10, one_hour_records_pruned: 5, one_day_records_pruned: 1, security_events_pruned: 2, checkpoints_pruned: 3, total_records_pruned: 41, wal_checkpoint_executed: true, vacuum_executed: false, db_size_bytes_before: 2000, db_size_bytes_after: 1500, wal_size_bytes_before: 100, wal_size_bytes_after: 0, duration_ms: 250, emergency_pruning_triggered: false };
afterEach(() => { vi.unstubAllGlobals(); setCsrfToken(null); });

describe('host analytics contracts', () => {
  it('accepts the real empty insights response that previously crashed the host page', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify({ evaluated_at: 1, total_insights: 0, insights: [] }))));
    const result = await fetchSystemInsights();
    expect(summarizeSystemInsights(result, [])).toEqual({ total_critical: 0, total_warning: 0, site_insights: [] });
  });
  it('counts severity and preserves known, unknown, and host attribution', () => {
    const entry = { id: 'a', insight_type: 'latency_regression', severity: 'critical', title: 'Lambat', description: 'Latensi naik', detected_at: 1 };
    const response = parseSystemInsights({ evaluated_at: 1, total_insights: 3, insights: [{ ...entry, site_id: 7 }, { ...entry, id: 'b', site_id: 8, severity: 'warning' }, { ...entry, id: 'c', site_id: null, severity: 'info' }] });
    const summary = summarizeSystemInsights(response, [{ id: 7, primary_domain: 'example.test' } as Site]);
    expect(summary.total_critical).toBe(1);
    expect(summary.total_warning).toBe(1);
    expect(summary.site_insights.map(group => [group.site_name, group.can_review])).toEqual([['example.test', true], ['Situs #8', false], ['Server host', false]]);
  });
  it('maps actual retention/stats data before the card can store it', async () => {
    vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response(JSON.stringify(retention))));
    const data = await fetchAnalyticsRetention();
    expect(data.storage.main_db_size_bytes).toBe(15728640);
    expect(data.storage.tables[0].row_count).toBe(10);
    expect(data.config.rollup_1m_retention_secs).toBe(1209600);
    expect(data.config.rollup_1d_retention_secs).toBeNull();
    expect(data.last_report).toBeNull();
  });
  it('rejects malformed successful payloads with an actionable error', () => {
    expect(() => parseSystemInsights({})).toThrow('Format respons insight sistem tidak valid');
    expect(() => parseSystemInsights({ evaluated_at: 1, total_insights: 1, insights: [null] })).toThrow();
    expect(() => parseSystemInsights({ evaluated_at: 1, total_insights: 1, insights: [{ id: 'x', insight_type: 'x', severity: 'unknown', title: 'x', description: 'x', detected_at: 1 }] })).toThrow();
    expect(() => parseRetention({ retention: retention.retention, stats: {} })).toThrow('Format respons retensi analitik tidak valid');
  });
  it('sends retention update fields in the hours/days/MB units Rust expects', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify(retention)));
    vi.stubGlobal('fetch', fetchMock);
    setCsrfToken('fixture');
    await updateAnalyticsRetention(parseRetention(retention).config);
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ raw_retention_hours: 2, one_minute_retention_days: 14, one_hour_retention_days: 400, one_day_retention_days: null, security_events_retention_days: 7, max_db_size_mb: 100, auto_vacuum: true });
  });
  it('maps maintenance counts, file sizes, timings, and supported request options', async () => {
    const fetchMock = vi.fn().mockResolvedValue(new Response(JSON.stringify(maintenance)));
    vi.stubGlobal('fetch', fetchMock);
    setCsrfToken('fixture');
    const report = await triggerAnalyticsMaintenance({ force_vacuum: true });
    expect(JSON.parse(fetchMock.mock.calls[0][1].body)).toEqual({ force_vacuum: true });
    expect(report.pruned_raw_records).toBe(20);
    expect(report.bytes_reclaimed).toBe(600);
    expect(report.completed_at).toBe(1700000000.25);
    expect(parseMaintenance({ ...maintenance, db_size_bytes_after: 3000 }).bytes_reclaimed).toBe(0);
  });
  it('maps unhealthy integrity details without treating healthy details as errors', () => {
    expect(parseIntegrity({ is_healthy: false, checked_at: 1, status: 'corrupt', details: ['Page 42'] }).errors).toEqual(['Page 42']);
    expect(parseIntegrity({ is_healthy: true, checked_at: 1, status: 'ok', details: ['ok'] }).errors).toEqual([]);
  });
});
