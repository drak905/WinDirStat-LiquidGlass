import React, { useEffect, useMemo, useState } from 'react';
import { Flame, Search, ArrowUp } from 'lucide-react';
import type { StorageNode } from '../types/storage';
import { formatBytes, getExtensionColor } from '../utils/formatters';
import { nodeToContextTarget, type ContextMenuTarget } from './ContextMenu';

interface FlamegraphViewProps {
  root: StorageNode;
  onContextMenuNode: (x: number, y: number, target: ContextMenuTarget) => void;
}

interface FlameFrame {
  node: StorageNode;
  depth: number;
  leftPct: number;
  widthPct: number;
  color: string;
}

export const FlamegraphView: React.FC<FlamegraphViewProps> = ({
  root,
  onContextMenuNode,
}) => {
  const [zoomStack, setZoomStack] = useState<StorageNode[]>([root]);
  const [filterText, setFilterText] = useState('');
  const [hoveredFrame, setHoveredFrame] = useState<FlameFrame | null>(null);

  useEffect(() => {
    setZoomStack([root]);
    setHoveredFrame(null);
  }, [root]);

  const activeRoot = zoomStack[zoomStack.length - 1] ?? root;

  const framesByDepth = useMemo(() => {
    const levels: FlameFrame[][] = [];
    const maxDepth = 6;

    function visit(node: StorageNode, depth: number, leftPct: number, widthPct: number) {
      if (depth > maxDepth || widthPct < 0.35) return;
      if (!levels[depth]) levels[depth] = [];
      const leadExt = node.isDir ? node.children?.[0]?.extension : node.extension;
      levels[depth].push({
        node,
        depth,
        leftPct,
        widthPct,
        color: getExtensionColor(leadExt),
      });

      if (!node.isDir || !node.children || node.children.length === 0 || node.size <= 0) {
        return;
      }

      let cursor = leftPct;
      for (const child of node.children) {
        const childWidth = (child.size / node.size) * widthPct;
        if (childWidth >= 0.35) {
          visit(child, depth + 1, cursor, childWidth);
        }
        cursor += childWidth;
      }
    }

    visit(activeRoot, 0, 0, 100);
    return levels;
  }, [activeRoot]);

  const normalizedFilter = filterText.trim().toLowerCase();

  return (
    <div data-testid="flamegraph-view" className="flex h-full min-h-0 flex-col gap-3">
      <div className="liquid-glass-card flex flex-wrap items-center justify-between gap-2 rounded-2xl px-4 py-2.5 text-xs">
        <div className="flex items-center gap-2">
          <Flame className="h-4 w-4 text-orange-400" />
          <span className="font-semibold text-slate-900 dark:text-white">
            Storage Flamegraph — {activeRoot.name}
          </span>
          {zoomStack.length > 1 && (
            <button
              type="button"
              onClick={() => setZoomStack((prev) => prev.slice(0, -1))}
              className="flex items-center gap-1 rounded-lg border border-white/25 bg-white/15 px-2 py-0.5 text-[11px]"
            >
              <ArrowUp className="h-3 w-3" />
              Up
            </button>
          )}
        </div>

        <div className="relative">
          <Search className="pointer-events-none absolute left-2.5 top-2 h-3.5 w-3.5 text-slate-400" />
          <input
            type="text"
            data-testid="flamegraph-filter"
            value={filterText}
            onChange={(e) => setFilterText(e.target.value)}
            placeholder="Filter path or extension..."
            className="w-60 rounded-xl border border-white/25 bg-white/20 py-1.5 pl-8 pr-3 text-xs text-slate-900 placeholder-slate-500 outline-none focus:ring-2 focus:ring-sky-400/50 dark:bg-black/30 dark:text-slate-100"
          />
        </div>
      </div>

      <div className="liquid-glass-card flex min-h-0 flex-1 flex-col justify-between overflow-hidden rounded-3xl p-4">
        <div className="glass-scroll flex-1 space-y-1.5 overflow-y-auto">
          {framesByDepth.map((row, depth) => (
            <div key={depth} className="relative h-9 w-full">
              {row.map((frame) => {
                const matches =
                  !normalizedFilter ||
                  frame.node.name.toLowerCase().includes(normalizedFilter) ||
                  frame.node.path.toLowerCase().includes(normalizedFilter);
                return (
                  <button
                    key={frame.node.id}
                    type="button"
                    title={`${frame.node.path} (${formatBytes(frame.node.size)})`}
                    style={{
                      left: `${frame.leftPct}%`,
                      width: `${frame.widthPct}%`,
                      backgroundColor: frame.color,
                      opacity: matches ? 0.88 : 0.22,
                    }}
                    onMouseEnter={() => setHoveredFrame(frame)}
                    onMouseLeave={() => setHoveredFrame(null)}
                    onClick={() => {
                      if (frame.node.isDir && frame.node.children?.length) {
                        setZoomStack((prev) => [...prev, frame.node]);
                      }
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      onContextMenuNode(e.clientX, e.clientY, nodeToContextTarget(frame.node));
                    }}
                    className="absolute top-0 flex h-8 items-center overflow-hidden rounded-lg border border-white/30 px-2 text-left text-[11px] font-semibold text-white shadow-sm transition hover:brightness-110"
                  >
                    <span className="truncate">
                      {frame.node.name} ({formatBytes(frame.node.size)})
                    </span>
                  </button>
                );
              })}
            </div>
          ))}
        </div>

        <div className="mt-3 flex items-center justify-between rounded-2xl border border-white/15 bg-white/10 px-3.5 py-2 text-xs dark:bg-black/25">
          {hoveredFrame ? (
            <>
              <span className="truncate font-mono text-[11px] text-slate-800 dark:text-slate-200">
                {hoveredFrame.node.path}
              </span>
              <span className="shrink-0 font-mono font-bold text-sky-600 dark:text-sky-300">
                {formatBytes(hoveredFrame.node.size)}
              </span>
            </>
          ) : (
            <span className="text-slate-600 dark:text-slate-400">
              Click a directory bar to zoom stack • Right-click any bar for Agent CLI or 7-Day Vault
            </span>
          )}
        </div>
      </div>
    </div>
  );
};
