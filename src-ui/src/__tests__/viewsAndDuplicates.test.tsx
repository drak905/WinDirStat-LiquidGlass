import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { SunburstView } from '../components/SunburstView';
import { FlamegraphView } from '../components/FlamegraphView';
import { DuplicatesView } from '../components/DuplicatesView';
import { createDemoSnapshot, createDemoDuplicates } from '../data/demoSnapshot';

describe('Sunburst, Flamegraph, and Duplicate File Inspector Views', () => {
  it('renders interactive Sunburst arcs and triggers context menu on right-click', () => {
    const snapshot = createDemoSnapshot();
    const onContextMenu = vi.fn();

    render(
      <SunburstView
        root={snapshot.root}
        onContextMenuNode={onContextMenu}
      />
    );

    const svg = screen.getByTestId('sunburst-svg');
    expect(svg).toBeInTheDocument();
    const arcs = svg.querySelectorAll('path[data-node-id]');
    expect(arcs.length).toBeGreaterThan(3);

    fireEvent.contextMenu(arcs[0], { clientX: 120, clientY: 140 });
    expect(onContextMenu).toHaveBeenCalledTimes(1);
  });

  it('renders interactive Flamegraph frames with search filtering', async () => {
    const snapshot = createDemoSnapshot();
    const onContextMenu = vi.fn();
    const user = userEvent.setup();

    render(
      <FlamegraphView
        root={snapshot.root}
        onContextMenuNode={onContextMenu}
      />
    );

    expect(screen.getByTestId('flamegraph-view')).toBeInTheDocument();
    const filterInput = screen.getByTestId('flamegraph-filter');
    await user.type(filterInput, 'node_modules');
    expect(screen.getAllByText(/node_modules/i).length).toBeGreaterThan(0);
  });

  it('renders Duplicate File Inspector and allows 1-click quarantine of redundant copies', async () => {
    const groups = createDemoDuplicates();
    const onQuarantine = vi.fn(async () => {});
    const onAskAgent = vi.fn();
    const user = userEvent.setup();

    render(
      <DuplicatesView
        groups={groups}
        onQuarantineFile={onQuarantine}
        onAskAgent={onAskAgent}
      />
    );

    expect(screen.getByTestId('duplicates-view')).toBeInTheDocument();
    const quarantineBtns = screen.getAllByTestId('dup-quarantine-btn');
    expect(quarantineBtns.length).toBeGreaterThan(0);
    await user.click(quarantineBtns[0]);
    expect(onQuarantine).toHaveBeenCalledTimes(1);
  });
});
