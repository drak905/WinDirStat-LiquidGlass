import { render, screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';
import { VaultManagerPanel } from '../components/VaultManagerPanel';
import type { VaultItem } from '../types/storage';

describe('7-Day Quarantine Vault Manager', () => {
  it('displays quarantined items with 168h TTL countdown, SHA-256/xxHash64 verification, and 1-click rollback', async () => {
    const items: VaultItem[] = [
      {
        id: 'vault-1',
        batchId: 'batch-101',
        originalPath: 'C:\\Users\\thinh\\Downloads\\old-backup.zip',
        fileName: 'old-backup.zip',
        sizeBytes: 3_221_225_472,
        quarantinedAt: '2026-11-18T10:00:00Z',
        expiresAt: '2026-11-25T10:00:00Z',
        remainingHours: 164.5,
        sha256: '4a7d1ed414474e4033ac29ccb8653d9b4a7d1ed414474e4033ac29ccb8653d9b',
        xxhash64: 'c20ad4d76fe97759',
        hashVerified: true,
        status: 'quarantined',
      },
    ];

    const onRollback = vi.fn(async () => {});
    const onPurge = vi.fn(async () => {});
    const user = userEvent.setup();

    render(
      <VaultManagerPanel
        items={items}
        onRollback={onRollback}
        onPurge={onPurge}
        onRefresh={() => {}}
      />
    );

    const table = screen.getByTestId('vault-table');
    expect(table).toBeInTheDocument();
    expect(table).toHaveTextContent('old-backup.zip');
    expect(table).toHaveTextContent('164.5h / 168h');
    expect(table).toHaveTextContent('SHA-256');
    expect(table).toHaveTextContent('xxHash64');

    const rollbackBtn = screen.getByTestId('vault-rollback-btn');
    await user.click(rollbackBtn);

    await waitFor(() => expect(onRollback).toHaveBeenCalledWith('vault-1'));
  });
});
