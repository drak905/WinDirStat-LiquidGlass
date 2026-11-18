import { beforeEach, describe, expect, it, vi } from 'vitest';
import { getBridge, isTauriRuntime, resetSimulationStoreForTests } from '../services/tauriBridge';

describe('Tauri IPC Bridge Adapter & Browser Simulation Fallback', () => {
  beforeEach(() => {
    resetSimulationStoreForTests();
    delete window.__WDS_TEST_BRIDGE__;
  });

  it('detects non-Tauri browser runtime and simulates scan, advisor >=3 sources, quarantine, and rollback', async () => {
    expect(isTauriRuntime()).toBe(false);
    const bridge = getBridge();

    const drives = await bridge.listDrives();
    expect(drives.length).toBeGreaterThanOrEqual(3);

    const snap100k = await bridge.scanDirectory('demo://100k-snapshot');
    expect(snap100k.totalNodes).toBeGreaterThanOrEqual(100_000);

    const advice = await bridge.askAgentAdvisor({
      provider: 'codex',
      prompt: 'Should I delete C:\\Windows\\Temp\\installer-cache-bundle.tmp?',
      targetPath: 'C:\\Windows\\Temp\\installer-cache-bundle.tmp',
      targetName: 'installer-cache-bundle.tmp',
      targetSizeBytes: 9_400_000_000,
    });
    expect(advice.verdict).toBe('SAFE_TO_DELETE');
    expect(advice.sources.length).toBeGreaterThanOrEqual(3);
    expect(advice.risk_score).toBeLessThan(30);

    const quarantined = await bridge.quarantineItem({
      path: 'C:\\Users\\thinh\\Downloads\\ubuntu-24.04.1-desktop-amd64.iso',
      name: 'ubuntu-24.04.1-desktop-amd64.iso',
      sizeBytes: 6_120_000_000,
    });
    expect(quarantined.remainingHours).toBe(168);
    expect(quarantined.status).toBe('quarantined');

    const restored = await bridge.rollbackVaultItem(quarantined.id);
    expect(restored.status).toBe('restored');
  });

  it('honors window.__WDS_TEST_BRIDGE__ partial overrides', async () => {
    const customListDrives = vi.fn(async () => [
      {
        id: 'Z:\\',
        label: 'Z:\\ Test Override',
        mountPoint: 'Z:\\',
        totalBytes: 500_000_000_000,
        usedBytes: 100_000_000_000,
        freeBytes: 400_000_000_000,
        kind: 'nvme' as const,
      },
    ]);

    window.__WDS_TEST_BRIDGE__ = {
      listDrives: customListDrives,
    };

    const bridge = getBridge();
    const drives = await bridge.listDrives();
    expect(customListDrives).toHaveBeenCalledTimes(1);
    expect(drives[0].id).toBe('Z:\\');
  });
});
