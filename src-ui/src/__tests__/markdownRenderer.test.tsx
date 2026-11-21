import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { MarkdownRenderer } from '../components/MarkdownRenderer';

describe('MarkdownRenderer', () => {
  afterEach(() => {
    vi.restoreAllMocks();
  });

  it('renders headings, emphasis, strikethrough, inline Windows path code, lists, blockquote, and hr without raw markers', () => {
    const sample = [
      '# Disk Analysis',
      '## Key Findings',
      '### Cache Locations',
      '',
      'Found **safe** items, *optional* caches, and ~~stale~~ entries at `C:\\Windows\\Temp`.',
      '',
      '- First cache bullet',
      '- Second cache bullet',
      '',
      '1. Review list first',
      '2. Quarantine safely',
      '',
      '> Verify backups before purging system caches.',
      '',
      '---',
    ].join('\n');

    const { container } = render(<MarkdownRenderer content={sample} />);

    expect(screen.getByRole('heading', { level: 1, name: 'Disk Analysis' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { level: 2, name: 'Key Findings' })).toBeInTheDocument();
    expect(screen.getByRole('heading', { level: 3, name: 'Cache Locations' })).toBeInTheDocument();

    expect(container.querySelector('strong')).toHaveTextContent('safe');
    expect(container.querySelector('em')).toHaveTextContent('optional');
    expect(container.querySelector('del')).toHaveTextContent('stale');

    const inlineCode = container.querySelector('code');
    expect(inlineCode).not.toBeNull();
    expect(inlineCode).toHaveTextContent('C:\\Windows\\Temp');

    expect(container.querySelectorAll('ul li')).toHaveLength(2);
    expect(container.querySelectorAll('ol li')).toHaveLength(2);
    expect(container.querySelector('blockquote')).toHaveTextContent(
      'Verify backups before purging system caches.'
    );
    expect(container.querySelector('hr')).not.toBeNull();

    const text = container.textContent ?? '';
    expect(text).not.toContain('## ');
    expect(text).not.toContain('**safe**');
  });

  it('renders GFM tables with safety status pill styling while keeping cell text intact', () => {
    const markdownTable = [
      '| Folder Path | Size | Safety | Notes |',
      '| --- | --- | --- | --- |',
      '| `C:\\Users\\thinh\\AppData\\Local\\Temp` | 4.2 GB | Safe to delete | Recreated by apps |',
      '| `C:\\Users\\thinh\\Downloads` | 18.0 GB | Caution | Review personal installers |',
      '| `C:\\Windows\\WinSxS` | 9.5 GB | Do not delete | Windows Component Store |',
    ].join('\n');

    const { container } = render(<MarkdownRenderer content={markdownTable} />);

    const table = container.querySelector('table');
    expect(table).not.toBeNull();
    expect(table?.querySelectorAll('thead th')).toHaveLength(4);
    expect(table?.querySelectorAll('tbody tr')).toHaveLength(3);

    const safeCell = screen.getByText('Safe to delete');
    const cautionCell = screen.getByText('Caution');
    const keepCell = screen.getByText('Do not delete');

    expect(safeCell.className).toContain('bg-emerald-500');
    expect(cautionCell.className).toContain('bg-amber-500');
    expect(keepCell.className).toContain('bg-rose-500');
  });

  it('renders fenced code block language label and copies exact snippet via Copy button', async () => {
    const user = userEvent.setup();
    const writeText = vi.fn(async () => {});
    Object.defineProperty(navigator, 'clipboard', {
      value: { writeText },
      configurable: true,
    });
    const snippet = '```powershell\npip cache purge\ncargo cache -a\n```';

    render(<MarkdownRenderer content={snippet} />);

    expect(screen.getByText('powershell')).toBeInTheDocument();
    const copyBtn = screen.getByRole('button', { name: 'Copy code' });
    await user.click(copyBtn);

    expect(writeText).toHaveBeenCalledTimes(1);
    expect(writeText).toHaveBeenCalledWith('pip cache purge\ncargo cache -a');
    expect(screen.getByText('Copied')).toBeInTheDocument();
  });

  it('allows safe external https links and blocks unsafe javascript links', () => {
    const markdown = [
      'Read [Microsoft Docs](https://learn.microsoft.com/en-us/windows/storage) and [Ignore XSS](javascript:alert(1)).',
    ].join('\n');

    const { container } = render(<MarkdownRenderer content={markdown} />);

    const safeLink = screen.getByRole('link', { name: /Microsoft Docs/i });
    expect(safeLink).toHaveAttribute('href', 'https://learn.microsoft.com/en-us/windows/storage');
    expect(safeLink).toHaveAttribute('target', '_blank');
    expect(safeLink).toHaveAttribute('rel', 'noopener noreferrer');

    expect(screen.getByText('Ignore XSS')).toBeInTheDocument();
    expect(container.querySelector('a[href^="javascript:"]')).toBeNull();
  });

  it('renders real kiro-cli cleanup tables with bold category pills and wraps long bare URLs cleanly', () => {
    const kiroSample = [
      "I'll verify the public sources for the top cleanup targets first.",
      '## C: cleanup advice (13.52 GiB free of 190.63 GiB, 92.9% used)',
      '',
      'Only sizes from the scan are used. Folders marked "+" are lower bounds.',
      '',
      '| Priority | Category | Target | Size | Action |',
      '|---|---|---|---|---|',
      '| 1 | **Safe to Quarantine** | `C:\\Users\\thinh\\Downloads` | 857.68 MiB (471 files) | Review installers and ISOs. |',
      '| 3 | **Safe to Quarantine** | `C:\\hãy xem qua trang web này và tôi muốn tạo ssh priv.pdf` | 643 KB | Open it first. |',
      '| 4 | **Clean via Official Tool** | `C:\\Users\\thinh\\AppData\\Local\\pip\\Cache` | 336.26 MiB+ | Run `py -m pip cache purge`. |',
      '| 11 | **Keep Critical** | `C:\\EFI`, `C:\\$Windows.~WS` | 33.40 MiB, 751 KB | Leave them alone. |',
      '',
      '## Verified sources',
      '- Microsoft page file guidance: https://learn.microsoft.com/en-us/troubleshoot/windows-client/performance/how-to-determine-the-appropriate-page-file-size-for-64-bit-versions-of-windows',
    ].join('\n');

    render(<MarkdownRenderer content={kiroSample} />);

    expect(
      screen.getByRole('heading', {
        level: 2,
        name: 'C: cleanup advice (13.52 GiB free of 190.63 GiB, 92.9% used)',
      })
    ).toBeInTheDocument();

    const safePill = screen.getAllByText('Safe to Quarantine')[0].closest('span');
    const cleanPill = screen.getByText('Clean via Official Tool').closest('span');
    const keepPill = screen.getByText('Keep Critical').closest('span');

    expect(safePill?.className).toContain('bg-emerald-500');
    expect(safePill?.className).toContain('whitespace-nowrap');
    expect(safePill?.className).toContain('[&_strong]:text-current');
    expect(cleanPill?.className).toContain('bg-sky-500');
    expect(cleanPill?.className).toContain('whitespace-nowrap');
    expect(keepPill?.className).toContain('bg-rose-500');
    expect(keepPill?.className).toContain('whitespace-nowrap');

    expect(screen.getByText('C:\\$Windows.~WS').tagName).toBe('CODE');
    expect(
      screen.getByText('C:\\hãy xem qua trang web này và tôi muốn tạo ssh priv.pdf').tagName
    ).toBe('CODE');

    const bareLink = screen.getByRole('link', {
      name: /learn\.microsoft\.com\/en-us\/troubleshoot\/windows-client\/performance\/how-to-determine-the-appropriate-page-file-size-for-64-bit-versions-of-windows/,
    });
    expect(bareLink).toHaveAttribute('target', '_blank');
    expect(bareLink).toHaveAttribute('rel', 'noopener noreferrer');
    expect(bareLink.className).toContain('[overflow-wrap:anywhere]');
    expect(bareLink.className).not.toContain('inline-flex');
  });
});
