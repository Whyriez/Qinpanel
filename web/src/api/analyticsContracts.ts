import type { AnalyticsRetentionResponse, IntegrityCheckReportDto, RetentionMaintenanceReportDto, SystemInsightsResponse, SystemTimeseriesResponse, UpdateRetentionRequest } from '../types/api';

function record(value: unknown, message: string): Record<string, unknown> {
  if (!value || typeof value !== 'object' || Array.isArray(value)) throw new Error(message);
  return value as Record<string, unknown>;
}
function numeric(value: unknown, message: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value)) throw new Error(message);
  return value;
}
function boolean(value: unknown, message: string): boolean {
  if (typeof value !== 'boolean') throw new Error(message);
  return value;
}
export function parseSystemInsights(value: unknown): SystemInsightsResponse {
  const message = 'Format respons insight sistem tidak valid';
  const data = record(value, message);
  numeric(data.evaluated_at, message);
  numeric(data.total_insights, message);
  if (!Array.isArray(data.insights)) throw new Error(message);
  for (const item of data.insights) {
    const entry = record(item, message);
    for (const key of ['id', 'insight_type', 'severity', 'title', 'description']) {
      if (typeof entry[key] !== 'string') throw new Error(message);
    }
    numeric(entry.detected_at, message);
    if (!['critical', 'warning', 'info'].includes(entry.severity as string)) throw new Error(message);
    if (entry.site_id != null) numeric(entry.site_id, message);
  }
  return data as unknown as SystemInsightsResponse;
}
export function parseSystemTimeseries(value: unknown): SystemTimeseriesResponse {
  const message = 'Format respons metrik server tidak valid';
  const data = record(value, message);
  if (!Array.isArray(data.points)) throw new Error(message);
  numeric(data.from_ts, message);
  numeric(data.to_ts, message);
  if (typeof data.interval !== 'string') throw new Error(message);
  for (const item of data.points) {
    const point = record(item, message);
    for (const key of ['timestamp', 'cpu_avg', 'cpu_max', 'load1_avg', 'load1_max', 'net_rx_avg_bytes_sec', 'net_tx_avg_bytes_sec']) numeric(point[key], message);
  }
  return data as unknown as SystemTimeseriesResponse;
}
export function parseMaintenance(value: unknown): RetentionMaintenanceReportDto {
  const message = 'Format laporan pemeliharaan analitik tidak valid';
  const data = record(value, message);
  const num = (key: string) => numeric(data[key], message);
  return {
    started_at: num('timestamp'),
    completed_at: num('timestamp') + num('duration_ms') / 1000,
    pruned_raw_records: num('raw_records_pruned'),
    pruned_rollup_1m_records: num('one_minute_records_pruned'),
    pruned_rollup_1h_records: num('one_hour_records_pruned'),
    pruned_rollup_1d_records: num('one_day_records_pruned'),
    pruned_security_events: num('security_events_pruned'),
    pruned_stale_tailer_records: num('checkpoints_pruned'),
    initial_db_size_bytes: num('db_size_bytes_before'),
    final_db_size_bytes: num('db_size_bytes_after'),
    bytes_reclaimed: Math.max(0, num('db_size_bytes_before') + num('wal_size_bytes_before') - num('db_size_bytes_after') - num('wal_size_bytes_after')),
    vacuum_performed: boolean(data.vacuum_executed, message),
    wal_checkpoint_performed: boolean(data.wal_checkpoint_executed, message),
    is_emergency_pruning: boolean(data.emergency_pruning_triggered, message),
  };
}
export function parseIntegrity(value: unknown): IntegrityCheckReportDto {
  const message = 'Format laporan integritas analitik tidak valid';
  const data = record(value, message);
  if (!Array.isArray(data.details) || !data.details.every(item => typeof item === 'string')) throw new Error(message);
  const healthy = boolean(data.is_healthy, message);
  return { is_healthy: healthy, checked_at: numeric(data.checked_at, message), errors: healthy ? [] : data.details };
}
export function parseRetention(value: unknown): AnalyticsRetentionResponse {
  const message = 'Format respons retensi analitik tidak valid';
  const data = record(value, message);
  const retention = record(data.retention, message), stats = record(data.stats, message);
  const num = (key: string) => numeric(stats[key], message);
  const duration = (key: string) => numeric(retention[key], message);
  if (!Array.isArray(stats.table_stats)) throw new Error(message);
  const tables = stats.table_stats.map(item => {
    const table = record(item, message);
    if (typeof table.table_name !== 'string') throw new Error(message);
    return { table_name: table.table_name, row_count: numeric(table.row_count, message) };
  });
  return {
    config: {
      raw_retention_secs: duration('raw_retention_secs'),
      rollup_1m_retention_secs: duration('one_minute_retention_secs'),
      rollup_1h_retention_secs: duration('one_hour_retention_secs'),
      rollup_1d_retention_secs: retention.one_day_retention_secs == null ? null : duration('one_day_retention_secs'),
      security_events_retention_secs: duration('security_events_retention_secs'),
      max_db_size_bytes: retention.max_db_size_bytes == null ? null : duration('max_db_size_bytes'),
      auto_vacuum: boolean(retention.auto_vacuum, message),
    },
    storage: {
      main_db_size_bytes: num('db_size_bytes'), wal_size_bytes: num('wal_size_bytes'),
      total_size_bytes: num('total_file_size_bytes'), page_size: num('page_size'), page_count: num('page_count'),
      freelist_count: num('freelist_count'), free_bytes: num('free_bytes'), fragmentation_ratio: num('fragmentation_ratio'), tables,
    },
    last_report: data.last_maintenance == null ? null : parseMaintenance(data.last_maintenance),
  };
}
export function retentionUpdatePayload(data: UpdateRetentionRequest) {
  return {
    raw_retention_hours: data.raw_retention_secs == null ? data.raw_retention_secs : data.raw_retention_secs / 3600,
    one_minute_retention_days: data.rollup_1m_retention_secs == null ? data.rollup_1m_retention_secs : data.rollup_1m_retention_secs / 86400,
    one_hour_retention_days: data.rollup_1h_retention_secs == null ? data.rollup_1h_retention_secs : data.rollup_1h_retention_secs / 86400,
    one_day_retention_days: data.rollup_1d_retention_secs == null ? data.rollup_1d_retention_secs : data.rollup_1d_retention_secs / 86400,
    security_events_retention_days: data.security_events_retention_secs == null ? data.security_events_retention_secs : data.security_events_retention_secs / 86400,
    max_db_size_mb: data.max_db_size_bytes == null ? data.max_db_size_bytes : data.max_db_size_bytes / 1048576,
    auto_vacuum: data.auto_vacuum,
  };
}
