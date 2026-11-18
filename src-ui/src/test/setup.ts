import '@testing-library/jest-dom/vitest';
import { cleanup } from '@testing-library/react';
import { afterEach } from 'vitest';

afterEach(() => {
  cleanup();
  delete window.__WDS_TEST_BRIDGE__;
});

if (typeof globalThis.ResizeObserver === 'undefined') {
  globalThis.ResizeObserver = class ResizeObserver {
    observe() {}
    unobserve() {}
    disconnect() {}
  };
}

if (typeof HTMLElement.prototype.scrollIntoView !== 'function') {
  HTMLElement.prototype.scrollIntoView = () => {};
}

// jsdom logs "Not implemented: HTMLCanvasElement.prototype.getContext" when queried;
// return null quietly so TreemapView exercises its headless fallback cleanly.
if (typeof HTMLCanvasElement !== 'undefined') {
  HTMLCanvasElement.prototype.getContext = (() => null) as unknown as HTMLCanvasElement['getContext'];
}
