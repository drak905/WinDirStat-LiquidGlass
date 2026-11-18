import React from 'react';
import {
  LayoutGrid,
  Disc,
  Flame,
  CopyCheck,
  Bot,
  ShieldCheck,
  HardDrive,
  RefreshCw,
  Zap,
} from 'lucide-react';
import type { DriveInfo, NavTabId } from '../types/storage';
import { formatBytes } from '../utils/formatters';

interface GlassDockProps {
  drives: DriveInfo[];
  selectedDrive: string;
  onSelectDrive: (driveId: string) => void;
  onTriggerScan: (driveId?: string) => void;
  isScanning: boolean;
  activeTab: NavTabId;
  onSelectTab: (tab: NavTabId) => void;
  duplicateCount: number;
  vaultCount: number;
  totalScannedBytes: number;
  totalNodeCount: number;
}

const NAV_ITEMS: {
  id: NavTabId;
  testId: string;
  label: string;
  subtitle: string;
  icon: React.ComponentType<{ className?: string }>;
}[] = [
  {
    id: 'treemap',
    testId: 'nav-treemap',
    label: 'Visual Treemap',
    subtitle: 'Squarified 3D Cushions',
    icon: LayoutGrid,
  },
  {
    id: 'sunburst',
    testId: 'nav-sunburst',
    label: 'Sunburst',
    subtitle: 'Radial Depth Rings',
    icon: Disc,
  },
  {
    id: 'flamegraph',
    testId: 'nav-flamegraph',
    label: 'Flamegraph',
    subtitle: 'Hotspot Call Stacks',
    icon: Flame,
  },
  {
    id: 'duplicates',
    testId: 'nav-duplicates',
    label: 'Duplicate Inspector',
    subtitle: 'xxHash64 + SHA-256',
    icon: CopyCheck,
  },
  {
    id: 'advisor',
    testId: 'nav-advisor',
    label: 'Agent CLI Advisor',
    subtitle: 'Cross-Checked AI Chat',
    icon: Bot,
  },
  {
    id: 'vault',
    testId: 'nav-vault',
    label: '7-Day Quarantine Vault',
    subtitle: '168h Safe Rollback',
    icon: ShieldCheck,
  },
];

