import React, { useState } from 'react';
import {
  Bot,
  Send,
  ShieldAlert,
  ExternalLink,
  CheckCircle2,
  Wifi,
  WifiOff,
  AlertTriangle,
  Sparkles,
  RefreshCw,
} from 'lucide-react';
import type {
  AgentAdvisorResponse,
  AgentProvider,
  AgentProviderStatus,
  AgentVerdict,
} from '../types/storage';
import { AGENT_PROVIDERS } from '../services/tauriBridge';
import { formatBytes } from '../utils/formatters';

const FALLBACK_OPENCODE_MODELS = [
  'opencode/big-pickle',
  'opencode/glm-5.3-flash-free',
  'opencode/deepseek-v4.1-flash-free',
  'opencode/beam-free',
];

interface AgentAdvisorPanelProps {
  providers: AgentProviderStatus[];
  selectedProvider: AgentProvider;
  onProviderChange: (provider: AgentProvider) => void;
  opencodeModels?: string[];
  responses: AgentAdvisorResponse[];
  isStreaming: boolean;
  onSendPrompt: (params: {
    provider: AgentProvider;
    prompt: string;
    targetPath?: string;
    targetName?: string;
    targetSizeBytes?: number;
    customCommand?: string;
    model?: string;
  }) => Promise<unknown>;
  onRefreshProviders?: () => Promise<void>;
  onQuarantineRecommended: (response: AgentAdvisorResponse) => Promise<void>;
}

function getVerdictStyle(verdict: AgentVerdict): string {
  switch (verdict) {
    case 'SAFE_TO_DELETE':
      return 'bg-emerald-500/25 text-emerald-700 border-emerald-400/40 dark:text-emerald-300';
    case 'QUARANTINE_RECOMMENDED':
      return 'bg-amber-500/25 text-amber-800 border-amber-400/40 dark:text-amber-300';
    case 'REVIEW_CAREFULLY':
      return 'bg-orange-500/25 text-orange-800 border-orange-400/40 dark:text-orange-300';
    case 'KEEP':
      return 'bg-rose-500/25 text-rose-800 border-rose-400/40 dark:text-rose-300';
  }
}

function getRiskColor(score: number): string {
  if (score <= 25) return 'bg-emerald-400';
  if (score <= 60) return 'bg-amber-400';
  return 'bg-rose-500';
}

