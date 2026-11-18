import React, { useMemo, useState } from 'react';
import { ShieldCheck, RotateCcw, Trash2, Clock, CheckCircle } from 'lucide-react';
import type { VaultItem } from '../types/storage';
import { formatBytes, formatRemainingTtl } from '../utils/formatters';

interface VaultManagerPanelProps {
  items: VaultItem[];
  onRollback: (id: string) => Promise<void>;
  onPurge: (id: string) => Promise<void>;
  onRefresh: () => void;
}

export const VaultManagerPanel: React.FC<VaultManagerPanelProps> = ({
  items,
  onRollback,
  onPurge,
  onRefresh,
}) => {
  const [statusFilter, setStatusFilter] = useState<'all' | 'quarantined' | 'restored'>('all');
  const [busyId, setBusyId] = useState<string | null>(null);

  const filtered = useMemo(() => {
    if (statusFilter === 'all') return items;
    return items.filter((item) => item.status === statusFilter);
  }, [items, statusFilter]);

  const activeQuarantined = useMemo(
    () => items.filter((i) => i.status === 'quarantined'),
    [items]
  );

  const protectedBytes = useMemo(
    () => activeQuarantined.reduce((acc, i) => acc + i.sizeBytes, 0),
    [activeQuarantined]
  );

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      {/* Header Summary */}
      <div className="liquid-glass-card flex flex-wrap items-center justify-between gap-3 rounded-2xl px-4 py-3">
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-emerald-500/20 text-emerald-400">
            <ShieldCheck className="h-5 w-5" />
          </div>
          <div>
            <div className="text-xs font-semibold text-slate-900 dark:text-white">
              7-Day Quarantine Vault Manager
            </div>
            <div className="text-[11px] text-slate-600 dark:text-slate-400">
              {activeQuarantined.length} active items • {formatBytes(protectedBytes)} shielded for 168h rollback
            </div>
          </div>
        </div>

        <div className="flex items-center gap-2 text-xs">
          {(['all', 'quarantined', 'restored'] as const).map((tab) => (
            <button
              key={tab}
              type="button"
              onClick={() => setStatusFilter(tab)}
              className={`rounded-xl px-2.5 py-1 capitalize transition ${
                statusFilter === tab
                  ? 'bg-sky-500/25 font-semibold text-sky-800 dark:text-sky-200'
                  : 'text-slate-600 hover:bg-white/15 dark:text-slate-300'
              }`}
            >
              {tab}
            </button>
          ))}
          <button
            type="button"
            onClick={onRefresh}
            className="rounded-xl border border-white/25 bg-white/15 px-2.5 py-1 text-[11px] font-medium hover:bg-white/25"
          >
            Refresh
          </button>
        </div>
      </div>

      {/* Vault Items Table */}
      <div className="liquid-glass-card glass-scroll min-h-0 flex-1 overflow-auto rounded-3xl p-3">
        <table
          data-testid="vault-table"
          className="w-full border-collapse text-left text-xs"
        >
          <thead>
            <tr className="border-b border-white/15 text-[11px] uppercase tracking-wider text-slate-600 dark:text-slate-400">
              <th className="px-3 py-2.5">Item &amp; Original Path</th>
              <th className="px-3 py-2.5">Size</th>
              <th className="px-3 py-2.5">Remaining TTL (168h)</th>
              <th className="px-3 py-2.5">Integrity (SHA-256 / xxHash64)</th>
              <th className="px-3 py-2.5 text-right">Actions</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-white/10">
            {filtered.length === 0 ? (
              <tr>
                <td
                  colSpan={5}
                  className="px-3 py-10 text-center text-slate-600 dark:text-slate-400"
                >
                  No vault entries match the selected filter.
                </td>
              </tr>
            ) : (
              filtered.map((item) => {
                const ttlPct = Math.max(0, Math.min(100, (item.remainingHours / 168) * 100));
                return (
                  <tr key={item.id} className="hover:bg-white/10">
                    <td className="px-3 py-3">
                      <div className="flex items-center gap-2">
                        <span className="font-semibold text-slate-900 dark:text-white">
                          {item.fileName}
                        </span>
                        <span className="rounded bg-white/15 px-1.5 py-0.5 font-mono text-[10px] text-slate-600 dark:text-slate-300">
                          {item.batchId}
                        </span>
                        {item.status === 'restored' && (
                          <span className="rounded-full bg-emerald-500/20 px-2 py-0.5 text-[10px] font-semibold text-emerald-700 dark:text-emerald-300">
                            Restored
                          </span>
                        )}
                      </div>
                      <div className="mt-0.5 font-mono text-[10px] text-slate-600 dark:text-slate-400">
                        {item.originalPath}
                      </div>
                    </td>

                    <td className="px-3 py-3 font-mono font-semibold text-slate-800 dark:text-slate-200">
                      {formatBytes(item.sizeBytes)}
                    </td>

                    <td className="px-3 py-3">
                      <div className="flex items-center gap-1.5 font-mono text-[11px] font-semibold text-amber-700 dark:text-amber-300">
                        <Clock className="h-3.5 w-3.5" />
                        {formatRemainingTtl(item.remainingHours)}
                      </div>
                      <div className="mt-1 h-1.5 w-28 overflow-hidden rounded-full bg-slate-900/30 dark:bg-white/10">
                        <div
                          className="h-full rounded-full bg-gradient-to-r from-emerald-400 to-sky-400"
                          style={{ width: `${ttlPct}%` }}
                        />
                      </div>
                    </td>

                    <td className="px-3 py-3">
                      <div className="flex items-center gap-1.5 text-[11px] font-medium text-emerald-700 dark:text-emerald-300">
                        <CheckCircle className="h-3.5 w-3.5" />
                        <span>SHA-256 &amp; xxHash64 {item.hashVerified ? 'Verified' : 'Pending'}</span>
                      </div>
                      <div className="mt-0.5 font-mono text-[10px] text-slate-600 dark:text-slate-400">
                        sha256: {item.sha256.slice(0, 12)}… | xx64: {item.xxhash64}
                      </div>
                    </td>

                    <td className="px-3 py-3 text-right">
                      <div className="inline-flex items-center gap-1.5">
                        <button
                          type="button"
                          data-testid="vault-rollback-btn"
                          disabled={busyId === item.id || item.status !== 'quarantined'}
                          onClick={async () => {
                            setBusyId(item.id);
                            try {
                              await onRollback(item.id);
                            } finally {
                              setBusyId(null);
                            }
                          }}
                          className="liquid-glass-interactive flex items-center gap-1 rounded-xl bg-emerald-500/85 px-2.5 py-1.5 text-[11px] font-semibold text-slate-950 hover:bg-emerald-400 disabled:opacity-45"
                        >
                          <RotateCcw className="h-3.5 w-3.5" />
                          Rollback
                        </button>
                        <button
                          type="button"
                          aria-label={`Purge ${item.fileName}`}
                          disabled={busyId === item.id}
                          onClick={async () => {
                            setBusyId(item.id);
                            try {
                              await onPurge(item.id);
                            } finally {
                              setBusyId(null);
                            }
                          }}
                          className="flex h-7 w-7 items-center justify-center rounded-xl border border-rose-400/30 bg-rose-500/15 text-rose-500 hover:bg-rose-500/30 disabled:opacity-50"
                        >
                          <Trash2 className="h-3.5 w-3.5" />
                        </button>
                      </div>
                    </td>
                  </tr>
                );
              })
            )}
          </tbody>
        </table>
      </div>
    </div>
  );
};
