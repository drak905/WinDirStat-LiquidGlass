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

  it('supports switching to kiro, shows honest status pills, and renders chat and error response modes cleanly', async () => {
    const user = userEvent.setup();
    const onProviderChange = vi.fn();
    const kiroChatResponse: AgentAdvisorResponse = {
      id: 'kiro-chat-1',
      targetPath: 'General Storage Inquiry',
      targetName: 'General Storage Inquiry',
      targetSizeBytes: 0,
      provider: 'kiro',
      verdict: 'REVIEW_CAREFULLY',
      risk_score: 0,
      reclaimable_bytes: 0,
      usage_value_assessment: 'Kiro CLI live reply: Keep Windows Update caches unless running Disk Cleanup.',
      rationale: 'Live reply from kiro-cli.exe (2.4s)',
      sources: [
        {
          title: 'learn.microsoft.com',
          url: 'https://learn.microsoft.com/en-us/windows/win32/shell/disk-cleanup',
          domain: 'learn.microsoft.com',
          verified: true,
          snippet: 'Referenced in kiro-cli.exe response',
        },
      ],
      createdAt: new Date().toISOString(),
      userPrompt: 'Hello Kiro CLI, how should I free space safely?',
      responseMode: 'chat',
      assistantReply: 'Kiro CLI live reply: Keep Windows Update caches unless running Disk Cleanup.',
      isLiveCli: true,
      resolvedCommand: 'kiro-cli.exe',
      durationMs: 2400,
    };

    const kiroErrorResponse: AgentAdvisorResponse = {
      id: 'kiro-err-1',
      targetPath: 'General Storage Inquiry',
      targetName: 'General Storage Inquiry',
      targetSizeBytes: 0,
      provider: 'claude',
      verdict: 'REVIEW_CAREFULLY',
      risk_score: 0,
      reclaimable_bytes: 0,
      usage_value_assessment: 'Claude Code CLI was not found on PATH.',
      rationale: 'Claude Code CLI was not found on PATH.',
      sources: [],
      createdAt: new Date().toISOString(),
      userPrompt: 'Test offline CLI',
      responseMode: 'error',
      isLiveCli: false,
      resolvedCommand: 'claude',
      error: 'Claude Code CLI was not found on PATH.',
      installHint: 'npm install -g @anthropic-ai/claude-code',
    };

    const { rerender } = render(
      <AgentAdvisorPanel
        providers={[
          { provider: 'antigravity', displayName: 'Antigravity CLI', available: true, command: 'agy', latencyMs: 42 },
          { provider: 'kiro', displayName: 'Kiro CLI', available: true, command: 'kiro-cli', latencyMs: 45, resolvedPath: 'C:\\Users\\thinh\\AppData\\Local\\Kiro-Cli\\kiro-cli.exe' },
          { provider: 'claude', displayName: 'Claude Code CLI', available: false, command: 'claude', latencyMs: 0, installHint: 'npm install -g @anthropic-ai/claude-code' },
        ]}
        selectedProvider="kiro"
        onProviderChange={onProviderChange}
        responses={[kiroChatResponse]}
        isStreaming={false}
        onSendPrompt={async () => {}}
        onQuarantineRecommended={async () => {}}
      />
    );

    expect(screen.getByTestId('agent-status-pill')).toHaveTextContent(/Connected.*kiro-cli/i);
    expect(screen.getByTestId('agent-user-prompt')).toHaveTextContent('Hello Kiro CLI, how should I free space safely?');
    expect(screen.getByTestId('agent-live-badge')).toHaveTextContent(/Live CLI.*kiro-cli\.exe.*2\.4s/);
    expect(screen.getByTestId('agent-chat-card')).toHaveTextContent('Kiro CLI live reply');
    expect(screen.queryByTestId('agent-quarantine-btn')).not.toBeInTheDocument();

    // Switching selector to another option calls onProviderChange
    await user.selectOptions(screen.getByTestId('agent-provider-select'), 'claude');
    expect(onProviderChange).toHaveBeenCalledWith('claude');

    // When unavailable provider is active, Not Installed status pill and banner appear alongside error card
    rerender(
      <AgentAdvisorPanel
        providers={[
          { provider: 'kiro', displayName: 'Kiro CLI', available: true, command: 'kiro-cli', latencyMs: 45 },
          { provider: 'claude', displayName: 'Claude Code CLI', available: false, command: 'claude', latencyMs: 0, installHint: 'npm install -g @anthropic-ai/claude-code' },
        ]}
        selectedProvider="claude"
        onProviderChange={onProviderChange}
        responses={[kiroErrorResponse]}
        isStreaming={false}
        onSendPrompt={async () => {}}
        onQuarantineRecommended={async () => {}}
      />
    );

    expect(screen.getByTestId('agent-status-pill')).toHaveTextContent('Not Installed');
    expect(screen.getByTestId('agent-unavailable-banner')).toHaveTextContent('npm install -g @anthropic-ai/claude-code');
    expect(screen.getByTestId('agent-error-card')).toHaveTextContent('Claude Code CLI was not found on PATH.');
  });

  it('shows OpenCode model selector when opencode is active and forwards selected model to onSendPrompt', async () => {
    const user = userEvent.setup();
    const onSend = vi.fn(async () => ({}));

    render(
      <AgentAdvisorPanel
        providers={[
          { provider: 'opencode', displayName: 'OpenCode CLI', available: true, command: 'opencode', latencyMs: 44 },
        ]}
        selectedProvider="opencode"
        onProviderChange={() => {}}
        opencodeModels={['opencode/big-pickle', 'opencode/glm-5.3-flash-free']}
        responses={[]}
        isStreaming={false}
        onSendPrompt={onSend}
        onQuarantineRecommended={async () => {}}
      />
    );

    const modelSelect = screen.getByTestId('opencode-model-select') as HTMLSelectElement;
    expect(Array.from(modelSelect.options).map((o) => o.value)).toEqual([
      '',
      'opencode/big-pickle',
      'opencode/glm-5.3-flash-free',
    ]);

    await user.selectOptions(modelSelect, 'opencode/big-pickle');
    await user.type(screen.getByTestId('agent-chat-input'), 'Say hello from OpenCode');
    await user.click(screen.getByTestId('agent-send-btn'));

    await waitFor(() => expect(onSend).toHaveBeenCalledTimes(1));
    expect(onSend).toHaveBeenCalledWith({
      provider: 'opencode',
      prompt: 'Say hello from OpenCode',
      customCommand: undefined,
      model: 'opencode/big-pickle',
    });
  });
});
