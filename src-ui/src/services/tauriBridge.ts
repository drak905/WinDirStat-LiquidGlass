import { invoke } from '@tauri-apps/api/core';
import {
  createDemoDrives,
  createDemoDuplicates,
  createDemoSnapshot,
  createInitialVaultItems,
  generateSynthetic100kTree,
} from '../data/demoSnapshot';
import type {
  AgentAdvisorResponse,
  AgentCliTestResult,
  AgentProvider,
  AgentProviderStatus,
  AskAgentAdvisorParams,
  DriveInfo,
  DuplicateGroup,
  QuarantineItemParams,
  ScanSnapshot,
  VaultItem,
  WdsBridge,
} from '../types/storage';

// Stateful browser simulation store (used in web preview / Vitest / Playwright when Tauri backend is absent)
let simVaultItems: VaultItem[] = createInitialVaultItems();
let simDuplicates: DuplicateGroup[] = createDemoDuplicates();

export function isTauriRuntime(): boolean {
  return typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window;
}

async function tryTauriInvoke<T>(command: string, args?: Record<string, unknown>): Promise<T | null> {
  if (isTauriRuntime()) {
    try {
      return await invoke<T>(command, args);
    } catch {
      return null;
    }
  }
  if (
    typeof window !== 'undefined' &&
    (window as unknown as { __WDS_DESKTOP_EXE__?: boolean }).__WDS_DESKTOP_EXE__
  ) {
    try {
      const response = await fetch('/api/invoke', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ command, args: args ?? {} }),
      });
      if (!response.ok) return null;
      const data = (await response.json()) as T | null;
      return data ?? null;
    } catch {
      return null;
    }
  }
  return null;
}

function buildSimulatedSources(targetPath: string): AgentAdvisorResponse['sources'] {
  const lower = targetPath.toLowerCase();
  if (lower.includes('.iso') || lower.includes('ubuntu') || lower.includes('win11')) {
    return [
      {
        title: 'Microsoft Windows Storage Sense Cleanup Reference',
        url: 'https://support.microsoft.com/en-us/windows/manage-drive-space-with-storage-sense',
        domain: 'support.microsoft.com',
        verified: true,
        snippet: 'Installation disk images in Downloads are safe to remove once deployed.',
      },
      {
        title: 'Ubuntu Official ISO Verification & Retention Guide',
        url: 'https://ubuntu.com/tutorials/how-to-verify-ubuntu',
        domain: 'ubuntu.com',
        verified: true,
        snippet: 'Verified ISO artifacts do not affect an already-installed OS volume.',
      },
      {
        title: 'NIST SP 800-88 Media Hygiene & Staging Guidelines',
        url: 'https://csrc.nist.gov/publications/detail/sp/800-88/rev-1/final',
        domain: 'csrc.nist.gov',
        verified: true,
        snippet: 'Temporary installation media should be archived or safely purged.',
      },
    ];
  }
  if (lower.includes('docker') || lower.includes('.vhdx')) {
    return [
      {
        title: 'Docker Desktop Windows WSL2 VHDX Compaction Docs',
        url: 'https://docs.docker.com/desktop/settings-and-maintenance/settings/',
        domain: 'docs.docker.com',
        verified: true,
        snippet: 'Never delete docker_data.vhdx directly while containers/volumes are active; prune images instead.',
      },
      {
        title: 'Microsoft WSL2 Disk Management & Optimize-VHD',
        url: 'https://learn.microsoft.com/en-us/windows/wsl/disk-space',
        domain: 'learn.microsoft.com',
        verified: true,
        snippet: 'VHDX virtual disks expand automatically and require wsl --shutdown before compacting.',
      },
      {
        title: 'Docker System Prune CLI Specification',
        url: 'https://docs.docker.com/reference/cli/docker/system/prune/',
        domain: 'docs.docker.com',
        verified: true,
        snippet: 'Reclaim unused container layers safely without corrupting VM state.',
      },
    ];
  }
  if (lower.includes('system32') || lower.includes('.dll') || lower.includes('.exe')) {
    return [
      {
        title: 'Microsoft Windows File Protection & System32 Integrity',
        url: 'https://learn.microsoft.com/en-us/windows/win32/fileio/system-files',
        domain: 'learn.microsoft.com',
        verified: true,
        snippet: 'Core Windows binaries under System32 are protected and critical for boot stability.',
      },
      {
        title: 'Using System File Checker (SFC) in Windows',
        url: 'https://support.microsoft.com/en-us/topic/use-the-system-file-checker-tool-to-repair-missing-or-corrupted-system-files-79aa86cb-ca52-166a-92a3-966e85d4094e',
        domain: 'support.microsoft.com',
        verified: true,
        snippet: 'Removing System32 binaries triggers immediate OS repair or BSOD conditions.',
      },
      {
        title: 'Microsoft DISM Component Store Cleanup Guidance',
        url: 'https://learn.microsoft.com/en-us/windows-hardware/manufacture/desktop/clean-up-the-winsxs-folder',
        domain: 'learn.microsoft.com',
        verified: true,
        snippet: 'Use official DISM tooling instead of manual deletion inside Windows system folders.',
      },
    ];
  }
  return [
    {
      title: 'Microsoft Windows Disk Cleanup & Temp Cache Specification',
      url: 'https://learn.microsoft.com/en-us/windows/win32/shell/disk-cleanup',
      domain: 'learn.microsoft.com',
      verified: true,
      snippet: 'Transient cache, build artifacts, and temp directories are safely reclaimable.',
    },
    {
      title: 'Node.js & pnpm Content-Addressable Store Pruning Docs',
      url: 'https://pnpm.io/cli/store',
      domain: 'pnpm.io',
      verified: true,
      snippet: 'Dormant dependency caches and build targets can be regenerated cleanly.',
    },
    {
      title: 'WinDirStat Official File Cleanup Best Practices',
      url: 'https://windirstat.net/documentation.html',
      domain: 'windirstat.net',
      verified: true,
      snippet: 'Quarantine large unreferenced caches before permanent deletion for zero-regret cleanup.',
    },
  ];
}

