import type { StorageNode } from '../types/storage';
import { getExtensionColor, hexToRgb } from './formatters';

export interface TreemapBounds {
  x: number;
  y: number;
  width: number;
  height: number;
  minPixelThreshold?: number;
  maxDepth?: number;
}

export interface TreemapRect {
  node: StorageNode;
  x: number;
  y: number;
  w: number;
  h: number;
  depth: number;
  ax: number;
  bx: number;
  ay: number;
  by: number;
  rgb: [number, number, number];
  isAggregated: boolean;
}

export interface TreemapLayoutResult {
  rects: TreemapRect[];
  culledCount: number;
  elapsedMs: number;
}

const CUSHION_INITIAL_HEIGHT = 0.52;
const CUSHION_DECAY = 0.74;

// Scalar ridge helpers avoid allocating [a, b] tuples during 100k-node traversal
let ridgeA = 0;
let ridgeB = 0;

function computeRidge(
  c1: number,
  c2: number,
  h: number,
  a: number,
  b: number
): void {
  const span = c2 - c1;
  if (span <= 0.5) {
    ridgeA = a;
    ridgeB = b;
    return;
  }
  const k = (4 * h) / (span * span);
  ridgeA = a - k;
  ridgeB = b + k * (c1 + c2);
}

function worstAspectRatio(
  side: number,
  rowArea: number,
  maxArea: number,
  minArea: number
): number {
  if (rowArea <= 0 || minArea <= 0 || side <= 0) return Number.POSITIVE_INFINITY;
  const s2 = rowArea * rowArea;
  const w2 = side * side;
  return Math.max((w2 * maxArea) / s2, s2 / (w2 * minArea));
}

const RGB_CACHE = new Map<string, [number, number, number]>();
const DEFAULT_DIR_RGB: [number, number, number] = [79, 70, 229];

function getCachedExtRgb(ext?: string): [number, number, number] {
  const key = ext ? ext.toLowerCase() : 'other';
  let cached = RGB_CACHE.get(key);
  if (!cached) {
    cached = hexToRgb(getExtensionColor(key));
    RGB_CACHE.set(key, cached);
  }
  return cached;
}

function getNodeColorRgb(node: StorageNode): [number, number, number] {
  if (node.isDir) {
    const leadExt = node.children?.[0]?.extension;
    return leadExt ? getCachedExtRgb(leadExt) : DEFAULT_DIR_RGB;
  }
  return getCachedExtRgb(node.extension);
}

