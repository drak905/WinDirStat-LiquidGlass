import React from 'react';
import { Moon, Sun, Minus, Square, X, Sparkles } from 'lucide-react';
import { isTauriRuntime } from '../services/tauriBridge';

interface TitleBarProps {
  darkMode: boolean;
  onToggleTheme: () => void;
  activeDrive: string;
  scanStatusText: string;
}

export const TitleBar: React.FC<TitleBarProps> = ({
  darkMode,
  onToggleTheme,
  activeDrive,
  scanStatusText,
}) => {
  const handleWindowAction = async (action: 'minimize' | 'toggleMaximize' | 'close') => {
    if (!isTauriRuntime()) return;
    try {
      const { getCurrentWindow } = await import('@tauri-apps/api/window');
      const win = getCurrentWindow();
      if (action === 'minimize') await win.minimize();
      else if (action === 'toggleMaximize') await win.toggleMaximize();
      else await win.close();
    } catch {
      // Ignore outside Tauri desktop shell
    }
  };

  return (
    <header
      data-tauri-drag-region
      className="liquid-glass flex h-11 shrink-0 items-center justify-between rounded-2xl px-4 select-none"
    >
      <div data-tauri-drag-region className="flex items-center gap-3">
        <div className="flex h-7 w-7 items-center justify-center rounded-xl bg-gradient-to-br from-sky-400/80 via-indigo-500/80 to-fuchsia-500/80 text-white shadow-md">
          <Sparkles className="h-4 w-4" />
        </div>
        <div className="flex items-baseline gap-2">
          <span className="text-sm font-semibold tracking-tight text-slate-900 dark:text-white">
            WinDirStat Liquid Glass
          </span>
          <span className="rounded-full border border-white/25 bg-white/15 px-2 py-0.5 text-[11px] font-medium text-slate-700 dark:text-slate-200">
            {activeDrive}
          </span>
        </div>
        <span className="hidden text-xs text-slate-600 dark:text-slate-400 md:inline">
          {scanStatusText}
        </span>
      </div>

      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onToggleTheme}
          aria-label={darkMode ? 'Switch to light glass mode' : 'Switch to dark glass mode'}
          className="liquid-glass-interactive flex h-7 items-center gap-1.5 rounded-xl border border-white/20 bg-white/15 px-2.5 text-xs font-medium text-slate-800 hover:bg-white/25 dark:text-slate-100"
        >
          {darkMode ? <Sun className="h-3.5 w-3.5 text-amber-300" /> : <Moon className="h-3.5 w-3.5 text-indigo-600" />}
          <span className="hidden sm:inline">{darkMode ? 'Light' : 'Dark'}</span>
        </button>

        <div className="ml-1 flex items-center gap-1">
          <button
            type="button"
            aria-label="Minimize window"
            onClick={() => void handleWindowAction('minimize')}
            className="flex h-7 w-7 items-center justify-center rounded-lg text-slate-700 transition hover:bg-white/20 dark:text-slate-300"
          >
            <Minus className="h-3.5 w-3.5" />
          </button>
          <button
            type="button"
            aria-label="Maximize window"
            onClick={() => void handleWindowAction('toggleMaximize')}
            className="flex h-7 w-7 items-center justify-center rounded-lg text-slate-700 transition hover:bg-white/20 dark:text-slate-300"
          >
            <Square className="h-3 w-3" />
          </button>
          <button
            type="button"
            aria-label="Close window"
            onClick={() => void handleWindowAction('close')}
            className="flex h-7 w-7 items-center justify-center rounded-lg text-slate-700 transition hover:bg-rose-500/80 hover:text-white dark:text-slate-300"
          >
            <X className="h-3.5 w-3.5" />
          </button>
        </div>
      </div>
    </header>
  );
};
