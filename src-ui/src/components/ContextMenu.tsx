import React, { useEffect } from 'react';
import { Bot, ShieldAlert, ZoomIn, Copy } from 'lucide-react';
import type { StorageNode } from '../types/storage';
import { formatBytes } from '../utils/formatters';

export interface ContextMenuTarget {
  name: string;
  path: string;
  sizeBytes: number;
  isDir: boolean;
  kind: 'file' | 'folder' | 'extension';
}

export function nodeToContextTarget(node: StorageNode): ContextMenuTarget {
  return {
    name: node.name,
    path: node.path,
    sizeBytes: node.size,
    isDir: node.isDir,
    kind: node.isDir ? 'folder' : 'file',
  };
}

interface ContextMenuProps {
  x: number;
  y: number;
  target: ContextMenuTarget | null;
  onClose: () => void;
  onAskAgent: (target: ContextMenuTarget) => void;
  onQuarantine: (target: ContextMenuTarget) => void;
  onZoomIn?: (target: ContextMenuTarget) => void;
}

export const ContextMenu: React.FC<ContextMenuProps> = ({
  x,
  y,
  target,
  onClose,
  onAskAgent,
  onQuarantine,
  onZoomIn,
}) => {
  useEffect(() => {
    if (!target) return;
    const handleKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') onClose();
    };
    const handlePointerDown = (e: MouseEvent) => {
      const el = e.target as HTMLElement | null;
      if (!el?.closest('[data-testid="treemap-context-menu"]')) {
        onClose();
      }
    };
    window.addEventListener('keydown', handleKey);
    window.addEventListener('mousedown', handlePointerDown);
    return () => {
      window.removeEventListener('keydown', handleKey);
      window.removeEventListener('mousedown', handlePointerDown);
    };
  }, [target, onClose]);

  if (!target) return null;

  const clampedX = typeof window !== 'undefined' ? Math.min(x, Math.max(16, window.innerWidth - 310)) : x;
  const clampedY = typeof window !== 'undefined' ? Math.min(y, Math.max(16, window.innerHeight - 250)) : y;

  const handleCopyPath = async () => {
    try {
      await navigator.clipboard?.writeText(target.path);
    } catch {
      // Clipboard unavailable in restricted test context
    }
    onClose();
  };

  return (
    <div
      data-testid="treemap-context-menu"
      role="menu"
      style={{ left: clampedX, top: clampedY }}
      className="liquid-glass liquid-spring-enter fixed z-50 w-72 rounded-2xl p-2 text-slate-900 dark:text-slate-100"
    >
      <div className="mb-1.5 rounded-xl border border-white/15 bg-white/15 px-3 py-2 dark:bg-black/25">
        <div className="flex items-center justify-between gap-2">
          <span className="truncate text-xs font-semibold">{target.name}</span>
          <span className="shrink-0 rounded-full bg-sky-500/20 px-2 py-0.5 font-mono text-[10px] font-semibold text-sky-700 dark:text-sky-300">
            {formatBytes(target.sizeBytes)}
          </span>
        </div>
        <div className="mt-0.5 truncate font-mono text-[10px] text-slate-600 dark:text-slate-400">
          {target.path}
        </div>
      </div>

      <button
        type="button"
        role="menuitem"
        data-testid="ctx-ask-agent"
        onClick={() => {
          onAskAgent(target);
          onClose();
        }}
        className="liquid-glass-interactive flex w-full items-center gap-2.5 rounded-xl px-3 py-2 text-left text-xs font-medium hover:bg-sky-500/25"
      >
        <Bot className="h-4 w-4 shrink-0 text-sky-400" />
        <span>Ask Agent CLI: Should I delete or keep this?</span>
      </button>

      <button
        type="button"
        role="menuitem"
        data-testid="ctx-quarantine"
        onClick={() => {
          onQuarantine(target);
          onClose();
        }}
        className="liquid-glass-interactive mt-1 flex w-full items-center gap-2.5 rounded-xl px-3 py-2 text-left text-xs font-medium hover:bg-amber-500/25"
      >
        <ShieldAlert className="h-4 w-4 shrink-0 text-amber-400" />
        <span>Move to 7-Day Quarantine Vault</span>
      </button>

      <div className="my-1.5 h-px bg-white/15" />

      {target.isDir && onZoomIn && (
        <button
          type="button"
          role="menuitem"
          onClick={() => {
            onZoomIn(target);
            onClose();
          }}
          className="flex w-full items-center gap-2.5 rounded-xl px-3 py-1.5 text-left text-xs text-slate-700 hover:bg-white/15 dark:text-slate-300"
        >
          <ZoomIn className="h-3.5 w-3.5" />
          <span>Zoom Into Folder</span>
        </button>
      )}

      <button
        type="button"
        role="menuitem"
        onClick={() => void handleCopyPath()}
        className="flex w-full items-center gap-2.5 rounded-xl px-3 py-1.5 text-left text-xs text-slate-700 hover:bg-white/15 dark:text-slate-300"
      >
        <Copy className="h-3.5 w-3.5" />
        <span>Copy Full Path</span>
      </button>
    </div>
  );
};
