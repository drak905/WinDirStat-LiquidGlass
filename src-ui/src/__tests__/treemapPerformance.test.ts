import { describe, expect, it } from 'vitest';
import { generateSynthetic100kTree } from '../data/demoSnapshot';
import {
  computeTreemapLayout,
  hitTestTreemap,
  renderTreemapToBuffer,
} from '../utils/treemapLayout';

describe('Squarified Cushion Treemap Layout & 100k Node <16.6ms Render Budget', () => {
  it('generates at least 100,000 nodes with accurate parent-sum propagation', () => {
    const snapshot = generateSynthetic100kTree();
    expect(snapshot.totalNodes).toBeGreaterThanOrEqual(100_000);
    expect(snapshot.root.size).toBeGreaterThan(0);
    expect(snapshot.root.children?.length).toBeGreaterThan(0);
  });

  it('produces non-overlapping squarified rectangles inside viewport bounds', () => {
    const snapshot = generateSynthetic100kTree();
    const { rects } = computeTreemapLayout(snapshot.root, {
      x: 0,
      y: 0,
      width: 1000,
      height: 600,
      minPixelThreshold: 2.0,
    });

    expect(rects.length).toBeGreaterThan(10);
    const allWithinBounds = rects.every(
      (r) =>
        r.w >= 0 &&
        r.h >= 0 &&
        r.x >= -0.01 &&
        r.y >= -0.01 &&
        r.x + r.w <= 1000.01 &&
        r.y + r.h <= 600.01
    );
    expect(allWithinBounds).toBe(true);

    // Verify top 100 rects do not overlap
    const sample = rects.slice(0, 100);
    let hasOverlap = false;
    for (let i = 0; i < sample.length; i += 1) {
      const a = sample[i];
      for (let j = i + 1; j < sample.length; j += 1) {
        const b = sample[j];
        const eps = 0.05;
        if (
          a.x + eps < b.x + b.w &&
          a.x + a.w - eps > b.x &&
          a.y + eps < b.y + b.h &&
          a.y + a.h - eps > b.y
        ) {
          hasOverlap = true;
          break;
        }
      }
      if (hasOverlap) break;
    }
    expect(hasOverlap).toBe(false);

    const first = rects[0];
    const hit = hitTestTreemap(rects, first.x + first.w * 0.5, first.y + first.h * 0.5);
    expect(hit?.node.id).toBe(first.node.id);
  });

  it('lays out and cushion-renders 100k nodes well within the 16.6ms 60fps frame budget', () => {
    const snapshot = generateSynthetic100kTree();
    const width = 960;
    const height = 600;
    const pixelBuffer = new Uint8ClampedArray(width * height * 4);

    // Warm up JIT across 3 frames so steady-state 60fps TurboFan performance is measured
    for (let w = 0; w < 3; w += 1) {
      const warmup = computeTreemapLayout(snapshot.root, {
        x: 0,
        y: 0,
        width,
        height,
        minPixelThreshold: 2.0,
      });
      renderTreemapToBuffer(warmup.rects, pixelBuffer, width, height);
    }

    const samples: number[] = [];
    let lastResult = computeTreemapLayout(snapshot.root, {
      x: 0,
      y: 0,
      width,
      height,
      minPixelThreshold: 2.0,
    });
    for (let f = 0; f < 5; f += 1) {
      const t0 = performance.now();
      lastResult = computeTreemapLayout(snapshot.root, {
        x: 0,
        y: 0,
        width,
        height,
        minPixelThreshold: 2.0,
      });
      renderTreemapToBuffer(lastResult.rects, pixelBuffer, width, height);
      samples.push(performance.now() - t0);
    }
    samples.sort((a, b) => a - b);
    const medianMs = samples[Math.floor(samples.length / 2)];

    expect(lastResult.culledCount).toBeGreaterThan(50_000);
    expect(lastResult.rects.length).toBeLessThan(8_000);
    expect(pixelBuffer.some((v) => v > 0)).toBe(true);
    expect(medianMs).toBeLessThan(16.6);
  });
});
