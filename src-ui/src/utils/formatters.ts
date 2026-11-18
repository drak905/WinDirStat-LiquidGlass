const EXTENSION_PALETTE: Record<string, string> = {
  '.iso': '#e11d48',
  '.img': '#be123c',
  '.vhdx': '#f43f5e',
  '.mp4': '#7c3aed',
  '.mkv': '#8b5cf6',
  '.mov': '#a855f7',
  '.zip': '#f59e0b',
  '.7z': '#d97706',
  '.tar': '#fbbf24',
  '.log': '#0ea5e9',
  '.tmp': '#38bdf8',
  '.cache': '#06b6d4',
  '.exe': '#2563eb',
  '.dll': '#3b82f6',
  '.pdb': '#6366f1',
  '.ts': '#10b981',
  '.tsx': '#14b8a6',
  '.rs': '#ea580c',
  '.cpp': '#f97316',
  '.json': '#84cc16',
  '.sqlite': '#ec4899',
  '.bin': '#64748b',
  dir: '#475569',
  other: '#60a5fa',
};

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 B';
  const units = ['B', 'KB', 'MB', 'GB', 'TB', 'PB'];
  const exp = Math.min(Math.floor(Math.log(bytes) / Math.log(1024)), units.length - 1);
  const val = bytes / 1024 ** exp;
  return `${val >= 100 || exp === 0 ? val.toFixed(0) : val.toFixed(2)} ${units[exp]}`;
}

export function formatCount(count: number): string {
  return new Intl.NumberFormat('en-US').format(count);
}

export function formatRemainingTtl(remainingHours: number): string {
  const clamped = Math.max(0, Math.min(168, remainingHours));
  return `${clamped.toFixed(1)}h / 168h`;
}

export function getExtensionColor(extension?: string): string {
  if (!extension) return EXTENSION_PALETTE.other;
  const normalized = extension.toLowerCase();
  if (EXTENSION_PALETTE[normalized]) {
    return EXTENSION_PALETTE[normalized];
  }
  // Deterministic vivid fallback hue per unknown extension
  let hash = 0;
  for (let i = 0; i < normalized.length; i += 1) {
    hash = (hash * 31 + normalized.charCodeAt(i)) >>> 0;
  }
  const hue = hash % 360;
  return hslToHex(hue, 72, 54);
}

export function hexToRgb(hex: string): [number, number, number] {
  const clean = hex.replace('#', '');
  if (clean.length !== 6) return [96, 165, 250];
  const r = Number.parseInt(clean.slice(0, 2), 16);
  const g = Number.parseInt(clean.slice(2, 4), 16);
  const b = Number.parseInt(clean.slice(4, 6), 16);
  return [r, g, b];
}

function hslToHex(h: number, s: number, l: number): string {
  const sNorm = s / 100;
  const lNorm = l / 100;
  const a = sNorm * Math.min(lNorm, 1 - lNorm);
  const f = (n: number) => {
    const k = (n + h / 30) % 12;
    const color = lNorm - a * Math.max(Math.min(k - 3, 9 - k, 1), -1);
    return Math.round(255 * color)
      .toString(16)
      .padStart(2, '0');
  };
  return `#${f(0)}${f(8)}${f(4)}`;
}
