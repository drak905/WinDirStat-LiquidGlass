import React, { useEffect, useMemo, useRef, useState } from 'react';
import ReactMarkdown, { type Components, type UrlTransform } from 'react-markdown';
import remarkGfm from 'remark-gfm';
import { Check, Copy, ExternalLink } from 'lucide-react';

interface MarkdownRendererProps {
  content: string;
  compact?: boolean;
  className?: string;
}

const REMARK_PLUGINS = [remarkGfm];

function sanitizeMarkdownHref(url: string): string {
  const trimmed = url.trim();
  if (!trimmed) return '';
  try {
    const parsed = new URL(trimmed);
    if (parsed.protocol === 'http:' || parsed.protocol === 'https:' || parsed.protocol === 'mailto:') {
      return trimmed;
    }
    return '';
  } catch {
    return '';
  }
}

const safeUrlTransform: UrlTransform = (url) => sanitizeMarkdownHref(url);

export async function copyTextToClipboard(text: string): Promise<boolean> {
  if (typeof navigator !== 'undefined' && navigator.clipboard?.writeText) {
    try {
      await navigator.clipboard.writeText(text);
      return true;
    } catch {
      // Fall through to legacy textarea copy when Clipboard API is unavailable
    }
  }
  if (typeof document !== 'undefined') {
    try {
      const textarea = document.createElement('textarea');
      textarea.value = text;
      textarea.setAttribute('readonly', '');
      textarea.style.position = 'fixed';
      textarea.style.opacity = '0';
      document.body.appendChild(textarea);
      textarea.select();
      const copied = typeof document.execCommand === 'function' ? document.execCommand('copy') : false;
      document.body.removeChild(textarea);
      return copied;
    } catch {
      return false;
    }
  }
  return false;
}

function extractPlainText(node: React.ReactNode): string {
  if (node === null || node === undefined || typeof node === 'boolean') {
    return '';
  }
  if (typeof node === 'string' || typeof node === 'number') {
    return String(node);
  }
  if (Array.isArray(node)) {
    return node.map(extractPlainText).join('');
  }
  if (React.isValidElement<{ children?: React.ReactNode }>(node)) {
    return extractPlainText(node.props.children);
  }
  return '';
}

function getSafetyBadgeClass(rawText: string): string | null {
  const text = rawText.trim();
  if (!text || text.length > 32) return null;
  if (/^(safe|safe to (delete|remove|clean|quarantine)|low risk|yes\b)/i.test(text)) {
    return 'border-emerald-500/35 bg-emerald-500/18 text-emerald-800 dark:border-emerald-400/35 dark:bg-emerald-500/20 dark:text-emerald-200';
  }
  if (/^(clean via|official tool|use official)/i.test(text)) {
    return 'border-sky-500/35 bg-sky-500/18 text-sky-800 dark:border-sky-400/35 dark:bg-sky-500/20 dark:text-sky-200';
  }
  if (/^(caution|review|review carefully|medium risk|conditional)/i.test(text)) {
    return 'border-amber-500/40 bg-amber-500/18 text-amber-900 dark:border-amber-400/35 dark:bg-amber-500/20 dark:text-amber-200';
  }
  if (/^(keep|do not (delete|remove)|unsafe|high risk|critical|no\b)/i.test(text)) {
    return 'border-rose-500/35 bg-rose-500/18 text-rose-800 dark:border-rose-400/35 dark:bg-rose-500/20 dark:text-rose-200';
  }
  return null;
}

const MarkdownCodeBlock: React.FC<{ code: string; language: string }> = ({ code, language }) => {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => {
      if (timerRef.current) {
        clearTimeout(timerRef.current);
      }
    };
  }, []);

  const handleCopy = async () => {
    const ok = await copyTextToClipboard(code);
    if (ok) {
      setCopied(true);
      if (timerRef.current) clearTimeout(timerRef.current);
      timerRef.current = setTimeout(() => setCopied(false), 1800);
    }
  };

  return (
    <div className="my-2.5 overflow-hidden rounded-2xl border border-slate-900/15 bg-slate-900/92 text-slate-100 shadow-inner dark:border-white/15 dark:bg-black/65">
      <div className="flex items-center justify-between border-b border-white/10 bg-white/5 px-3 py-1.5">
        <span className="font-mono text-[10px] font-semibold uppercase tracking-wider text-sky-300">
          {language || 'code'}
        </span>
        <button
          type="button"
          aria-label="Copy code"
          onClick={() => void handleCopy()}
          className="inline-flex items-center gap-1 rounded-lg border border-white/15 bg-white/10 px-2 py-0.5 text-[10px] font-medium text-slate-200 transition hover:bg-white/20"
        >
          {copied ? (
            <>
              <Check className="h-3 w-3 text-emerald-400" />
              <span>Copied</span>
            </>
          ) : (
            <>
              <Copy className="h-3 w-3" />
              <span>Copy</span>
            </>
          )}
        </button>
      </div>
      <pre className="glass-scroll m-0 overflow-x-auto px-3.5 py-2.5 font-mono text-[11.5px] leading-5 text-slate-100">
        <code>{code}</code>
      </pre>
    </div>
  );
};