export const AgentAdvisorPanel: React.FC<AgentAdvisorPanelProps> = ({
  providers,
  selectedProvider,
  onProviderChange,
  opencodeModels,
  responses,
  isStreaming,
  onSendPrompt,
  onRefreshProviders,
  onQuarantineRecommended,
}) => {
  const [promptInput, setPromptInput] = useState('');
  const [customCmd, setCustomCmd] = useState('my-local-agent --json');
  const [selectedOpenCodeModel, setSelectedOpenCodeModel] = useState('');
  const [quarantinedIds, setQuarantinedIds] = useState<Record<string, boolean>>({});
  const [isRefreshingProviders, setIsRefreshingProviders] = useState(false);

  const activeProviderStatus = providers.find((p) => p.provider === selectedProvider);
  const availableBuiltins = providers.filter((p) => p.available && p.provider !== 'custom');
  const openCodeModelOptions =
    opencodeModels && opencodeModels.length > 0 ? opencodeModels : FALLBACK_OPENCODE_MODELS;

  const activeModelOverride =
    selectedProvider === 'opencode' && selectedOpenCodeModel ? selectedOpenCodeModel : undefined;

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = promptInput.trim();
    if (!trimmed || isStreaming) return;
    setPromptInput('');
    await onSendPrompt({
      provider: selectedProvider,
      prompt: trimmed,
      customCommand: selectedProvider === 'custom' ? customCmd : undefined,
      model: activeModelOverride,
    });
  };

  const handleRefreshProviders = async () => {
    if (!onRefreshProviders || isRefreshingProviders) return;
    setIsRefreshingProviders(true);
    try {
      await onRefreshProviders();
    } finally {
      setIsRefreshingProviders(false);
    }
  };

  const renderStatusPill = () => {
    if (providers.length === 0) {
      return (
        <div
          data-testid="agent-status-pill"
          className="flex items-center gap-1.5 rounded-full border border-white/25 bg-white/15 px-2.5 py-1 text-[11px] font-medium text-slate-700 dark:text-slate-300"
        >
          <Wifi className="h-3 w-3 animate-pulse" />
          <span>Checking CLIs…</span>
        </div>
      );
    }
    if (activeProviderStatus?.available) {
      return (
        <div
          data-testid="agent-status-pill"
          title={activeProviderStatus.resolvedPath ?? activeProviderStatus.command}
          className="flex items-center gap-1.5 rounded-full border border-emerald-400/30 bg-emerald-500/15 px-2.5 py-1 text-[11px] font-medium text-emerald-700 dark:text-emerald-300"
        >
          <Wifi className="h-3 w-3" />
          <span>
            Connected &bull; {activeProviderStatus.command}
            {activeProviderStatus.latencyMs ? ` • ${activeProviderStatus.latencyMs}ms` : ''}
          </span>
        </div>
      );
    }
    return (
      <div
        data-testid="agent-status-pill"
        title={activeProviderStatus?.installHint}
        className="flex items-center gap-1.5 rounded-full border border-amber-400/40 bg-amber-500/15 px-2.5 py-1 text-[11px] font-medium text-amber-800 dark:text-amber-300"
      >
        <WifiOff className="h-3 w-3" />
        <span>Not Installed</span>
      </div>
    );
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      {/* Header & Provider Selector */}
      <div className="liquid-glass-card flex flex-col gap-2.5 rounded-2xl px-4 py-3">
        <div className="flex flex-wrap items-center justify-between gap-3">
          <div className="flex items-center gap-3">
            <div className="flex h-9 w-9 items-center justify-center rounded-xl bg-gradient-to-br from-sky-400/30 to-indigo-500/30 text-sky-400">
              <Bot className="h-5 w-5" />
            </div>
            <div>
              <div className="text-xs font-semibold text-slate-900 dark:text-white">
                Agent CLI Storage Advisor
              </div>
              <div className="text-[11px] text-slate-600 dark:text-slate-400">
                Pluggable local CLI router with &ge;3 internet cross-checked references
              </div>
            </div>
          </div>

          <div className="flex flex-wrap items-center gap-2">
            {renderStatusPill()}

            {onRefreshProviders && (
              <button
                type="button"
                aria-label="Refresh detected Agent CLIs"
                onClick={() => void handleRefreshProviders()}
                disabled={isRefreshingProviders}
                title="Re-scan local Agent CLIs on this PC"
                className="flex h-7 w-7 items-center justify-center rounded-xl border border-white/20 bg-white/15 text-slate-700 hover:bg-white/25 disabled:opacity-50 dark:text-slate-200"
              >
                <RefreshCw
                  className={`h-3.5 w-3.5 ${isRefreshingProviders ? 'animate-spin' : ''}`}
                />
              </button>
            )}

            <select
              data-testid="agent-provider-select"
              aria-label="Agent CLI Provider"
              value={selectedProvider}
              onChange={(e) => onProviderChange(e.target.value as AgentProvider)}
              className="rounded-xl border border-white/25 bg-white/25 px-3 py-1.5 text-xs font-semibold text-slate-900 outline-none focus:ring-2 focus:ring-sky-400/60 dark:bg-slate-900/75 dark:text-slate-100"
            >
              {AGENT_PROVIDERS.map((p) => (
                <option key={p.id} value={p.id} className="bg-slate-900 text-slate-100">
                  {p.label}
                </option>
              ))}
            </select>

            {selectedProvider === 'opencode' && (
              <select
                data-testid="opencode-model-select"
                aria-label="OpenCode Model"
                value={selectedOpenCodeModel}
                onChange={(e) => setSelectedOpenCodeModel(e.target.value)}
                title="Choose an OpenCode model or Auto fallback"
                className="max-w-52 rounded-xl border border-white/25 bg-white/25 px-2.5 py-1.5 font-mono text-[11px] font-medium text-slate-900 outline-none focus:ring-2 focus:ring-sky-400/60 dark:bg-slate-900/75 dark:text-slate-100"
              >
                <option value="" className="bg-slate-900 text-slate-100">
                  Auto (Config / Free Fallback)
                </option>
                {openCodeModelOptions.map((modelId) => (
                  <option key={modelId} value={modelId} className="bg-slate-900 text-slate-100">
                    {modelId}
                  </option>
                ))}
              </select>
            )}

            {selectedProvider === 'custom' && (
              <input
                type="text"
                value={customCmd}
                onChange={(e) => setCustomCmd(e.target.value)}
                placeholder="Custom CLI binary..."
                className="w-44 rounded-xl border border-white/25 bg-white/20 px-2.5 py-1.5 font-mono text-[11px] text-slate-900 outline-none dark:bg-black/30 dark:text-slate-100"
              />
            )}
          </div>
        </div>

        {activeProviderStatus && !activeProviderStatus.available && (
          <div
            data-testid="agent-unavailable-banner"
            className="flex flex-wrap items-center justify-between gap-2 rounded-xl border border-amber-400/30 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-900 dark:text-amber-200"
          >
            <div>
              <strong>{activeProviderStatus.displayName}</strong> was not detected on this PC.{' '}
              {activeProviderStatus.installHint && (
                <span className="opacity-90">{activeProviderStatus.installHint}</span>
              )}
            </div>
            {availableBuiltins.length > 0 && (
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="opacity-80">Switch to detected CLI:</span>
                {availableBuiltins.map((p) => (
                  <button
                    key={p.provider}
                    type="button"
                    onClick={() => onProviderChange(p.provider)}
                    className="rounded-lg border border-emerald-400/40 bg-emerald-500/20 px-2 py-0.5 font-semibold text-emerald-800 hover:bg-emerald-500/30 dark:text-emerald-200"
                  >
                    {p.displayName}
                  </button>
                ))}
              </div>
            )}
          </div>
        )}
      </div>

      {/* Streaming Response Cards Feed */}
      <div className="glass-scroll min-h-0 flex-1 space-y-3 overflow-y-auto pr-1">
        {isStreaming && (
          <div className="liquid-glass-card liquid-spring-enter flex items-center gap-3 rounded-3xl p-4 text-xs">
            <Sparkles className="h-4 w-4 animate-pulse text-sky-400" />
            <span className="font-medium text-slate-800 dark:text-slate-200">
              Running{' '}
              <strong className="font-mono">
                {activeProviderStatus?.command ?? selectedProvider}
              </strong>{' '}
              CLI&hellip; (live queries can take 10&ndash;40s)
            </span>
          </div>
        )}

        {responses.length === 0 && !isStreaming ? (
          <div className="liquid-glass-card flex h-56 flex-col items-center justify-center gap-3 rounded-3xl p-6 text-center">
            <Bot className="h-8 w-8 text-sky-400/80" />
            <div className="max-w-md space-y-1">
              <div className="text-sm font-semibold text-slate-900 dark:text-white">
                Ask before deleting unknown directories or large binaries
              </div>
              <p className="text-xs text-slate-600 dark:text-slate-400">
                Right-click any file, folder, or extension in the Treemap, or ask directly below.
                Every path verdict cross-checks at least 3 authoritative documentation URLs.
              </p>
            </div>
            <div className="flex flex-wrap justify-center gap-2">
              {[
                'Should I delete C:\\Windows\\Temp\\installer-cache-bundle.tmp?',
                'Is docker_data.vhdx safe to remove?',
                'Check C:\\Users\\thinh\\Downloads\\ubuntu-24.04.1-desktop-amd64.iso',
              ].map((suggestion) => (
                <button
                  key={suggestion}
                  type="button"
                  disabled={isStreaming}
                  onClick={() =>
                    void onSendPrompt({
                      provider: selectedProvider,
                      prompt: suggestion,
                      customCommand: selectedProvider === 'custom' ? customCmd : undefined,
                      model: activeModelOverride,
                    })
                  }
                  className="rounded-xl border border-white/20 bg-white/15 px-3 py-1.5 text-[11px] font-medium text-slate-800 hover:bg-white/25 disabled:cursor-not-allowed disabled:opacity-50 dark:text-slate-200"
                >
                  {suggestion}
                </button>
              ))}
            </div>
          </div>
        ) : (
          responses.map((resp) => {
            const isQuarantined = Boolean(quarantinedIds[resp.id]);
            const mode = resp.responseMode ?? 'verdict';

            const renderHeaderMeta = () => (
              <>
                {resp.userPrompt && (
                  <div
                    data-testid="agent-user-prompt"
                    className="mb-2.5 rounded-xl border border-white/15 bg-white/10 px-3 py-1.5 text-[11px] text-slate-700 dark:bg-black/25 dark:text-slate-300"
                  >
                    <span className="font-semibold text-slate-900 dark:text-white">You: </span>
                    {resp.userPrompt}
                  </div>
                )}
              </>
            );

            const renderLiveBadge = () =>
              resp.isLiveCli ? (
                <span
                  data-testid="agent-live-badge"
                  className="rounded-lg border border-emerald-400/35 bg-emerald-500/15 px-2 py-0.5 font-mono text-[10px] font-semibold text-emerald-700 dark:text-emerald-300"
                >
                  Live CLI &bull; {resp.resolvedCommand ?? resp.provider}
                  {resp.durationMs ? ` • ${(resp.durationMs / 1000).toFixed(1)}s` : ''}
                </span>
              ) : null;

            const renderSourcesGrid = (sources: AgentAdvisorResponse['sources']) => (
              <div className="mt-3">
                <div className="mb-1.5 text-[11px] font-semibold uppercase tracking-wider text-slate-600 dark:text-slate-400">
                  Cross-Checked External Sources ({sources.length})
                </div>
                <div
                  data-testid="agent-sources-list"
                  className="grid grid-cols-1 gap-2 md:grid-cols-3"
                >
                  {sources.map((src) => (
                    <a
                      key={src.url}
                      href={src.url}
                      target="_blank"
                      rel="noopener noreferrer"
                      className="group flex flex-col justify-between rounded-2xl border border-white/15 bg-white/10 p-2.5 text-xs transition hover:border-sky-400/50 hover:bg-white/20 dark:bg-black/25"
                    >
                      <div>
                        <div className="flex items-center justify-between gap-1">
                          <span className="flex items-center gap-1 text-[10px] font-semibold text-emerald-600 dark:text-emerald-300">
                            <CheckCircle2 className="h-3 w-3" />
                            {src.domain}
                          </span>
                          <ExternalLink className="h-3 w-3 text-slate-400 group-hover:text-sky-400" />
                        </div>
                        <div className="mt-1 line-clamp-2 font-medium text-slate-900 dark:text-slate-100">
                          {src.title}
                        </div>
                      </div>
                      <p className="mt-1 line-clamp-2 text-[10px] text-slate-600 dark:text-slate-400">
                        {src.snippet}
                      </p>
                    </a>
                  ))}
                </div>
              </div>
            );

            if (mode === 'error') {
              return (
                <article
                  key={resp.id}
                  data-testid="agent-error-card"
                  className="liquid-glass-card liquid-spring-enter rounded-3xl border border-rose-400/40 p-4"
                >
                  {renderHeaderMeta()}
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="rounded-full border border-rose-400/40 bg-rose-500/20 px-2.5 py-0.5 text-[11px] font-bold tracking-wide text-rose-800 dark:text-rose-200">
                      CLI ERROR
                    </span>
                    <span className="rounded-lg bg-white/15 px-2 py-0.5 font-mono text-[10px] text-slate-700 dark:text-slate-300">
                      provider: {resp.resolvedCommand ?? resp.provider}
                    </span>
                  </div>
                  <p className="mt-2.5 whitespace-pre-wrap break-words text-xs font-medium text-rose-900 dark:text-rose-200">
                    {resp.error || resp.rationale}
                  </p>
                  {resp.installHint && (
                    <div className="mt-2 rounded-xl border border-amber-400/30 bg-amber-500/10 px-3 py-2 text-[11px] text-amber-900 dark:text-amber-200">
                      <strong>Hint:</strong> {resp.installHint}
                    </div>
                  )}
                </article>
              );
            }

            if (mode === 'chat') {
              const replyBody = resp.assistantReply || resp.usage_value_assessment;
              return (
                <article
                  key={resp.id}
                  data-testid="agent-chat-card"
                  className="liquid-glass-card liquid-spring-enter rounded-3xl p-4"
                >
                  {renderHeaderMeta()}
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="rounded-full border border-sky-400/40 bg-sky-500/20 px-2.5 py-0.5 text-[11px] font-bold tracking-wide text-sky-800 dark:text-sky-200">
                      AGENT REPLY
                    </span>
                    <span className="rounded-lg bg-white/15 px-2 py-0.5 font-mono text-[10px] text-slate-700 dark:text-slate-300">
                      provider: {resp.provider}
                    </span>
                    {renderLiveBadge()}
                  </div>

                  <div className="mt-3 whitespace-pre-wrap break-words rounded-2xl border border-white/15 bg-white/10 p-3 text-xs leading-relaxed text-slate-800 dark:bg-black/20 dark:text-slate-200">
                    {replyBody}
                  </div>

                  {resp.sources.length > 0 && renderSourcesGrid(resp.sources)}
                </article>
              );
            }

            return (
              <article
                key={resp.id}
                className="liquid-glass-card liquid-spring-enter rounded-3xl p-4"
              >
                {renderHeaderMeta()}
                <div className="flex flex-wrap items-start justify-between gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-2">
                      <span
                        data-testid="agent-verdict-badge"
                        className={`rounded-full border px-2.5 py-0.5 text-[11px] font-bold tracking-wide ${getVerdictStyle(resp.verdict)}`}
                      >
                        {resp.verdict}
                      </span>
                      <span className="rounded-lg bg-white/15 px-2 py-0.5 font-mono text-[10px] text-slate-700 dark:text-slate-300">
                        provider: {resp.provider}
                      </span>
                      {renderLiveBadge()}
                      <span className="font-mono text-xs font-bold text-sky-600 dark:text-sky-300">
                        Reclaimable: {formatBytes(resp.reclaimable_bytes)}
                      </span>
                    </div>
                    <h3 className="mt-1.5 truncate text-sm font-semibold text-slate-900 dark:text-white">
                      {resp.targetName}
                    </h3>
                    <div className="truncate font-mono text-[11px] text-slate-600 dark:text-slate-400">
                      {resp.targetPath}
                    </div>
                  </div>

                  {/* Risk Score Gauge (0-100) */}
                  <div className="w-40 shrink-0 rounded-2xl border border-white/15 bg-white/10 p-2.5 dark:bg-black/25">
                    <div className="flex items-center justify-between text-[11px]">
                      <span className="font-medium text-slate-700 dark:text-slate-300">
                        Risk Score
                      </span>
                      <span className="font-mono font-bold text-slate-900 dark:text-white">
                        {resp.risk_score} / 100
                      </span>
                    </div>
                    <div className="mt-1.5 h-2 w-full overflow-hidden rounded-full bg-slate-900/30 dark:bg-white/10">
                      <div
                        className={`h-full rounded-full transition-all duration-500 ${getRiskColor(resp.risk_score)}`}
                        style={{ width: `${Math.max(4, Math.min(100, resp.risk_score))}%` }}
                      />
                    </div>
                  </div>
                </div>

                {/* Usage Value Assessment */}
                <div className="mt-3 rounded-2xl border border-white/15 bg-white/10 p-3 text-xs dark:bg-black/20">
                  <div className="mb-1 flex items-center gap-1.5 font-semibold text-slate-900 dark:text-white">
                    <AlertTriangle className="h-3.5 w-3.5 text-amber-400" />
                    Usage Value Assessment
                  </div>
                  <p className="whitespace-pre-wrap break-words leading-relaxed text-slate-700 dark:text-slate-300">
                    {resp.usage_value_assessment}
                  </p>
                  <p className="mt-1 whitespace-pre-wrap break-words text-[11px] text-slate-600 dark:text-slate-400">
                    {resp.rationale}
                  </p>
                </div>

                {/* Verified Cross-Checked External Source URLs (>= 3) */}
                {resp.sources.length > 0 && renderSourcesGrid(resp.sources)}

                {/* 1-Click Quarantine Action */}
                <div className="mt-3 flex items-center justify-end gap-2">
                  <button
                    type="button"
                    data-testid="agent-quarantine-btn"
                    disabled={isQuarantined || resp.verdict === 'KEEP'}
                    onClick={async () => {
                      await onQuarantineRecommended(resp);
                      setQuarantinedIds((prev) => ({ ...prev, [resp.id]: true }));
                    }}
                    className="liquid-glass-interactive flex items-center gap-1.5 rounded-xl bg-gradient-to-r from-amber-500/90 to-orange-500/90 px-3.5 py-2 text-xs font-semibold text-slate-950 shadow-md disabled:cursor-not-allowed disabled:opacity-50"
                  >
                    <ShieldAlert className="h-4 w-4" />
                    {isQuarantined
                      ? 'Moved to 7-Day Vault'
                      : 'Quarantine via 7-Day Vault'}
                  </button>
                </div>
              </article>
            );
          })
        )}
      </div>

      {/* Chat Prompt Composer */}
      <form
        onSubmit={handleSubmit}
        className="liquid-glass-card flex items-center gap-2 rounded-2xl p-2"
      >
        <input
          type="text"
          data-testid="agent-chat-input"
          value={promptInput}
          onChange={(e) => setPromptInput(e.target.value)}
          placeholder={`Ask ${activeProviderStatus?.command ?? selectedProvider} CLI a question or enter a Windows path…`}
          className="flex-1 bg-transparent px-3 py-2 text-xs text-slate-900 placeholder-slate-500 outline-none dark:text-slate-100"
        />
        <button
          type="submit"
          data-testid="agent-send-btn"
          disabled={isStreaming || !promptInput.trim()}
          className="liquid-glass-interactive flex items-center gap-1.5 rounded-xl bg-gradient-to-r from-sky-500 to-indigo-500 px-4 py-2 text-xs font-semibold text-white shadow-md disabled:opacity-50"
        >
          <Send className="h-3.5 w-3.5" />
          Ask Agent
        </button>
      </form>
    </div>
  );
};
