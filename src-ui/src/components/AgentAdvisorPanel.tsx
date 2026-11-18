import React, { useState } from 'react';
import {
  Bot,
  Send,
  ShieldAlert,
  ExternalLink,
  CheckCircle2,
  Wifi,
  AlertTriangle,
  Sparkles,
} from 'lucide-react';
import type {
  AgentAdvisorResponse,
  AgentProvider,
  AgentProviderStatus,
  AgentVerdict,
} from '../types/storage';
import { AGENT_PROVIDERS } from '../services/tauriBridge';
import { formatBytes } from '../utils/formatters';

interface AgentAdvisorPanelProps {
  providers: AgentProviderStatus[];
  selectedProvider: AgentProvider;
  onProviderChange: (provider: AgentProvider) => void;
  responses: AgentAdvisorResponse[];
  isStreaming: boolean;
  onSendPrompt: (params: {
    provider: AgentProvider;
    prompt: string;
    customCommand?: string;
  }) => Promise<unknown>;
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
  responses,
  isStreaming,
  onSendPrompt,
  onQuarantineRecommended,
}) => {
  const [promptInput, setPromptInput] = useState('');
  const [customCmd, setCustomCmd] = useState('my-local-agent --json');
  const [quarantinedIds, setQuarantinedIds] = useState<Record<string, boolean>>({});

  const activeProviderStatus = providers.find((p) => p.provider === selectedProvider);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = promptInput.trim();
    if (!trimmed || isStreaming) return;
    setPromptInput('');
    await onSendPrompt({
      provider: selectedProvider,
      prompt: trimmed,
      customCommand: selectedProvider === 'custom' ? customCmd : undefined,
    });
  };

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      {/* Header & Provider Selector */}
      <div className="liquid-glass-card flex flex-wrap items-center justify-between gap-3 rounded-2xl px-4 py-3">
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
          <div className="flex items-center gap-1.5 rounded-full border border-emerald-400/30 bg-emerald-500/15 px-2.5 py-1 text-[11px] font-medium text-emerald-700 dark:text-emerald-300">
            <Wifi className="h-3 w-3" />
            <span>
              {activeProviderStatus?.available ? 'Connected' : 'Ready (Adapter)'}
              {activeProviderStatus?.latencyMs ? ` • ${activeProviderStatus.latencyMs}ms` : ''}
            </span>
          </div>

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

      {/* Streaming Response Cards Feed */}
      <div className="glass-scroll min-h-0 flex-1 space-y-3 overflow-y-auto pr-1">
        {isStreaming && (
          <div className="liquid-glass-card liquid-spring-enter flex items-center gap-3 rounded-3xl p-4 text-xs">
            <Sparkles className="h-4 w-4 animate-pulse text-sky-400" />
            <span className="font-medium text-slate-800 dark:text-slate-200">
              Streaming verdict from <strong className="font-mono">{selectedProvider}</strong> CLI &amp; cross-checking external sources…
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
                Every verdict cross-checks at least 3 authoritative documentation URLs.
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
                  onClick={() => void onSendPrompt({ provider: selectedProvider, prompt: suggestion })}
                  className="rounded-xl border border-white/20 bg-white/15 px-3 py-1.5 text-[11px] font-medium text-slate-800 hover:bg-white/25 dark:text-slate-200"
                >
                  {suggestion}
                </button>
              ))}
            </div>
          </div>
        ) : (
          responses.map((resp) => {
            const isQuarantined = Boolean(quarantinedIds[resp.id]);
            return (
              <article
                key={resp.id}
                className="liquid-glass-card liquid-spring-enter rounded-3xl p-4"
              >
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
                  <p className="leading-relaxed text-slate-700 dark:text-slate-300">
                    {resp.usage_value_assessment}
                  </p>
                  <p className="mt-1 text-[11px] text-slate-600 dark:text-slate-400">
                    {resp.rationale}
                  </p>
                </div>

                {/* Verified Cross-Checked External Source URLs (>= 3) */}
                <div className="mt-3">
                  <div className="mb-1.5 text-[11px] font-semibold uppercase tracking-wider text-slate-600 dark:text-slate-400">
                    Cross-Checked External Sources ({resp.sources.length})
                  </div>
                  <div
                    data-testid="agent-sources-list"
                    className="grid grid-cols-1 gap-2 md:grid-cols-3"
                  >
                    {resp.sources.map((src) => (
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
          placeholder={`Ask ${selectedProvider} CLI whether a path is safe to delete or keep…`}
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
