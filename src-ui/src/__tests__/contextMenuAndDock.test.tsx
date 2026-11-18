import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import App from '../App';
import type { AgentAdvisorResponse, VaultItem } from '../types/storage';

describe('Floating Glass Dock, Treemap Breadcrumbs & Context Menu Integration', () => {
  it('renders glass dock, drive selector, all 6 nav tabs, and treemap canvas', async () => {
    render(<App />);

    expect(await screen.findByTestId('glass-dock')).toBeInTheDocument();
    expect(screen.getByTestId('drive-selector')).toBeInTheDocument();
    expect(screen.getByTestId('nav-treemap')).toBeInTheDocument();
    expect(screen.getByTestId('nav-sunburst')).toBeInTheDocument();
    expect(screen.getByTestId('nav-flamegraph')).toBeInTheDocument();
    expect(screen.getByTestId('nav-duplicates')).toBeInTheDocument();
    expect(screen.getByTestId('nav-advisor')).toBeInTheDocument();
    expect(screen.getByTestId('nav-vault')).toBeInTheDocument();
    expect(screen.getByTestId('treemap-canvas')).toBeInTheDocument();
  });

  it('opens glass context menu on right-click and dispatches Ask Agent CLI and 7-Day Quarantine Vault actions', async () => {
    const askSpy = vi.fn(
      async (): Promise<AgentAdvisorResponse> => ({
        id: 'adv-test-1',
        targetPath: 'C:\\Windows\\Temp\\installer-cache',
        targetName: 'installer-cache',
        targetSizeBytes: 8_400_000_000,
        provider: 'antigravity',
        verdict: 'SAFE_TO_DELETE',
        risk_score: 12,
        reclaimable_bytes: 8_400_000_000,
        usage_value_assessment: 'Orphaned MSI installer cache; safe to purge.',
        rationale: 'Verified safe across Microsoft Learn and community docs.',
        sources: [
          {
            title: 'Microsoft Windows Disk Cleanup Guide',
            url: 'https://learn.microsoft.com/en-us/windows/win32/shell/disk-cleanup',
            domain: 'learn.microsoft.com',
            verified: true,
            snippet: 'Temporary installer caches can be safely removed.',
          },
          {
            title: 'Windows Temp Directory Lifecycle',
            url: 'https://docs.microsoft.com/en-us/windows/deployment/upgrade',
            domain: 'docs.microsoft.com',
            verified: true,
            snippet: 'Completed setup caches are not required at runtime.',
          },
          {
            title: 'WinDirStat Community Cleanup Reference',
            url: 'https://windirstat.net/documentation.html',
            domain: 'windirstat.net',
            verified: true,
            snippet: 'Temp installation folders are high-yield reclaim targets.',
          },
        ],
        createdAt: new Date().toISOString(),
      })
    );

    const quarantineSpy = vi.fn(
      async (params: { path: string; name: string; sizeBytes: number }): Promise<VaultItem> => ({
        id: 'vault-ctx-1',
        batchId: 'batch-ctx-1',
        originalPath: params.path,
        fileName: params.name,
        sizeBytes: params.sizeBytes,
        quarantinedAt: new Date().toISOString(),
        expiresAt: new Date(Date.now() + 168 * 3600 * 1000).toISOString(),
        remainingHours: 168,
        sha256: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
        xxhash64: '9f86d081884c7d65',
        hashVerified: true,
        status: 'quarantined',
      })
    );

    window.__WDS_TEST_BRIDGE__ = {
      askAgentAdvisor: askSpy,
      quarantineItem: quarantineSpy,
    };

    const user = userEvent.setup();
    render(<App />);

    const canvas = await screen.findByTestId('treemap-canvas');
    fireEvent.contextMenu(canvas, { clientX: 180, clientY: 160 });

    const menu = await screen.findByTestId('treemap-context-menu');
    expect(menu).toBeInTheDocument();
    expect(screen.getByTestId('ctx-ask-agent')).toBeInTheDocument();
    expect(screen.getByTestId('ctx-quarantine')).toBeInTheDocument();

    // Test Quarantine action from context menu
    await user.click(screen.getByTestId('ctx-quarantine'));
    await waitFor(() => expect(quarantineSpy).toHaveBeenCalledTimes(1));

    // Re-open context menu on an extension group row and test Ask Agent CLI
    const extRow = await screen.findByTestId('ext-group-.iso');
    fireEvent.contextMenu(extRow, { clientX: 220, clientY: 220 });
    await screen.findByTestId('treemap-context-menu');
    await user.click(screen.getByTestId('ctx-ask-agent'));

    await waitFor(() => expect(askSpy).toHaveBeenCalledTimes(1));
    expect(await screen.findByTestId('agent-verdict-badge')).toHaveTextContent(
      'SAFE_TO_DELETE'
    );
  });

  it('supports breadcrumb zoom-in and zoom-out in TreemapView', async () => {
    const user = userEvent.setup();
    render(<App />);

    const dirRow = await screen.findByTestId('dir-node-Users');
    await user.dblClick(dirRow);

    const crumbRoot = await screen.findByTestId('breadcrumb-root');
    expect(screen.getByTestId('treemap-breadcrumbs')).toHaveTextContent('Users');

    await user.click(crumbRoot);
    await waitFor(() => {
      expect(screen.getByTestId('treemap-breadcrumbs')).not.toHaveTextContent('Users');
    });
  });
});
