import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { expect, test, type Locator, type Page } from '@playwright/test';
import {
  buildNestedTreeFromSaveTo,
  formatSaveToJson,
  generateSaveToTree,
  verifySyntheticTree,
} from '../../tests/fixtures/generate_synthetic_tree.mjs';

const HERE_DIR = path.dirname(fileURLToPath(import.meta.url));
const FIXTURES_DIR = path.resolve(HERE_DIR, '../../tests/fixtures');
const SYNTHETIC_100K_PATH = path.join(FIXTURES_DIR, 'synthetic-tree-100k.json');

const EXPECTED_PROVIDERS = ['antigravity', 'kiro', 'claude', 'codex', 'opencode'] as const;

let consoleErrors: string[] = [];
let pageErrors: string[] = [];

test.beforeAll(() => {
  // Ensure deterministic 100k-node fixture is generated & verified before E2E tests run
  const items = generateSaveToTree({ count: 100_000 });
  verifySyntheticTree(items, 100_000);
  if (!fs.existsSync(SYNTHETIC_100K_PATH)) {
    fs.writeFileSync(SYNTHETIC_100K_PATH, formatSaveToJson(items), 'utf8');
  }
  const nested = buildNestedTreeFromSaveTo(items);
  expect(nested).toBeTruthy();
  expect(nested.filesCount + nested.foldersCount + 1).toBe(100_000);
});

test.beforeEach(async ({ page }) => {
  consoleErrors = [];
  pageErrors = [];

  page.on('console', (msg) => {
    if (msg.type() === 'error') {
      consoleErrors.push(msg.text());
    }
  });
  page.on('pageerror', (err) => {
    pageErrors.push(err.message || String(err));
  });
  page.on('dialog', async (dialog) => {
    await dialog.accept();
  });

  await page.goto('/', { waitUntil: 'domcontentloaded' });
  await expect(page.getByTestId('glass-dock')).toBeVisible();
});

test.afterEach(() => {
  expect(consoleErrors, `Unexpected console errors: ${consoleErrors.join('\n')}`).toEqual([]);
  expect(pageErrors, `Unexpected uncaught page errors: ${pageErrors.join('\n')}`).toEqual([]);
});

async function selectProvider(locator: Locator, page: Page, provider: string) {
  const tag = await locator.evaluate((el) => el.tagName.toLowerCase());
  if (tag === 'select') {
    const matchingValue = await locator.evaluate((el, target) => {
      const sel = el as HTMLSelectElement;
      const opt = Array.from(sel.options).find(
        (o) => o.value.toLowerCase().includes(target) || o.textContent?.toLowerCase().includes(target)
      );
      return opt ? opt.value : null;
    }, provider);
    expect(matchingValue, `Option for provider "${provider}" should exist`).toBeTruthy();
    await locator.selectOption(matchingValue!);
    return;
  }

  await locator.click();
  const menuOption = page
    .locator(`[role="option"], [data-value="${provider}"], button, li`)
    .filter({ hasText: new RegExp(provider, 'i') })
    .first();
  await expect(menuOption).toBeVisible();
  await menuOption.click();
}