export const GlassDock: React.FC<GlassDockProps> = ({
  drives,
  selectedDrive,
  onSelectDrive,
  onTriggerScan,
  isScanning,
  activeTab,
  onSelectTab,
  duplicateCount,
  vaultCount,
  totalScannedBytes,
  totalNodeCount,
}) => {
  return (
    <aside
      data-testid="glass-dock"
      className="liquid-glass flex w-72 shrink-0 flex-col justify-between gap-4 rounded-3xl p-4"
    >
      <div className="flex flex-col gap-4">
        {/* Drive / Target Selector */}
        <section className="rounded-2xl border border-white/20 bg-white/10 p-3 dark:bg-black/20">
          <div className="mb-2 flex items-center justify-between">
            <span className="flex items-center gap-1.5 text-[11px] font-semibold uppercase tracking-wider text-slate-700 dark:text-slate-300">
              <HardDrive className="h-3.5 w-3.5 text-sky-400" />
              Target Volume
            </span>
            <span className="text-[11px] font-medium text-sky-600 dark:text-sky-300">
              {formatBytes(totalScannedBytes)}
            </span>
          </div>

          <select
            data-testid="drive-selector"
            aria-label="Select drive or snapshot"
            value={selectedDrive}
            onChange={(e) => onSelectDrive(e.target.value)}
            className="mb-2.5 w-full rounded-xl border border-white/25 bg-white/25 px-2.5 py-2 text-xs font-medium text-slate-900 outline-none focus:ring-2 focus:ring-sky-400/60 dark:bg-slate-900/70 dark:text-slate-100"
          >
            {drives.map((d) => (
              <option key={d.id} value={d.id} className="bg-slate-900 text-slate-100">
                {d.label}
              </option>
            ))}
          </select>

          <div className="flex gap-2">
            <button
              type="button"
              data-testid="scan-btn"
              disabled={isScanning}
              onClick={() => onTriggerScan()}
              className="liquid-glass-interactive flex flex-1 items-center justify-center gap-1.5 rounded-xl bg-gradient-to-r from-sky-500/85 to-indigo-500/85 px-3 py-2 text-xs font-semibold text-white shadow-lg shadow-sky-500/20 disabled:opacity-60"
            >
              <RefreshCw className={`h-3.5 w-3.5 ${isScanning ? 'animate-spin' : ''}`} />
              {isScanning ? 'Scanning…' : 'Scan'}
            </button>
            <button
              type="button"
              data-testid="load-100k-btn"
              onClick={() => onTriggerScan('demo://100k-snapshot')}
              title="Load 100k Node Performance Benchmark Snapshot"
              className="liquid-glass-interactive flex items-center gap-1 rounded-xl border border-white/25 bg-white/15 px-2.5 py-2 text-xs font-medium text-slate-800 hover:bg-white/25 dark:text-slate-100"
            >
              <Zap className="h-3.5 w-3.5 text-amber-400" />
              100k
            </button>
          </div>

          <div className="mt-2.5 flex items-center justify-between text-[11px] text-slate-600 dark:text-slate-400">
            <span>Nodes indexed</span>
            <span className="font-mono font-semibold text-slate-800 dark:text-slate-200">
              {totalNodeCount.toLocaleString('en-US')}
            </span>
          </div>
        </section>

        {/* Navigation Tabs */}
        <nav aria-label="Storage analysis views" className="flex flex-col gap-1.5">
          {NAV_ITEMS.map((item) => {
            const Icon = item.icon;
            const active = activeTab === item.id;
            const badge =
              item.id === 'duplicates'
                ? duplicateCount
                : item.id === 'vault'
                  ? vaultCount
                  : null;

            return (
              <button
                key={item.id}
                type="button"
                data-testid={item.testId}
                onClick={() => onSelectTab(item.id)}
                className={`liquid-glass-interactive group flex w-full items-center justify-between rounded-2xl px-3 py-2.5 text-left ${
                  active
                    ? 'border border-white/40 bg-white/30 text-slate-950 shadow-lg dark:border-white/25 dark:bg-white/15 dark:text-white'
                    : 'border border-transparent text-slate-700 hover:border-white/20 hover:bg-white/15 dark:text-slate-300 dark:hover:bg-white/10'
                }`}
              >
                <div className="flex items-center gap-2.5">
                  <div
                    className={`flex h-8 w-8 items-center justify-center rounded-xl transition ${
                      active
                        ? 'bg-gradient-to-br from-sky-400 to-indigo-500 text-white shadow-md'
                        : 'bg-white/15 text-slate-700 group-hover:bg-white/25 dark:text-slate-300'
                    }`}
                  >
                    <Icon className="h-4 w-4" />
                  </div>
                  <div>
                    <div className="text-xs font-semibold leading-tight">{item.label}</div>
                    <div className="text-[10px] text-slate-600 dark:text-slate-400">
                      {item.subtitle}
                    </div>
                  </div>
                </div>

                {badge !== null && badge > 0 && (
                  <span
                    className={`rounded-full px-2 py-0.5 text-[10px] font-bold ${
                      item.id === 'vault'
                        ? 'bg-emerald-500/25 text-emerald-700 dark:text-emerald-300'
                        : 'bg-amber-500/25 text-amber-700 dark:text-amber-300'
                    }`}
                  >
                    {badge}
                  </span>
                )}
              </button>
            );
          })}
        </nav>
      </div>

      {/* Footer Tip */}
      <div className="rounded-2xl border border-white/15 bg-white/10 p-3 text-[11px] leading-relaxed text-slate-700 dark:bg-black/20 dark:text-slate-300">
        <div className="font-semibold text-slate-900 dark:text-white">Liquid Glass Controls</div>
        <p className="mt-0.5 opacity-85">
          Right-click any treemap cell, directory, or extension group to ask an Agent CLI or
          quarantine with 168h rollback protection.
        </p>
      </div>
    </aside>
  );
};