function createMarkdownComponents(compact: boolean): Components {
  return {
    h1: ({ children }) => (
      <h1
        className={`${
          compact ? 'text-sm' : 'text-base'
        } mt-4 mb-2 border-b border-slate-900/12 pb-1.5 font-bold tracking-tight text-slate-950 first:mt-0 dark:border-white/12 dark:text-white`}
      >
        {children}
      </h1>
    ),
    h2: ({ children }) => (
      <h2
        className={`${
          compact ? 'text-[13px]' : 'text-sm'
        } mt-4 mb-2 border-b border-slate-900/10 pb-1 font-semibold tracking-tight text-slate-950 first:mt-0 dark:border-white/10 dark:text-white`}
      >
        {children}
      </h2>
    ),
    h3: ({ children }) => (
      <h3 className="mt-3 mb-1.5 text-[13px] font-semibold text-sky-800 first:mt-0 dark:text-sky-300">
        {children}
      </h3>
    ),
    h4: ({ children }) => (
      <h4 className="mt-2.5 mb-1 text-xs font-semibold uppercase tracking-wide text-slate-700 first:mt-0 dark:text-slate-300">
        {children}
      </h4>
    ),
    p: ({ children }) => (
      <p className="my-2 break-words first:mt-0 last:mb-0">{children}</p>
    ),
    ul: ({ children }) => (
      <ul className="my-2 list-disc space-y-1 pl-5 marker:text-sky-500 first:mt-0 last:mb-0 dark:marker:text-sky-400">
        {children}
      </ul>
    ),
    ol: ({ children }) => (
      <ol className="my-2 list-decimal space-y-1 pl-5 marker:font-semibold marker:text-sky-600 first:mt-0 last:mb-0 dark:marker:text-sky-400">
        {children}
      </ol>
    ),
    li: ({ children }) => (
      <li className="break-words pl-0.5 leading-relaxed [&>ul]:mt-1 [&>ol]:mt-1">
        {children}
      </li>
    ),
    input: ({ type, checked }) => {
      if (type === 'checkbox') {
        return (
          <input
            type="checkbox"
            checked={Boolean(checked)}
            readOnly
            aria-label="Task status"
            className="mr-1.5 inline-block h-3.5 w-3.5 rounded border-slate-400 align-middle accent-sky-500"
          />
        );
      }
      return null;
    },
    blockquote: ({ children }) => (
      <blockquote className="my-2.5 rounded-r-2xl border-l-[3px] border-sky-500/80 bg-sky-500/10 px-3.5 py-2 text-slate-800 dark:border-sky-400/80 dark:bg-sky-400/10 dark:text-slate-200">
        {children}
      </blockquote>
    ),
    hr: () => <hr className="my-3 border-t border-slate-900/12 dark:border-white/12" />,
    strong: ({ children }) => (
      <strong className="font-semibold text-slate-950 dark:text-white">{children}</strong>
    ),
    em: ({ children }) => (
      <em className="italic text-slate-800 dark:text-slate-200">{children}</em>
    ),
    del: ({ children }) => (
      <del className="text-slate-500 line-through dark:text-slate-400">{children}</del>
    ),
    code: ({ children }) => (
      <code className="rounded-md border border-slate-900/10 bg-slate-900/8 px-1.5 py-0.5 font-mono text-[11.5px] font-medium text-sky-900 [overflow-wrap:anywhere] dark:border-white/12 dark:bg-white/10 dark:text-sky-200">
        {children}
      </code>
    ),
    pre: ({ children }) => {
      const childArray = React.Children.toArray(children);
      const firstChild = childArray[0];
      let language = 'code';
      let codeText = '';

      if (
        React.isValidElement<{ className?: string; children?: React.ReactNode }>(firstChild)
      ) {
        const match = /language-([A-Za-z0-9_+.-]+)/.exec(firstChild.props.className ?? '');
        if (match?.[1]) {
          language = match[1];
        }
        codeText = extractPlainText(firstChild.props.children);
      } else {
        codeText = extractPlainText(children);
      }

      return <MarkdownCodeBlock code={codeText.replace(/\n$/, '')} language={language} />;
    },
    table: ({ children }) => (
      <div className="glass-scroll my-3 overflow-x-auto rounded-2xl border border-slate-900/12 bg-white/35 shadow-sm dark:border-white/12 dark:bg-slate-950/45">
        <table className="w-full border-collapse text-left text-xs">{children}</table>
      </div>
    ),
    thead: ({ children }) => (
      <thead className="bg-slate-900/8 text-[11px] font-semibold uppercase tracking-wider text-slate-700 dark:bg-white/10 dark:text-slate-200">
        {children}
      </thead>
    ),
    tbody: ({ children }) => (
      <tbody className="divide-y divide-slate-900/8 dark:divide-white/8">{children}</tbody>
    ),
    tr: ({ children }) => (
      <tr className="even:bg-slate-900/[0.03] transition-colors hover:bg-sky-500/8 dark:even:bg-white/[0.025] dark:hover:bg-sky-400/10">
        {children}
      </tr>
    ),
    th: ({ children }) => (
      <th className="whitespace-nowrap px-3 py-2 text-left font-semibold">{children}</th>
    ),
    td: ({ children }) => {
      const cellText = extractPlainText(children);
      const badgeClass = getSafetyBadgeClass(cellText);
      return (
        <td className="px-3 py-2 align-top break-words">
          {badgeClass ? (
            <span
              className={`inline-flex items-center whitespace-nowrap rounded-full border px-2 py-0.5 text-[11px] font-semibold [&_strong]:text-current ${badgeClass}`}
            >
              {children}
            </span>
          ) : (
            children
          )}
        </td>
      );
    },
    a: ({ href, children }) => {
      const safeHref = typeof href === 'string' ? sanitizeMarkdownHref(href) : '';
      if (!safeHref) {
        return (
          <span className="font-medium text-slate-700 underline decoration-dotted dark:text-slate-300">
            {children}
          </span>
        );
      }
      return (
        <a
          href={safeHref}
          target="_blank"
          rel="noopener noreferrer"
          className="inline break-words font-medium text-sky-700 underline decoration-sky-500/45 underline-offset-2 transition [overflow-wrap:anywhere] hover:text-sky-600 hover:decoration-sky-400 dark:text-sky-300 dark:hover:text-sky-200"
        >
          <span>{children}</span>
          <ExternalLink
            aria-hidden="true"
            className="ml-0.5 inline h-3 w-3 align-text-top opacity-75"
          />
        </a>
      );
    },
    img: ({ src, alt }) => {
      const safeSrc = typeof src === 'string' ? sanitizeMarkdownHref(src) : '';
      const label = alt?.trim() || safeSrc || 'image';
      if (safeSrc) {
        return (
          <a
            href={safeSrc}
            target="_blank"
            rel="noopener noreferrer"
            className="inline-flex items-center gap-1 rounded-lg border border-sky-400/30 bg-sky-500/10 px-2 py-0.5 text-[11px] font-medium text-sky-800 hover:bg-sky-500/20 dark:text-sky-200"
          >
            <span>[Image: {label}]</span>
            <ExternalLink aria-hidden="true" className="h-3 w-3 shrink-0" />
          </a>
        );
      }
      return (
        <span className="inline-flex items-center rounded-lg border border-white/20 bg-white/10 px-2 py-0.5 text-[11px] text-slate-600 dark:text-slate-300">
          [Image: {label}]
        </span>
      );
    },
  };
}

export const MarkdownRenderer: React.FC<MarkdownRendererProps> = ({
  content,
  compact = false,
  className = '',
}) => {
  const components = useMemo(() => createMarkdownComponents(compact), [compact]);

  return (
    <div
      className={`markdown-body select-text ${
        compact ? 'text-xs leading-5' : 'text-[13px] leading-6'
      } text-slate-800 dark:text-slate-100 ${className}`.trim()}
    >
      <ReactMarkdown
        remarkPlugins={REMARK_PLUGINS}
        urlTransform={safeUrlTransform}
        components={components}
      >
        {content}
      </ReactMarkdown>
    </div>
  );
};