export function computeTreemapLayout(
  root: StorageNode,
  bounds: TreemapBounds
): TreemapLayoutResult {
  const t0 = performance.now();
  const minPixelThreshold = bounds.minPixelThreshold ?? 2.0;
  const maxDepth = bounds.maxDepth ?? 10;
  const rects: TreemapRect[] = [];
  let culledCount = 0;

  function squarify(
    children: StorageNode[],
    rx: number,
    ry: number,
    rw: number,
    rh: number,
    depth: number,
    heightFactor: number,
    ax: number,
    bx: number,
    ay: number,
    by: number
  ): void {
    if (children.length === 0 || rw <= 0.4 || rh <= 0.4) return;

    let totalSize = 0;
    for (let i = 0; i < children.length; i += 1) {
      if (children[i].size > 0) totalSize += children[i].size;
    }
    if (totalSize <= 0) return;

    // Ensure descending order when caller provides unsorted input
    let ordered = children;
    let needsSort = false;
    for (let i = 1; i < children.length; i += 1) {
      if (children[i].size > children[i - 1].size) {
        needsSort = true;
        break;
      }
    }
    if (needsSort) {
      ordered = [...children].sort((a, b) => b.size - a.size);
    }

    let curX = rx;
    let curY = ry;
    let remW = rw;
    let remH = rh;
    let remSize = totalSize;
    let i = 0;

    const minSubdivideSide = minPixelThreshold * 3.5;
    const minSubdivideArea = minPixelThreshold * minPixelThreshold * 36;
    const minVisibleCellArea = minPixelThreshold * minPixelThreshold * 6.25;

    while (i < ordered.length && remW > 0.35 && remH > 0.35 && remSize > 0) {
      const first = ordered[i];
      if (first.size <= 0) {
        i += 1;
        continue;
      }

      const shortSide = Math.min(remW, remH);
      const scale = (remW * remH) / remSize;
      const firstArea = first.size * scale;

      // Tail-aggregation culling: since ordered[] is sorted descending, once the largest
      // remaining child falls below minVisibleCellArea, collapse the remaining slice into
      // one aggregate cushion cell covering [curX, curY, remW, remH].
      if (firstArea < minVisibleCellArea) {
        let remainingFiles = 0;
        for (let t = i; t < ordered.length; t += 1) {
          remainingFiles += Math.max(1, ordered[t].fileCount);
        }
        culledCount += Math.max(0, remainingFiles - 1);
        if (remW >= 0.85 && remH >= 0.85) {
          computeRidge(curX, curX + remW, heightFactor, ax, bx);
          const aggAx = ridgeA;
          const aggBx = ridgeB;
          computeRidge(curY, curY + remH, heightFactor, ay, by);
          const aggAy = ridgeA;
          const aggBy = ridgeB;
          rects.push({
            node: first,
            x: curX,
            y: curY,
            w: remW,
            h: remH,
            depth,
            ax: aggAx,
            bx: aggBx,
            ay: aggAy,
            by: aggBy,
            rgb: getNodeColorRgb(first),
            isAggregated: true,
          });
        }
        break;
      }

      let rowEnd = i + 1;
      let rowArea = firstArea;
      let rowMax = firstArea;
      let rowMin = firstArea;
      let currentWorst = worstAspectRatio(shortSide, rowArea, rowMax, rowMin);

      while (rowEnd < ordered.length) {
        const nextNode = ordered[rowEnd];
        if (nextNode.size <= 0) {
          rowEnd += 1;
          continue;
        }
        const nextArea = nextNode.size * scale;
        if (nextArea < minVisibleCellArea) {
          break;
        }
        const candArea = rowArea + nextArea;
        const candMax = Math.max(rowMax, nextArea);
        const candMin = Math.min(rowMin, nextArea);
        const candWorst = worstAspectRatio(shortSide, candArea, candMax, candMin);
        if (candWorst <= currentWorst) {
          rowArea = candArea;
          rowMax = candMax;
          rowMin = candMin;
          currentWorst = candWorst;
          rowEnd += 1;
        } else {
          break;
        }
      }

      const horizontalSplit = remW >= remH;
      const stripThickness = horizontalSplit ? rowArea / remH : rowArea / remW;
      let offset = 0;
      let rowSizeSum = 0;

      for (let k = i; k < rowEnd; k += 1) {
        const child = ordered[k];
        if (child.size <= 0) continue;
        rowSizeSum += child.size;
        const childArea = child.size * scale;
        const span = stripThickness > 0 ? childArea / stripThickness : 0;

        const cx = horizontalSplit ? curX : curX + offset;
        const cy = horizontalSplit ? curY + offset : curY;
        const cw = horizontalSplit ? stripThickness : span;
        const ch = horizontalSplit ? span : stripThickness;
        offset += span;

        if (cw < 0.85 || ch < 0.85) {
          culledCount += Math.max(1, child.fileCount);
          continue;
        }

        computeRidge(cx, cx + cw, heightFactor, ax, bx);
        const nextAx = ridgeA;
        const nextBx = ridgeB;
        computeRidge(cy, cy + ch, heightFactor, ay, by);
        const nextAy = ridgeA;
        const nextBy = ridgeB;

        if (child.isDir && child.children && child.children.length > 0) {
          const tooSmallToSubdivide =
            Math.min(cw, ch) < minSubdivideSide ||
            cw * ch < minSubdivideArea ||
            depth >= maxDepth;

          if (tooSmallToSubdivide) {
            culledCount += Math.max(0, child.fileCount - 1);
            rects.push({
              node: child,
              x: cx,
              y: cy,
              w: cw,
              h: ch,
              depth,
              ax: nextAx,
              bx: nextBx,
              ay: nextAy,
              by: nextBy,
              rgb: getNodeColorRgb(child),
              isAggregated: true,
            });
          } else {
            squarify(
              child.children,
              cx,
              cy,
              cw,
              ch,
              depth + 1,
              heightFactor * CUSHION_DECAY,
              nextAx,
              nextBx,
              nextAy,
              nextBy
            );
          }
        } else {
          rects.push({
            node: child,
            x: cx,
            y: cy,
            w: cw,
            h: ch,
            depth,
            ax: nextAx,
            bx: nextBx,
            ay: nextAy,
            by: nextBy,
            rgb: getNodeColorRgb(child),
            isAggregated: false,
          });
        }
      }

      remSize -= rowSizeSum;
      if (horizontalSplit) {
        curX += stripThickness;
        remW -= stripThickness;
      } else {
        curY += stripThickness;
        remH -= stripThickness;
      }
      i = rowEnd;
    }
  }

  if (root.isDir && root.children && root.children.length > 0) {
    computeRidge(bounds.x, bounds.x + bounds.width, CUSHION_INITIAL_HEIGHT, 0, 0);
    const ax0 = ridgeA;
    const bx0 = ridgeB;
    computeRidge(bounds.y, bounds.y + bounds.height, CUSHION_INITIAL_HEIGHT, 0, 0);
    const ay0 = ridgeA;
    const by0 = ridgeB;
    squarify(
      root.children,
      bounds.x,
      bounds.y,
      bounds.width,
      bounds.height,
      1,
      CUSHION_INITIAL_HEIGHT * CUSHION_DECAY,
      ax0,
      bx0,
      ay0,
      by0
    );
  } else if (root.size > 0) {
    computeRidge(bounds.x, bounds.x + bounds.width, CUSHION_INITIAL_HEIGHT, 0, 0);
    const ax0 = ridgeA;
    const bx0 = ridgeB;
    computeRidge(bounds.y, bounds.y + bounds.height, CUSHION_INITIAL_HEIGHT, 0, 0);
    const ay0 = ridgeA;
    const by0 = ridgeB;
    rects.push({
      node: root,
      x: bounds.x,
      y: bounds.y,
      w: bounds.width,
      h: bounds.height,
      depth: 0,
      ax: ax0,
      bx: bx0,
      ay: ay0,
      by: by0,
      rgb: getNodeColorRgb(root),
      isAggregated: false,
    });
  }

  return {
    rects,
    culledCount,
    elapsedMs: performance.now() - t0,
  };
}

