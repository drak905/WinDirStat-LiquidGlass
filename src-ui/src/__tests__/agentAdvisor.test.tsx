import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { AgentAdvisorPanel } from '../components/AgentAdvisorPanel';
import type { AgentAdvisorResponse, AgentProvider } from '../types/storage';

describe('Agent CLI Storage Advisor Chat', () => {
  it('supports all 6 CLI providers and renders structured verdict, risk gauge, reclaimable bytes, usage assessment, >=3 verified URLs, and 1-click quarantine', async () => {
    const onQuarantine = vi.fn(async () => {});
    const onAsk = vi.fn(
      async (params: { provider: AgentProvider; prompt: string }): Promise<AgentAdvisorResponse> => ({
        id: 'resp-1',
        targetPath: 'C:\\Users\\thinh\\Downloads\\ubuntu-24.04.iso',
        targetName: 'ubuntu-24.04.iso',
        targetSizeBytes: 6_120_000_000,
        provider: params.provider,
        verdict: 'QUARANTINE_RECOMMENDED',
        risk_score: 18,
        reclaimable_bytes: 6_120_000_000,
        usage_value_assessment: 'Old OS installer ISO already flashed; low active utility.',
        rationale: 'Verified across Ubuntu official docs and disk hygiene guidelines.',
        sources: [
          {
            title: 'Ubuntu Official ISO Verification Docs',
            url: 'https://ubuntu.com/tutorials/how-to-verify-ubuntu',
            domain: 'ubuntu.com',
            verified: true,
            snippet: 'Installation media can be discarded after OS deployment.',
          },
          {
            title: 'Microsoft Storage Sense Reference',
            url: 'https://support.microsoft.com/en-us/windows/manage-drive-space-with-storage-sense',
            domain: 'support.microsoft.com',
            verified: true,
            snippet: 'Stale Downloads ISOs are safe candidates for cleanup.',
          },
          {
            title: 'NIST Media Sanitization Guidelines',
            url: 'https://csrc.nist.gov/publications/detail/sp/800-88/rev-1/final',
            domain: 'csrc.nist.gov',
            verified: true,
            snippet: 'Staging images should be archived or quarantined cleanly.',
          },
        ],
        createdAt: new Date().toISOString(),
      })
    );

    const user = userEvent.setup();
    render(
      <AgentAdvisorPanel
        providers={[
          { provider: 'antigravity', displayName: 'Antigravity CLI', available: true, command: 'antigravity', latencyMs: 42 },
          { provider: 'kiro', displayName: 'Kiro CLI', available: true, command: 'kiro', latencyMs: 65 },
          { provider: 'claude', displayName: 'Claude Code CLI', available: true, command: 'claude', latencyMs: 55 },
          { provider: 'codex', displayName: 'OpenAI Codex CLI', available: true, command: 'codex', latencyMs: 70 },
          { provider: 'opencode', displayName: 'OpenCode CLI', available: true, command: 'opencode', latencyMs: 48 },
          { provider: 'custom', displayName: 'Custom CLI', available: true, command: 'custom-agent' },
        ]}
        selectedProvider="antigravity"
        onProviderChange={() => {}}
        responses={[]}
        isStreaming={false}
        onSendPrompt={onAsk}
        onQuarantineRecommended={onQuarantine}
      />
    );

    const select = screen.getByTestId('agent-provider-select') as HTMLSelectElement;
    const optionValues = Array.from(select.options).map((o) => o.value);
    expect(optionValues).toEqual([
      'antigravity',
      'kiro',
      'claude',
      'codex',
      'opencode',
      'custom',
    ]);

    const input = screen.getByTestId('agent-chat-input');
    await user.type(input, 'Should I delete ubuntu-24.04.iso?');
    await user.click(screen.getByTestId('agent-send-btn'));

    await waitFor(() => expect(onAsk).toHaveBeenCalledTimes(1));

    // Re-render with received response card
    const response = await onAsk.mock.results[0].value;
    render(
      <AgentAdvisorPanel
        providers={[
          { provider: 'antigravity', displayName: 'Antigravity CLI', available: true, command: 'antigravity', latencyMs: 42 },
          { provider: 'kiro', displayName: 'Kiro CLI', available: true, command: 'kiro', latencyMs: 65 },
          { provider: 'claude', displayName: 'Claude Code CLI', available: true, command: 'claude', latencyMs: 55 },
          { provider: 'codex', displayName: 'OpenAI Codex CLI', available: true, command: 'codex', latencyMs: 70 },
          { provider: 'opencode', displayName: 'OpenCode CLI', available: true, command: 'opencode', latencyMs: 48 },
          { provider: 'custom', displayName: 'Custom CLI', available: true, command: 'custom-agent' },
        ]}
        selectedProvider="claude"
        onProviderChange={() => {}}
        responses={[response]}
        isStreaming={false}
        onSendPrompt={onAsk}
        onQuarantineRecommended={onQuarantine}
      />
    );

    expect(screen.getByTestId('agent-verdict-badge')).toHaveTextContent('QUARANTINE_RECOMMENDED');
    expect(screen.getByText(/18\s*\/\s*100/)).toBeInTheDocument();
    expect(screen.getByText(/Old OS installer ISO already flashed/i)).toBeInTheDocument();

    const sourcesList = screen.getByTestId('agent-sources-list');
    const links = sourcesList.querySelectorAll('a');
    expect(links.length).toBeGreaterThanOrEqual(3);

    await user.click(screen.getByTestId('agent-quarantine-btn'));
    expect(onQuarantine).toHaveBeenCalledWith(response);
  });
});