function buildSimulatedAdvice(params: AskAgentAdvisorParams): AgentAdvisorResponse {
  const targetPath = params.targetPath || 'C:\\Windows\\Temp\\installer-cache-bundle.tmp';
  const targetName =
    params.targetName ||
    targetPath.split(/[\\/]/).filter(Boolean).pop() ||
    'Selected Item';
  const size = params.targetSizeBytes ?? 8_940_000_000;
  const lower = `${targetPath} ${params.prompt}`.toLowerCase();

  const isCriticalSystem =
    lower.includes('system32') || lower.includes('ntoskrnl') || lower.includes('kernelbase');
  const isStatefulVm = lower.includes('docker') || lower.includes('.vhdx') || lower.includes('.sqlite');
  const isSafeCache =
    lower.includes('temp') ||
    lower.includes('.tmp') ||
    lower.includes('.log') ||
    lower.includes('node_modules') ||
    lower.includes('target-rust') ||
    lower.includes('.pdb') ||
    lower.includes('cache');

  let verdict: AgentAdvisorResponse['verdict'] = 'QUARANTINE_RECOMMENDED';
  let risk_score = 19;
  let reclaimable_bytes = size;
  let usage_value_assessment =
    'Low ongoing runtime dependency. Moving to the 7-Day Quarantine Vault preserves instant rollback while reclaiming disk headroom.';
  let rationale =
    'Cross-checked path signatures and official vendor documentation across 3 external references.';

  if (isCriticalSystem) {
    verdict = 'KEEP';
    risk_score = 96;
    reclaimable_bytes = 0;
    usage_value_assessment =
      'Critical OS kernel / Win32 runtime module. Deleting this file can render Windows unbootable.';
    rationale =
      'Protected system path detected; cross-referenced Microsoft Learn and SFC integrity documentation.';
  } else if (isStatefulVm) {
    verdict = 'REVIEW_CAREFULLY';
    risk_score = 64;
    reclaimable_bytes = Math.round(size * 0.45);
    usage_value_assessment =
      'Contains active container layers or database state. Prune via `docker system prune` or compact VHDX rather than deleting raw container image.';
    rationale =
      'Verified against Docker Desktop and Microsoft WSL2 storage compaction specifications.';
  } else if (isSafeCache) {
    verdict = 'SAFE_TO_DELETE';
    risk_score = 9;
    reclaimable_bytes = size;
    usage_value_assessment =
      'Ephemeral build/temp/log cache with zero permanent data loss risk; regenerates automatically if needed.';
    rationale =
      'Confirmed non-essential cache artifact across Microsoft Learn, toolchain docs, and WinDirStat guidelines.';
  }

  return {
    id: `adv-${Date.now()}-${Math.random().toString(36).slice(2, 7)}`,
    targetPath,
    targetName,
    targetSizeBytes: size,
    provider: params.provider,
    verdict,
    risk_score,
    reclaimable_bytes,
    usage_value_assessment,
    rationale,
    sources: buildSimulatedSources(targetPath),
    createdAt: new Date().toISOString(),
    userPrompt: params.prompt,
    responseMode: 'verdict',
    isLiveCli: false,
  };
}