/**
 * Renders cushion-shaded rectangles directly into an RGBA Uint8ClampedArray buffer
 * using van Wijk quadratic surface normals. Zero-allocation hot loop for <16.6ms 60fps.
 */
export function renderTreemapToBuffer(
  rects: TreemapRect[],
  buffer: Uint8ClampedArray,
  width: number,
  height: number,
  hoveredNodeId?: string | null
): void {
  const totalPixels = width * height;
  if (totalPixels <= 0 || buffer.length < totalPixels * 4) return;

  // Fill deep translucent obsidian seam background via signed Int32Array (zero float64 boxing)
  const i32 = new Int32Array(buffer.buffer, buffer.byteOffset, totalPixels);
  i32.fill((0xff1e1612 | 0)); // little-endian ABGR -> rgba(18, 22, 30, 255)

  // Normalized top-left directional light vector (-0.36, -0.36, 0.86)
  const lx = -0.36;
  const ly = -0.36;
  const lz = 0.86;
  const ambient = 0.28;
  const diffuse = 0.72; // ambient + diffuse == 1.00 -> intensity in [0.28, 1.00]

  for (let rIdx = 0; rIdx < rects.length; rIdx += 1) {
    const rect = rects[rIdx];
    const px0 = Math.max(0, rect.x | 0);
    const py0 = Math.max(0, rect.y | 0);
    let px1 = Math.min(width, (rect.x + rect.w) | 0);
    let py1 = Math.min(height, (rect.y + rect.h) | 0);

    if (px1 - px0 >= 3) px1 -= 1;
    if (py1 - py0 >= 3) py1 -= 1;
    if (px1 <= px0 || py1 <= py0) continue;

    const rgb = rect.rgb;
    const baseR = rgb[0];
    const baseG = rgb[1];
    const baseB = rgb[2];
    const hoverLift = hoveredNodeId === rect.node.id ? 38 : 0;

    const ax = rect.ax;
    const bx = rect.bx;
    const ay = rect.ay;
    const by = rect.by;
    const dnx = -2 * ax;
    const nxStart = -(2 * ax * (px0 + 0.5) + bx);

    for (let y = py0; y < py1; y += 1) {
      const ny = -(2 * ay * (y + 0.5) + by);
      const ny2Plus1 = ny * ny + 1.0;
      const nyLyLz = ny * ly + lz;

      let nx = nxStart;
      let pIdx = y * width + px0;

      for (let x = px0; x < px1; x += 1) {
        const dot = nx * lx + nyLyLz;
        const cosa = dot > 0 ? dot / Math.sqrt(nx * nx + ny2Plus1) : 0;
        const intensity = ambient + diffuse * cosa;

        let r = ((baseR * intensity) | 0) + hoverLift;
        let g = ((baseG * intensity) | 0) + hoverLift;
        let b = ((baseB * intensity) | 0) + hoverLift;
        if (r > 255) r = 255;
        if (g > 255) g = 255;
        if (b > 255) b = 255;

        i32[pIdx++] = 0xff000000 | (b << 16) | (g << 8) | r;
        nx += dnx;
      }
    }
  }
}

export function hitTestTreemap(
  rects: TreemapRect[],
  px: number,
  py: number
): TreemapRect | null {
  for (let i = rects.length - 1; i >= 0; i -= 1) {
    const r = rects[i];
    if (px >= r.x && px <= r.x + r.w && py >= r.y && py <= r.y + r.h) {
      return r;
    }
  }
  return null;
}