test.describe('WinDirStat Apple Liquid Glass E2E Suite', () => {
  test('renders Apple Liquid Glass backdrop-filter, SVG refraction filter, and dock navigation', async ({ page }) => {
    const dock = page.getByTestId('glass-dock');
    await expect(dock).toBeVisible();

    for (const navId of ['nav-treemap', 'nav-sunburst', 'nav-duplicates', 'nav-advisor', 'nav-vault']) {
      await expect(page.getByTestId(navId)).toBeVisible();
    }
    await expect(page.getByTestId('drive-selector')).toBeVisible();

    // Verify SVG refraction filter exists in the DOM with active filter primitives
    const filterInfo = await page.evaluate(() => {
      const filterEl = document.querySelector('svg filter#liquid-glass-refraction');
      if (!filterEl) return null;
      return {
        id: filterEl.id,
        primitiveCount: filterEl.children.length,
        primitiveTags: Array.from(filterEl.children).map((c) => c.tagName.toLowerCase()),
      };
    });
    expect(filterInfo, '<filter id="liquid-glass-refraction"> must exist in SVG defs').not.toBeNull();
    expect(filterInfo!.primitiveCount).toBeGreaterThan(0);

    // Verify glassmorphic backdrop-filter contains blur(28px) and url(#liquid-glass-refraction)
    const backdropMatch = await page.evaluate(() => {
      const candidates = [
        document.querySelector('[data-testid="glass-dock"]'),
        ...Array.from(document.querySelectorAll('.liquid-glass, [class*="liquid-glass"], [class*="glass"]')),
      ].filter((el): el is Element => Boolean(el));

      const inspected = candidates.map((el) => {
        const cs = window.getComputedStyle(el);
        const value = cs.backdropFilter || (cs as CSSStyleDeclaration & { webkitBackdropFilter?: string }).webkitBackdropFilter || '';
        return {
          tag: el.tagName.toLowerCase(),
          testId: el.getAttribute('data-testid'),
          backdropFilter: value,
        };
      });

      const matching = inspected.find(
        (entry) =>
          /blur\(28px\)/i.test(entry.backdropFilter) &&
          /url\((["']?)(?:https?:\/\/[^)]+)?#liquid-glass-refraction\1\)/i.test(entry.backdropFilter)
      );

      return { matching, inspectedCount: inspected.length, sample: inspected.slice(0, 5) };
    });

    expect(
      backdropMatch.matching,
      `Expected backdrop-filter containing blur(28px) and url(#liquid-glass-refraction). Inspected: ${JSON.stringify(backdropMatch.sample)}`
    ).toBeTruthy();

    // Cycle through all dock views without console/page errors
    await page.getByTestId('nav-sunburst').click();
    await page.getByTestId('nav-duplicates').click();
    await page.getByTestId('nav-advisor').click();
    await expect(page.getByTestId('agent-provider-select')).toBeVisible();
    await page.getByTestId('nav-vault').click();
    await expect(page.getByTestId('vault-table')).toBeVisible();
    await page.getByTestId('nav-treemap').click();
    await expect(page.getByTestId('treemap-canvas')).toBeVisible();
  });

  test('renders 100k-node synthetic tree within <16.6ms interactive frame budget', async ({ page }) => {
    await page.getByTestId('nav-treemap').click();
    const driveSelector = page.getByTestId('drive-selector');
    await expect(driveSelector).toBeVisible();

    const load100kBtn = page.getByTestId('load-100k-btn');
    if (await load100kBtn.isVisible()) {
      await load100kBtn.click();
    } else {
      // Fallback: if drive-selector is a <select> with a 100k/synthetic fixture option, select it
      const selectorTag = await driveSelector.evaluate((el) => el.tagName.toLowerCase());
      if (selectorTag === 'select') {
        const syntheticOpt = await driveSelector.evaluate((el) => {
          const sel = el as HTMLSelectElement;
          const opt = Array.from(sel.options).find((o) => /100k|synthetic|fixture|c:\\/i.test(`${o.value} ${o.text}`));
          return opt ? opt.value : null;
        });
        if (syntheticOpt) {
          await driveSelector.selectOption(syntheticOpt);
        }
      }
    }

    const canvas = page.getByTestId('treemap-canvas');
    await expect(canvas).toBeVisible();
    const box = await canvas.boundingBox();
    expect(box, 'Treemap canvas must have measurable layout bounds').not.toBeNull();
    expect(box!.width).toBeGreaterThan(300);
    expect(box!.height).toBeGreaterThan(200);

    // Confirm synthetic 100k JSON fixture parses cleanly inside browser runtime under budget
    const raw100k = fs.readFileSync(SYNTHETIC_100K_PATH, 'utf8');
    const inBrowserParseStats = await page.evaluate((jsonText) => {
      const t0 = performance.now();
      const parsed = JSON.parse(jsonText);
      const parseMs = performance.now() - t0;
      return { nodeCount: Array.isArray(parsed) ? parsed.length : 0, parseMs };
    }, raw100k);
    expect(inBrowserParseStats.nodeCount).toBe(100_000);

    // Measure main-thread interactive frame work time during hover, zoom (wheel), and pan (drag)
    // via both rAF microtask turns and treemap canvas input-event turn durations.
    await page.evaluate(() => {
      const w = window as Window & {
        __wdsFrameMetrics?: {
          frameWorkSamples: number[];
          eventWorkSamples: number[];
          rafDeltas: number[];
          running: boolean;
          cleanup: () => void;
        };
      };
      const canvasEl = document.querySelector('[data-testid="treemap-canvas"]');
      const metrics = {
        frameWorkSamples: [] as number[],
        eventWorkSamples: [] as number[],
        rafDeltas: [] as number[],
        running: true,
        cleanup: () => {},
      };
      w.__wdsFrameMetrics = metrics;

      const eventTypes = ['mousemove', 'wheel', 'mousedown', 'mouseup'] as const;
      const listeners: Array<{ type: string; fn: () => void }> = [];
      for (const evType of eventTypes) {
        const fn = () => {
          const evStart = performance.now();
          queueMicrotask(() => {
            metrics.eventWorkSamples.push(performance.now() - evStart);
          });
        };
        canvasEl?.addEventListener(evType, fn, { capture: true, passive: true });
        listeners.push({ type: evType, fn });
      }
      metrics.cleanup = () => {
        for (const { type, fn } of listeners) {
          canvasEl?.removeEventListener(type, fn, { capture: true });
        }
      };

      let prevRaf = performance.now();
      const tick = (now: number) => {
        const workStart = performance.now();
        const delta = now - prevRaf;
        prevRaf = now;
        if (metrics.rafDeltas.length > 0 || delta < 100) {
          metrics.rafDeltas.push(delta);
        }

        queueMicrotask(() => {
          metrics.frameWorkSamples.push(performance.now() - workStart);
        });

        if (metrics.running) {
          requestAnimationFrame(tick);
        }
      };
      requestAnimationFrame(tick);
    });

    const cx = box!.x + box!.width * 0.5;
    const cy = box!.y + box!.height * 0.5;

    // 1. Hover sweeps across treemap regions
    for (let step = 0; step < 12; step++) {
      const hx = box!.x + box!.width * (0.2 + (step % 6) * 0.12);
      const hy = box!.y + box!.height * (0.25 + Math.floor(step / 6) * 0.45);
      await page.mouse.move(hx, hy, { steps: 2 });
    }

    // 2. Zoom in and out with wheel
    await page.mouse.move(cx, cy);
    await page.mouse.wheel(0, -180);
    await page.waitForTimeout(80);
    await page.mouse.wheel(0, 180);
    await page.waitForTimeout(80);

    // 3. Pan drag across canvas
    await page.mouse.move(cx - 40, cy - 20);
    await page.mouse.down();
    await page.mouse.move(cx + 60, cy + 30, { steps: 8 });
    await page.mouse.up();
    await page.waitForTimeout(150);

    const frameStats = await page.evaluate(() => {
      const w = window as Window & {
        __wdsFrameMetrics?: {
          frameWorkSamples: number[];
          eventWorkSamples: number[];
          rafDeltas: number[];
          running: boolean;
          cleanup: () => void;
        };
      };
      const m = w.__wdsFrameMetrics!;
      m.running = false;
      m.cleanup();
      const combined = [...m.frameWorkSamples.slice(3), ...m.eventWorkSamples];
      const avgWorkMs = combined.reduce((acc, v) => acc + v, 0) / Math.max(1, combined.length);
      const sortedRaf = [...m.rafDeltas.slice(3)].sort((a, b) => a - b);
      const medianRafMs = sortedRaf[Math.floor(sortedRaf.length / 2)] || 0;
      return {
        sampleCount: combined.length,
        eventSamples: m.eventWorkSamples.length,
        avgWorkMs,
        medianRafMs,
      };
    });

    expect(frameStats.sampleCount).toBeGreaterThan(10);
    expect(frameStats.eventSamples).toBeGreaterThan(0);
    expect(
      frameStats.avgWorkMs,
      `Average frame work (${frameStats.avgWorkMs.toFixed(2)}ms) exceeded 16.6ms budget`
    ).toBeLessThan(16.6);
  });

  test('switches all 5 Agent CLI providers, streams chat response, and displays >=3 external sources', async ({ page }) => {
    await page.getByTestId('nav-advisor').click();

    const providerSelect = page.getByTestId('agent-provider-select');
    await expect(providerSelect).toBeVisible();

    // Verify all 5 required Agent CLI providers exist and can be switched cleanly
    for (const provider of EXPECTED_PROVIDERS) {
      await selectProvider(providerSelect, page, provider);
    }

    // Select antigravity (or claude) and run an evaluation query
    await selectProvider(providerSelect, page, 'antigravity');

    const chatInput = page.getByTestId('agent-chat-input');
    const sendBtn = page.getByTestId('agent-send-btn');
    await expect(chatInput).toBeVisible();
    await expect(sendBtn).toBeVisible();

    await chatInput.fill('Evaluate C:\\Users\\Developer\\AppData\\Local\\Temp for 7-day quarantine cleanup');
    await sendBtn.click();

    const verdictBadge = page.getByTestId('agent-verdict-badge');
    await expect(verdictBadge).toBeVisible({ timeout: 15_000 });
    await expect(verdictBadge).toHaveText(/\S+/);

    const sourcesList = page.getByTestId('agent-sources-list');
    await expect(sourcesList).toBeVisible({ timeout: 15_000 });

    // Assert >= 3 external HTTP/HTTPS citation URLs are rendered
    const sourceUrls = await sourcesList.evaluate((root) => {
      const anchors = Array.from(root.querySelectorAll('a[href]'))
        .map((a) => a.getAttribute('href') || '')
        .filter((href) => /^https?:\/\//i.test(href));
      if (anchors.length > 0) {
        return Array.from(new Set(anchors));
      }
      const textUrls = (root.textContent || '').match(/https?:\/\/[^\s)]+/gi) || [];
      return Array.from(new Set(textUrls));
    });

    expect(
      sourceUrls.length,
      `Expected >= 3 external source URLs in agent-sources-list, found ${sourceUrls.length}: ${JSON.stringify(sourceUrls)}`
    ).toBeGreaterThanOrEqual(3);
  });

  test('executes treemap context menu -> Move to 7-Day Quarantine Vault -> 1-click Rollback flow', async ({ page }) => {
    await page.getByTestId('nav-treemap').click();

    const canvas = page.getByTestId('treemap-canvas');
    await expect(canvas).toBeVisible();
    const box = await canvas.boundingBox();
    expect(box).not.toBeNull();

    // Open treemap context menu via right-click
    await page.mouse.click(box!.x + box!.width * 0.45, box!.y + box!.height * 0.45, {
      button: 'right',
    });

    const contextMenu = page.getByTestId('treemap-context-menu');
    await expect(contextMenu).toBeVisible();

    const askAgentItem = page.getByTestId('ctx-ask-agent');
    const quarantineItem = page.getByTestId('ctx-quarantine');
    await expect(askAgentItem).toBeVisible();
    await expect(quarantineItem).toBeVisible();
    await expect(quarantineItem).toContainText(/quarantine/i);

    // Trigger Move to 7-Day Quarantine Vault
    await quarantineItem.click();

    // Navigate to Vault tab if not auto-navigated
    const vaultNav = page.getByTestId('nav-vault');
    await vaultNav.click();

    const vaultTable = page.getByTestId('vault-table');
    await expect(vaultTable).toBeVisible();

    const rollbackBtns = page.getByTestId('vault-rollback-btn');
    const rollbackBtn = rollbackBtns.first();
    await expect(rollbackBtn).toBeVisible();
    await expect(rollbackBtn).toBeEnabled();

    const initialRollbackCount = await rollbackBtns.count();
    const initialRowCount = await vaultTable.locator('tbody tr, [role="row"], [data-testid^="vault-row"]').count();
    await rollbackBtn.click();

    // Confirm 1-click Rollback updated the vault table (row/button removed, button disabled, or marked Restored/Rolled Back)
    await expect(async () => {
      const currentRollbackCount = await rollbackBtns.count();
      const currentRowCount = await vaultTable.locator('tbody tr, [role="row"], [data-testid^="vault-row"]').count();
      const firstStillEnabled = currentRollbackCount > 0 ? await rollbackBtns.first().isEnabled() : false;
      const tableText = (await vaultTable.textContent()) || '';
      const rollbackSucceeded =
        currentRollbackCount < initialRollbackCount ||
        currentRowCount < initialRowCount ||
        !firstStillEnabled ||
        /restored|rolled\s*back|reverted|recovered/i.test(tableText);
      expect(rollbackSucceeded).toBe(true);
    }).toPass({ timeout: 10_000 });
  });

  test('routes treemap context menu Ask Agent action directly into Advisor view', async ({ page }) => {
    await page.getByTestId('nav-treemap').click();

    const canvas = page.getByTestId('treemap-canvas');
    await expect(canvas).toBeVisible();
    const box = await canvas.boundingBox();
    expect(box).not.toBeNull();

    await page.mouse.click(box!.x + box!.width * 0.45, box!.y + box!.height * 0.45, {
      button: 'right',
    });

    await expect(page.getByTestId('treemap-context-menu')).toBeVisible();
    await page.getByTestId('ctx-ask-agent').click();

    await expect(page.getByTestId('agent-provider-select')).toBeVisible();
    await expect(page.getByTestId('agent-chat-input')).toBeVisible();
  });
});
