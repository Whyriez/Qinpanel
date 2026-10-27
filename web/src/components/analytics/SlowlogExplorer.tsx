import React, { useState } from 'react';
import { PhpSlowlogEntryDto } from '../../types/api';

export interface SlowlogExplorerProps {
  events: PhpSlowlogEntryDto[];
  className?: string;
}

function formatDateTime(ts: number): string {
  const d = new Date(ts * 1000);
  return `${d.toLocaleDateString([], { month: 'short', day: 'numeric', year: 'numeric' })} ${d.toLocaleTimeString([], { hour: '2-digit', minute: '2-digit', second: '2-digit' })}`;
}

export const SlowlogExplorer: React.FC<SlowlogExplorerProps> = ({
  events,
  className = '',
}) => {
  const [expandedId, setExpandedId] = useState<number | null>(null);
  const [searchTerm, setSearchTerm] = useState('');

  const filteredEvents = events.filter((ev) => {
    if (!searchTerm.trim()) return true;
    const q = searchTerm.toLowerCase();
    return (
      ev.script_filename.toLowerCase().includes(q) ||
      (ev.top_function && ev.top_function.toLowerCase().includes(q)) ||
      ev.pool.toLowerCase().includes(q)
    );
  });

  return (
    <div className={`bg-[var(--surface)] border border-[var(--line)] rounded-xl p-5 shadow-sm ${className}`}>
      <div className="flex flex-wrap items-center justify-between gap-3 mb-4">
        <div>
          <h3 className="text-base font-semibold text-[var(--ink)]">
            PHP-FPM Slowlog Explorer
          </h3>
          <p className="text-xs text-[var(--ink-muted)]">
            Daftar eksekusi skrip PHP yang melebihi batas waktu <code>request_slowlog_timeout</code>
          </p>
        </div>
        <div className="flex items-center gap-2">
          <input
            type="text"
            placeholder="Cari skrip atau fungsi..."
            value={searchTerm}
            onChange={(e) => setSearchTerm(e.target.value)}
            className="px-3 py-1.5 text-xs bg-[var(--bg)] border border-[var(--line)] rounded-lg text-[var(--ink)] placeholder-[var(--ink-muted)] focus:outline-none focus:ring-1 focus:ring-[var(--primary)]"
          />
        </div>
      </div>

      {filteredEvents.length === 0 ? (
        <div className="flex flex-col items-center justify-center p-8 text-center text-sm text-[var(--ink-muted)]">
          <div className="text-2xl mb-1">🎉</div>
          <p className="font-medium text-[var(--ink)]">Tidak ada log eksekusi lambat tercatat</p>
          <p className="text-xs mt-1">
            Semua permintaan PHP berjalan cepat dalam toleransi waktu yang ditentukan.
          </p>
        </div>
      ) : (
        <div className="space-y-3">
          {filteredEvents.map((ev, idx) => {
            const isExpanded = expandedId === idx;
            return (
              <div
                key={idx}
                className="border border-[var(--line)] rounded-lg overflow-hidden bg-[var(--bg)] transition-colors"
              >
                {/* Header row */}
                <div
                  onClick={() => setExpandedId(isExpanded ? null : idx)}
                  className="p-3.5 flex flex-wrap items-center justify-between gap-3 cursor-pointer hover:bg-[var(--surface-hover)] select-none"
                >
                  <div className="flex items-center gap-3 min-w-0">
                    <span className="text-amber-500 font-bold text-xs bg-amber-500/10 px-2 py-0.5 rounded border border-amber-500/20">
                      {ev.duration_secs ? `${ev.duration_secs.toFixed(1)}s` : 'SLOW'}
                    </span>
                    <div className="min-w-0">
                      <div className="font-mono text-xs font-semibold text-[var(--ink)] truncate max-w-md">
                        {ev.script_filename}
                      </div>
                      <div className="text-[11px] text-[var(--ink-muted)] flex items-center gap-2 mt-0.5">
                        <span>Pool: <strong className="text-[var(--ink)]">{ev.pool}</strong></span>
                        <span>•</span>
                        <span>PID: <strong className="text-[var(--ink)]">{ev.pid}</strong></span>
                        {ev.top_function && (
                          <>
                            <span>•</span>
                            <span className="text-amber-400 font-mono">fn: {ev.top_function}</span>
                          </>
                        )}
                      </div>
                    </div>
                  </div>

                  <div className="flex items-center gap-3">
                    <span className="text-xs text-[var(--ink-muted)]">
                      {formatDateTime(ev.timestamp)}
                    </span>
                    <button
                      type="button"
                      className="text-xs px-2.5 py-1 rounded border border-[var(--line)] bg-[var(--surface)] text-[var(--ink)] font-medium hover:bg-[var(--surface-hover)]"
                    >
                      {isExpanded ? 'Tutup Stack' : `Stack Trace (${ev.frames.length})`}
                    </button>
                  </div>
                </div>

                {/* Collapsible Stack Trace */}
                {isExpanded && (
                  <div className="border-t border-[var(--line)] bg-[var(--surface)] p-3 text-xs">
                    <div className="text-[11px] font-semibold text-[var(--ink-muted)] uppercase tracking-wider mb-2">
                      Riwayat Pemanggilan Fungsi (Call Stack)
                    </div>
                    {ev.frames.length === 0 ? (
                      <div className="text-[var(--ink-muted)] italic text-xs py-2">
                        Tidak ada detail frame stack trace yang tersedia.
                      </div>
                    ) : (
                      <div className="overflow-x-auto">
                        <table className="w-full text-left border-collapse">
                          <thead>
                            <tr className="border-b border-[var(--line)] text-[var(--ink-muted)] text-[10px]">
                              <th className="py-1.5 px-2 w-12 text-center">Frame</th>
                              <th className="py-1.5 px-2">Fungsi</th>
                              <th className="py-1.5 px-2">File & Baris</th>
                              {ev.frames.some((f) => f.address) && (
                                <th className="py-1.5 px-2 text-right w-28">Alamat</th>
                              )}
                            </tr>
                          </thead>
                          <tbody className="divide-y divide-[var(--line)] font-mono text-[11px]">
                            {ev.frames.map((frame, fIdx) => (
                              <tr
                                key={fIdx}
                                className={fIdx === 0 ? 'bg-amber-500/5 text-amber-200' : 'text-[var(--ink)]'}
                              >
                                <td className="py-1.5 px-2 text-center text-[var(--ink-muted)]">
                                  #{frame.frame_index}
                                </td>
                                <td className="py-1.5 px-2 font-semibold">
                                  {frame.function}()
                                </td>
                                <td className="py-1.5 px-2 text-[var(--ink-muted)] break-all">
                                  <span>{frame.file}</span>
                                  <span className="text-[var(--primary)] font-bold">:{frame.line}</span>
                                </td>
                                {ev.frames.some((f) => f.address) && (
                                  <td className="py-1.5 px-2 text-right text-[var(--ink-muted)] text-[10px]">
                                    {frame.address || '-'}
                                  </td>
                                )}
                              </tr>
                            ))}
                          </tbody>
                        </table>
                      </div>
                    )}
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
};