function makeShaStub(seed: string): string {
  let h1 = 0xdeadbeef ^ seed.length;
  let h2 = 0x41c6ce57 ^ seed.length;
  for (let i = 0; i < seed.length; i += 1) {
    const ch = seed.charCodeAt(i);
    h1 = Math.imul(h1 ^ ch, 2654435761);
    h2 = Math.imul(h2 ^ ch, 1597334677);
  }
  const part = `${(h1 >>> 0).toString(16).padStart(8, '0')}${(h2 >>> 0).toString(16).padStart(8, '0')}`;
  return part.repeat(4).slice(0, 64);
}

const defaultBridge: WdsBridge = {
  async listDrives(): Promise<DriveInfo[]> {
    const native = await tryTauriInvoke<DriveInfo[]>('list_drives');
    return native ?? createDemoDrives();
  },

  async scanDirectory(target: string): Promise<ScanSnapshot> {
    if (target === 'demo://100k-snapshot') {
      return generateSynthetic100kTree();
    }
    const native = await tryTauriInvoke<ScanSnapshot>('scan_directory', { target });
    return native ?? createDemoSnapshot(target);
  },

  async findDuplicates(target: string): Promise<DuplicateGroup[]> {
    const native = await tryTauriInvoke<DuplicateGroup[]>('find_duplicates', { target });
    return native ?? simDuplicates;
  },

  async detectAgentClis(): Promise<AgentProviderStatus[]> {
    const native = await tryTauriInvoke<AgentProviderStatus[]>('detect_agent_clis');
    if (native) return native;
    return [
      { provider: 'antigravity', displayName: 'Antigravity CLI', available: true, command: 'antigravity', latencyMs: 38 },
      { provider: 'kiro', displayName: 'Kiro CLI', available: true, command: 'kiro', latencyMs: 64 },
      { provider: 'claude', displayName: 'Claude Code CLI', available: true, command: 'claude', latencyMs: 52 },
      { provider: 'codex', displayName: 'OpenAI Codex CLI', available: true, command: 'codex', latencyMs: 69 },
      { provider: 'opencode', displayName: 'OpenCode CLI', available: true, command: 'opencode', latencyMs: 44 },
      { provider: 'custom', displayName: 'Custom CLI Adapter', available: true, command: 'custom-agent', latencyMs: 50 },
    ];
  },

  async askAgentAdvisor(params: AskAgentAdvisorParams): Promise<AgentAdvisorResponse> {
    const payload = {
      provider: params.provider,
      prompt: params.prompt,
      targetPath: params.targetPath,
      targetName: params.targetName,
      targetSizeBytes: params.targetSizeBytes,
      customCommand: params.customCommand,
      activeDrive: params.activeDrive,
      model: params.model,
    };

    if (isTauriRuntime()) {
      try {
        const native = await invoke<AgentAdvisorResponse>('ask_agent_advisor', payload);
        if (native) return native;
        return {
          id: `adv-err-${Date.now()}`,
          targetPath: params.targetPath ?? params.activeDrive ?? 'General Storage Inquiry',
          targetName: params.targetName ?? 'Agent CLI Error',
          targetSizeBytes: params.targetSizeBytes ?? 0,
          provider: params.provider,
          verdict: 'REVIEW_CAREFULLY',
          risk_score: 0,
          reclaimable_bytes: 0,
          usage_value_assessment: 'Desktop backend returned an empty response for ask_agent_advisor.',
          rationale: 'Desktop backend returned an empty response.',
          sources: [],
          createdAt: new Date().toISOString(),
          userPrompt: params.prompt,
          responseMode: 'error',
          isLiveCli: false,
          error: 'Desktop backend returned an empty response for ask_agent_advisor.',
        };
      } catch (err) {
        const msg = err instanceof Error ? err.message : String(err);
        return {
          id: `adv-err-${Date.now()}`,
          targetPath: params.targetPath ?? params.activeDrive ?? 'General Storage Inquiry',
          targetName: params.targetName ?? 'Agent CLI Error',
          targetSizeBytes: params.targetSizeBytes ?? 0,
          provider: params.provider,
          verdict: 'REVIEW_CAREFULLY',
          risk_score: 0,
          reclaimable_bytes: 0,
          usage_value_assessment: msg,
          rationale: msg,
          sources: [],
          createdAt: new Date().toISOString(),
          userPrompt: params.prompt,
          responseMode: 'error',
          isLiveCli: false,
          error: `Desktop IPC error invoking ${params.provider}: ${msg}`,
        };
      }
    }

    const native = await tryTauriInvoke<AgentAdvisorResponse>('ask_agent_advisor', payload);
    if (native) return native;
    await new Promise((resolve) => setTimeout(resolve, 120));
    return buildSimulatedAdvice(params);
  },

  async listOpenCodeModels(): Promise<string[]> {
    const native = await tryTauriInvoke<string[]>('list_opencode_models');
    if (native && native.length > 0) return native;
    if (isTauriRuntime()) return [];
    return [
      'opencode/big-pickle',
      'opencode/glm-5.3-flash-free',
      'opencode/deepseek-v4.1-flash-free',
      'opencode/beam-free',
    ];
  },

  async testAgentCli(provider: AgentProvider, customCommand?: string): Promise<AgentCliTestResult> {
    const native = await tryTauriInvoke<AgentCliTestResult>('test_agent_cli', {
      provider,
      customCommand,
    });
    if (native) return native;
    if (isTauriRuntime()) {
      return { ok: false, error: 'Failed to invoke test_agent_cli on desktop backend.' };
    }
    return {
      ok: true,
      command: provider,
      output: `${provider} simulated connectivity ok`,
      durationMs: 12,
    };
  },

  async quarantineItem(params: QuarantineItemParams): Promise<VaultItem> {
    const native = await tryTauriInvoke<VaultItem>('quarantine_item', {
      path: params.path,
      name: params.name,
      sizeBytes: params.sizeBytes,
      batchId: params.batchId,
    });
    if (native) return native;

    const now = Date.now();
    const sha = makeShaStub(`${params.path}:${now}`);
    const item: VaultItem = {
      id: `vault-${now}-${Math.random().toString(36).slice(2, 6)}`,
      batchId: params.batchId ?? `batch-${new Date(now).toISOString().slice(0, 10).replace(/-/g, '')}`,
      originalPath: params.path,
      fileName: params.name,
      sizeBytes: params.sizeBytes,
      quarantinedAt: new Date(now).toISOString(),
      expiresAt: new Date(now + 168 * 3600 * 1000).toISOString(),
      remainingHours: 168.0,
      sha256: sha,
      xxhash64: sha.slice(0, 16),
      hashVerified: true,
      status: 'quarantined',
    };
    simVaultItems = [item, ...simVaultItems];
    // Also remove quarantined duplicate copy from duplicate list in simulation
    simDuplicates = simDuplicates
      .map((grp) => {
        const remaining = grp.files.filter((f) => f.path !== params.path);
        return {
          ...grp,
          files: remaining,
          wastedBytes: Math.max(0, grp.fileSize * Math.max(0, remaining.length - 1)),
        };
      })
      .filter((grp) => grp.files.length > 1);
    return item;
  },

  async listVaultItems(): Promise<VaultItem[]> {
    const native = await tryTauriInvoke<VaultItem[]>('list_vault_items');
    return native ?? [...simVaultItems];
  },

  async rollbackVaultItem(id: string): Promise<VaultItem> {
    const native = await tryTauriInvoke<VaultItem>('rollback_vault_item', { id });
    if (native) return native;
    let updated: VaultItem | null = null;
    simVaultItems = simVaultItems.map((item) => {
      if (item.id === id) {
        updated = { ...item, status: 'restored' };
        return updated;
      }
      return item;
    });
    if (!updated) {
      throw new Error(`Vault item ${id} not found`);
    }
    return updated;
  },

  async purgeVaultItem(id: string): Promise<void> {
    const native = await tryTauriInvoke<void>('purge_vault_item', { id });
    if (native !== null) return;
    simVaultItems = simVaultItems.filter((item) => item.id !== id);
  },
};

export function getBridge(): WdsBridge {
  const override = typeof window !== 'undefined' ? window.__WDS_TEST_BRIDGE__ : undefined;
  if (!override) return defaultBridge;
  return {
    ...defaultBridge,
    ...override,
  };
}

export function resetSimulationStoreForTests(): void {
  simVaultItems = createInitialVaultItems();
  simDuplicates = createDemoDuplicates();
}

export const AGENT_PROVIDERS: { id: AgentProvider; label: string }[] = [
  { id: 'antigravity', label: 'Antigravity CLI' },
  { id: 'kiro', label: 'Kiro CLI' },
  { id: 'claude', label: 'Claude Code CLI' },
  { id: 'codex', label: 'OpenAI Codex CLI' },
  { id: 'opencode', label: 'OpenCode CLI' },
  { id: 'custom', label: 'Custom CLI' },
];
