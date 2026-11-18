import fs from 'node:fs';
import path from 'node:path';
import { describe, expect, it } from 'vitest';

const repoRoot = path.resolve(process.cwd(), '..');

describe('Tauri v2 Liquid Glass Configuration', () => {
  it('configures transparent frameless acrylic/mica window and fluentOverlay scrollbar in tauri.conf.json', () => {
    const confPath = path.join(repoRoot, 'src-tauri', 'tauri.conf.json');
    expect(fs.existsSync(confPath)).toBe(true);

    const conf = JSON.parse(fs.readFileSync(confPath, 'utf-8'));
    expect(conf.build.frontendDist).toBe('../src-ui/dist');
    expect(conf.build.devUrl).toBe('http://localhost:1420');

    const win = conf.app.windows[0];
    expect(win.title).toBe('WinDirStat Liquid Glass');
    expect(win.width).toBe(1440);
    expect(win.height).toBe(920);
    expect(win.transparent).toBe(true);
    expect(win.decorations).toBe(false);
    expect(win.scrollBarStyle).toBe('fluentOverlay');
    expect(win.windowEffects).toBeDefined();
    expect(win.windowEffects.effects).toEqual(
      expect.arrayContaining(['acrylic', 'mica'])
    );
  });

  it('grants core and window permissions in capabilities/default.json alongside integrated Rust backend', () => {
    const capPath = path.join(repoRoot, 'src-tauri', 'capabilities', 'default.json');
    expect(fs.existsSync(capPath)).toBe(true);

    const cap = JSON.parse(fs.readFileSync(capPath, 'utf-8'));
    expect(cap.windows).toContain('main');
    expect(cap.permissions).toEqual(
      expect.arrayContaining([
        'core:default',
        'core:window:default',
        'core:window:allow-start-dragging',
        'core:window:allow-set-effects',
      ])
    );

    expect(fs.existsSync(path.join(repoRoot, 'src-tauri', 'Cargo.toml'))).toBe(true);
    expect(fs.existsSync(path.join(repoRoot, 'src-tauri', 'src'))).toBe(true);
  });
});
