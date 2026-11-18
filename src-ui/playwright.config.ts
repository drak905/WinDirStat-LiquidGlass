import fs from 'node:fs';
import path from 'node:path';
import { chromium, defineConfig, devices } from '@playwright/test';

function resolveBrowserProject() {
  let hasPlaywrightChromium = false;
  try {
    const exePath = chromium.executablePath();
    hasPlaywrightChromium = Boolean(exePath && fs.existsSync(exePath));
  } catch {
    hasPlaywrightChromium = false;
  }

  const edgeCandidates = [
    path.join(
      process.env['ProgramFiles(x86)'] || 'C:\\Program Files (x86)',
      'Microsoft\\Edge\\Application\\msedge.exe'
    ),
    path.join(
      process.env.ProgramFiles || 'C:\\Program Files',
      'Microsoft\\Edge\\Application\\msedge.exe'
    ),
  ];
  const hasEdge = edgeCandidates.some((candidate) => fs.existsSync(candidate));

  const args = ['--enable-webgl', '--ignore-gpu-blocklist'];
  if (process.env.CI) {
    args.push('--enable-unsafe-swiftshader');
  }

  if (hasPlaywrightChromium || process.env.PLAYWRIGHT_FORCE_CHROMIUM === '1' || !hasEdge) {
    return {
      name: 'chromium-liquid-glass',
      use: {
        ...devices['Desktop Chrome'],
        headless: true,
        viewport: { width: 1440, height: 900 },
        launchOptions: { args },
      },
    };
  }

  return {
    name: 'msedge-liquid-glass',
    use: {
      ...devices['Desktop Edge'],
      channel: 'msedge',
      headless: true,
      viewport: { width: 1440, height: 900 },
      launchOptions: { args },
    },
  };
}

export default defineConfig({
  testDir: './e2e',
  fullyParallel: false,
  forbidOnly: !!process.env.CI,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  timeout: 60_000,
  expect: {
    timeout: 10_000,
  },
  reporter: [
    ['list'],
    ['html', { open: 'never', outputFolder: 'playwright-report' }],
  ],
  use: {
    baseURL: 'http://localhost:1420',
    trace: 'on-first-retry',
    screenshot: 'only-on-failure',
    video: 'off',
  },
  projects: [resolveBrowserProject()],
  webServer: {
    command: 'pnpm dev',
    url: 'http://localhost:1420',
    reuseExistingServer: !process.env.CI,
    timeout: 120_000,
  },
});
