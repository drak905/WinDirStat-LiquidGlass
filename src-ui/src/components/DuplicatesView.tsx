import React, { useMemo, useState } from 'react';
import { CopyCheck, ShieldAlert, Bot, Search } from 'lucide-react';
import type { DuplicateGroup } from '../types/storage';
import { formatBytes } from '../utils/formatters';

interface DuplicatesViewProps {
  groups: DuplicateGroup[];
  onQuarantineFile: (params: { path: string; name: string; sizeBytes: number }) => Promise<void>;
  onAskAgent: (target: { path: string; name: string; sizeBytes: number }) => void;
}

export const DuplicatesView: React.FC<DuplicatesViewProps> = ({
  groups,
  onQuarantineFile,
  onAskAgent,
}) => {
  const [search, setSearch] = useState('');
  const [busyPath, setBusyPath] = useState<string | null>(null);

  const filtered = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return groups;
    return groups.filter(
      (g) =>
        g.extension.toLowerCase().includes(q) ||
        g.hash.toLowerCase().includes(q) ||
        g.files.some((f) => f.path.toLowerCase().includes(q))
    );
  }, [groups, search]);

  const totalWasted = useMemo(
    () => filtered.reduce((acc, g) => acc + g.wastedBytes, 0),
    [filtered]
  );

  return (
    <div data-testid="duplicates-view" className="flex h-full min-h-0 flex-col gap-3">
      <div className="liquid-glass-card flex flex-wrap items-center justify-between gap-3 rounded-2xl px-4 py-3">
        <div className="flex items-center gap-3">
          <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-amber-500/20 text-amber-400">
            <CopyCheck className="h-5 w-5" />
          </div>
          <div>
            <div className="text-xs font-semibold text-slate-900 dark:text-white">
              Duplicate File Inspector (xxHash64 + SHA-256 Verified)
            </div>
            <div className="text-[11px] text-slate-600 dark:text-slate-400">
              {filtered.length} duplicate clusters •{' '}
              <span className="font-semibold text-amber-600 dark:text-amber-300">
                {formatBytes(totalWasted)} reclaimable
              </span>
            </div>
          </div>
        </div>

        <div className="relative">
          <Search className="pointer-events-none absolute left-2.5 top-2 h-3.5 w-3.5 text-slate-400" />
          <input
            type="text"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            placeholder="Filter hash, path, extension..."
            className="w-60 rounded-xl border border-white/25 bg-white/20 py-1.5 pl-8 pr-3 text-xs text-slate-900 placeholder-slate-500 outline-none focus:ring-2 focus:ring-sky-400/50 dark:bg-black/30 dark:text-slate-100"
          />
        </div>
      </div>

      <div className="glass-scroll min-h-0 flex-1 space-y-3 overflow-y-auto pr-1">
        {filtered.length === 0 ? (
          <div className="liquid-glass-card flex h-48 items-center justify-center rounded-3xl text-xs text-slate-600 dark:text-slate-400">
            No duplicate clusters remaining for the active scan.
          </div>
        ) : (
          filtered.map((group) => (
            <section key={group.id} className="liquid-glass-card rounded-3xl p-4">
              <div className="mb-3 flex flex-wrap items-center justify-between gap-2">
                <div className="flex items-center gap-2">
                  <span className="rounded-lg bg-indigo-500/20 px-2 py-0.5 font-mono text-[11px] font-bold text-indigo-700 dark:text-indigo-300">
                    {group.extension}
                  </span>
                  <span className="font-mono text-[11px] text-slate-600 dark:text-slate-400">
                    {group.hashAlgorithm}: {group.hash.slice(0, 16)}…
                  </span>
                </div>
                <div className="flex items-center gap-2 text-xs">
                  <span className="text-slate-600 dark:text-slate-400">
                    {group.files.length} copies × {formatBytes(group.fileSize)}
                  </span>
                  <span className="rounded-full bg-amber-500/20 px-2.5 py-0.5 font-mono text-[11px] font-bold text-amber-700 dark:text-amber-300">
                    -{formatBytes(group.wastedBytes)} wasted
                  </span>
                </div>
              </div>

              <div className="space-y-2">
                {group.files.map((file, idx) => {
                  const isPrimary = idx === 0;
                  return (
                    <div
                      key={file.id}
                      className="flex flex-wrap items-center justify-between gap-2 rounded-2xl border border-white/15 bg-white/10 px-3 py-2 text-xs dark:bg-black/20"
                    >
                      <div className="min-w-0 flex-1">
                        <div className="flex items-center gap-2">
                          <span className="truncate font-semibold text-slate-900 dark:text-white">
                            {file.name}
                          </span>
                          {isPrimary && (
                            <span className="rounded-full bg-emerald-500/20 px-2 py-0.5 text-[10px] font-semibold text-emerald-700 dark:text-emerald-300">
                              Primary Keep
                            </span>
                          )}
                        </div>
                        <div className="truncate font-mono text-[10px] text-slate-600 dark:text-slate-400">
                          {file.path}
                        </div>
                      </div>

                      <div className="flex items-center gap-1.5">
                        <button
                          type="button"
                          onClick={() =>
                            onAskAgent({
                              path: file.path,
                              name: file.name,
                              sizeBytes: group.fileSize,
                            })
                          }
                          className="flex items-center gap-1 rounded-xl border border-white/20 bg-white/15 px-2.5 py-1.5 text-[11px] font-medium hover:bg-sky-500/25"
                        >
                          <Bot className="h-3.5 w-3.5 text-sky-400" />
                          Ask CLI
                        </button>
                        <button
                          type="button"
                          data-testid="dup-quarantine-btn"
                          disabled={busyPath === file.path}
                          onClick={async () => {
                            setBusyPath(file.path);
                            try {
                              await onQuarantineFile({
                                path: file.path,
                                name: file.name,
                                sizeBytes: group.fileSize,
                              });
                            } finally {
                              setBusyPath(null);
                            }
                          }}
                          className="flex items-center gap-1 rounded-xl bg-amber-500/80 px-2.5 py-1.5 text-[11px] font-semibold text-slate-950 transition hover:bg-amber-400 disabled:opacity-60"
                        >
                          <ShieldAlert className="h-3.5 w-3.5" />
                          {isPrimary ? 'Quarantine Copy' : 'Quarantine Redundant'}
                        </button>
                      </div>
                    </div>
                  );
                })}
              </div>
            </section>
          ))
        )}
      </div>
    </div>
  );
};
