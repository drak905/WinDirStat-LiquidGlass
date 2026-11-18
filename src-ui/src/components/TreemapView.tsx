import React, { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  ChevronRight,
  Folder,
  File as FileIcon,
  Layers,
  ArrowUp,
  Gauge,
} from 'lucide-react';
import type { ExtensionSummary, StorageNode } from '../types/storage';
import { formatBytes } from '../utils/formatters';
import {
  computeTreemapLayout,
  hitTestTreemap,
  renderTreemapToBuffer,
  type TreemapRect,
} from '../utils/treemapLayout';
import {
  createWebGL2CushionRenderer,
  type WebGL2CushionRenderer,
} from '../utils/webglCushionRenderer';
import { nodeToContextTarget, type ContextMenuTarget } from './ContextMenu';

interface TreemapViewProps {
  root: StorageNode;
  extensions: ExtensionSummary[];
  onContextMenuTarget: (x: number, y: number, target: ContextMenuTarget) => void;
  externalZoomTarget?: StorageNode | null;
}

const DEFAULT_WIDTH = 960;
const DEFAULT_HEIGHT = 540;

function findPathToNode(current: StorageNode, targetId: string): StorageNode[] | null {
  if (current.id === targetId) return [current];
  if (!current.children) return null;
  for (const child of current.children) {
    const sub = findPathToNode(child, targetId);
    if (sub) return [current, ...sub];
  }
  return null;
}

export const TreemapView: React.FC<TreemapViewProps> = ({
  root,
  extensions,
  onContextMenuTarget,
  externalZoomTarget,
}) => {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const offscreenCanvasRef = useRef<HTMLCanvasElement | null>(null);
  const glRendererRef = useRef<WebGL2CushionRenderer | null>(null);
  const bufferRef = useRef<Uint8ClampedArray | null>(null);
  const cachedImageDataRef = useRef<ImageData | null>(null);
  const cursorRef = useRef({ x: -1000, y: -1000 });
  const hoveredRectRef = useRef<TreemapRect | null>(null);
  const rafCompositeRef = useRef<number | null>(null);

  const [canvasSize, setCanvasSize] = useState({ width: DEFAULT_WIDTH, height: DEFAULT_HEIGHT });
  const [breadcrumbs, setBreadcrumbs] = useState<StorageNode[]>([root]);
  const [hoveredRect, setHoveredRect] = useState<TreemapRect | null>(null);
  const [selectedNodeId, setSelectedNodeId] = useState<string | null>(null);
  const [frameTimeMs, setFrameTimeMs] = useState<number>(0);

  // Reset breadcrumbs when scanned root changes
  useEffect(() => {
    setBreadcrumbs([root]);
    setSelectedNodeId(null);
    setHoveredRect(null);
  }, [root]);

  useEffect(() => {
    if (!externalZoomTarget) return;
    const found = findPathToNode(root, externalZoomTarget.id);
    if (found && found.length > 0) {
      setBreadcrumbs(found);
    }
  }, [externalZoomTarget, root]);

  const activeRoot = breadcrumbs[breadcrumbs.length - 1] ?? root;

  // Observe container dimensions (with graceful fallback in jsdom/SSR)
  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const updateSize = () => {
      const rect = el.getBoundingClientRect();
      if (rect.width >= 200 && rect.height >= 200) {
        setCanvasSize({
          width: Math.floor(rect.width),
          height: Math.floor(rect.height),
        });
      }
    };
    updateSize();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(updateSize);
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  // Compute squarified layout with hierarchical pixel-threshold pruning
  const layout = useMemo(() => {
    return computeTreemapLayout(activeRoot, {
      x: 0,
      y: 0,
      width: canvasSize.width,
      height: canvasSize.height,
      minPixelThreshold: 2.0,
      maxDepth: 10,
    });
  }, [activeRoot, canvasSize.width, canvasSize.height]);

  // Initialize WebGL2 renderer if supported
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const renderer = createWebGL2CushionRenderer(canvas);
    glRendererRef.current = renderer;
    return () => {
      renderer?.dispose();
      glRendererRef.current = null;
    };
  }, []);

  // Stage 1: Re-shade cushion pixel buffer & upload GPU texture only when layout or selection changes
  useEffect(() => {
    const { width, height } = canvasSize;
    if (width <= 0 || height <= 0) return;

    const needed = width * height * 4;
    if (!bufferRef.current || bufferRef.current.length !== needed) {
      bufferRef.current = new Uint8ClampedArray(needed);
    }

    const t0 = performance.now();
    renderTreemapToBuffer(
      layout.rects,
      bufferRef.current,
      width,
      height,
      selectedNodeId
    );
    const shadeMs = performance.now() - t0;

    if (glRendererRef.current) {
      glRendererRef.current.uploadTexture(bufferRef.current, width, height);
    } else if (typeof ImageData !== 'undefined') {
      const imgData = new ImageData(
        new Uint8ClampedArray(bufferRef.current),
        width,
        height
      );
      cachedImageDataRef.current = imgData;
      if (typeof document !== 'undefined') {
        const offscreen = offscreenCanvasRef.current ?? document.createElement('canvas');
        offscreen.width = width;
        offscreen.height = height;
        offscreenCanvasRef.current = offscreen;
        try {
          offscreen.getContext('2d')?.putImageData(imgData, 0, 0);
        } catch {
          // jsdom / environments without 2D canvas context fall back cleanly
        }
      }
    }

    setFrameTimeMs(Number((layout.elapsedMs + shadeMs).toFixed(2)));
  }, [layout, canvasSize, selectedNodeId]);

  // Stage 2: Composite cursor-linked specular glass highlight directly via GPU/canvas without React re-renders
  const compositeOverlay = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const { width, height } = canvasSize;
    if (width <= 0 || height <= 0) return;

    const cursor = cursorRef.current;
    const activeHover = hoveredRectRef.current;

    const glRenderer = glRendererRef.current;
    if (glRenderer) {
      const hr: [number, number, number, number] | null = activeHover
        ? [activeHover.x, activeHover.y, activeHover.w, activeHover.h]
        : null;
      glRenderer.draw(width, height, cursor.x, cursor.y, hr);
      return;
    }

    let ctx: CanvasRenderingContext2D | null = null;
    try {
      ctx = canvas.getContext('2d');
    } catch {
      ctx = null;
    }
    if (!ctx) return;

    if (offscreenCanvasRef.current) {
      ctx.drawImage(offscreenCanvasRef.current, 0, 0);
    } else if (cachedImageDataRef.current) {
      ctx.putImageData(cachedImageDataRef.current, 0, 0);
    }

    if (cursor.x >= 0 && cursor.y >= 0) {
      const grad = ctx.createRadialGradient(cursor.x, cursor.y, 4, cursor.x, cursor.y, 130);
      grad.addColorStop(0, 'rgba(255, 255, 255, 0.22)');
      grad.addColorStop(1, 'rgba(255, 255, 255, 0)');
      ctx.fillStyle = grad;
      ctx.fillRect(0, 0, width, height);
    }

    if (activeHover) {
      ctx.fillStyle = 'rgba(255, 255, 255, 0.16)';
      ctx.fillRect(activeHover.x, activeHover.y, activeHover.w, activeHover.h);
      ctx.strokeStyle = 'rgba(255, 255, 255, 0.85)';
      ctx.lineWidth = 1.5;
      ctx.strokeRect(
        activeHover.x + 0.75,
        activeHover.y + 0.75,
        Math.max(1, activeHover.w - 1.5),
        Math.max(1, activeHover.h - 1.5)
      );
    }
  }, [canvasSize]);

  useEffect(() => {
    hoveredRectRef.current = hoveredRect;
    compositeOverlay();
  }, [layout, canvasSize, hoveredRect, selectedNodeId, compositeOverlay]);

  useEffect(() => {
    return () => {
      if (rafCompositeRef.current !== null && typeof cancelAnimationFrame !== 'undefined') {
        cancelAnimationFrame(rafCompositeRef.current);
      }
    };
  }, []);

  const toCanvasCoords = useCallback(
    (clientX: number, clientY: number) => {
      const canvas = canvasRef.current;
      if (!canvas) return { x: clientX, y: clientY };
      const rect = canvas.getBoundingClientRect();
      const dispW = rect.width > 0 ? rect.width : canvasSize.width;
      const dispH = rect.height > 0 ? rect.height : canvasSize.height;
      return {
        x: ((clientX - rect.left) / dispW) * canvasSize.width,
        y: ((clientY - rect.top) / dispH) * canvasSize.height,
      };
    },
    [canvasSize.width, canvasSize.height]
  );

  const zoomIntoNode = useCallback(
    (node: StorageNode) => {
      if (!node.isDir || !node.children || node.children.length === 0) return;
      const path = findPathToNode(root, node.id);
      if (path && path.length > 0) {
        setBreadcrumbs(path);
      } else {
        setBreadcrumbs((prev) => [...prev, node]);
      }
      setHoveredRect(null);
    },
    [root]
  );

  const handleCanvasMouseMove = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const pt = toCanvasCoords(e.clientX, e.clientY);
    cursorRef.current = pt;
    const hit = hitTestTreemap(layout.rects, pt.x, pt.y);
    hoveredRectRef.current = hit;

    if ((hit?.node.id ?? null) !== (hoveredRect?.node.id ?? null)) {
      setHoveredRect(hit);
    } else if (rafCompositeRef.current === null && typeof requestAnimationFrame !== 'undefined') {
      rafCompositeRef.current = requestAnimationFrame(() => {
        rafCompositeRef.current = null;
        compositeOverlay();
      });
    }
  };

  const handleCanvasDoubleClick = (e: React.MouseEvent<HTMLCanvasElement>) => {
    const pt = toCanvasCoords(e.clientX, e.clientY);
    const hit = hitTestTreemap(layout.rects, pt.x, pt.y);
    if (!hit) return;
    if (hit.node.isDir) {
      zoomIntoNode(hit.node);
    } else {
      // Zoom into parent folder of leaf if not already at immediate parent
      const path = findPathToNode(root, hit.node.id);
      if (path && path.length >= 2) {
        setBreadcrumbs(path.slice(0, -1));
      }
    }
  };

  const handleCanvasContextMenu = (e: React.MouseEvent<HTMLCanvasElement>) => {
    e.preventDefault();
    const pt = toCanvasCoords(e.clientX, e.clientY);
    const hit = hitTestTreemap(layout.rects, pt.x, pt.y);
    const targetNode = hit?.node ?? layout.rects[0]?.node ?? activeRoot;
    onContextMenuTarget(e.clientX, e.clientY, nodeToContextTarget(targetNode));
  };

  const visibleChildren = useMemo(() => {
    return (activeRoot.children ?? []).slice(0, 60);
  }, [activeRoot]);

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      {/* Top Breadcrumb & Performance Bar */}
      <div className="liquid-glass-card flex flex-wrap items-center justify-between gap-2 rounded-2xl px-4 py-2.5">
        <div
          data-testid="treemap-breadcrumbs"
          className="flex flex-wrap items-center gap-1 text-xs"
        >
          {breadcrumbs.length > 1 && (
            <button
              type="button"
              onClick={() => setBreadcrumbs((prev) => prev.slice(0, -1))}
              className="mr-1 flex h-6 items-center gap-1 rounded-lg border border-white/25 bg-white/15 px-2 text-[11px] font-medium text-slate-800 hover:bg-white/25 dark:text-slate-100"
            >
              <ArrowUp className="h-3 w-3" />
              Up
            </button>
          )}
          {breadcrumbs.map((crumb, idx) => {
            const isLast = idx === breadcrumbs.length - 1;
            return (
              <React.Fragment key={crumb.id}>
                {idx > 0 && <ChevronRight className="h-3.5 w-3.5 text-slate-400" />}
                <button
                  type="button"
                  data-testid={idx === 0 ? 'breadcrumb-root' : `breadcrumb-${crumb.name}`}
                  onClick={() => setBreadcrumbs((prev) => prev.slice(0, idx + 1))}
                  className={`rounded-lg px-2 py-1 font-medium transition ${
                    isLast
                      ? 'bg-sky-500/25 text-sky-800 dark:text-sky-200'
                      : 'text-slate-700 hover:bg-white/20 dark:text-slate-300'
                  }`}
                >
                  {crumb.name}
                </button>
              </React.Fragment>
            );
          })}
        </div>

        <div className="flex items-center gap-2 text-[11px]">
          <span className="flex items-center gap-1 rounded-full border border-emerald-400/30 bg-emerald-500/15 px-2.5 py-0.5 font-mono font-medium text-emerald-700 dark:text-emerald-300">
            <Gauge className="h-3 w-3" />
            {frameTimeMs.toFixed(1)} ms
          </span>
          <span className="rounded-full border border-white/20 bg-white/15 px-2.5 py-0.5 font-mono text-slate-700 dark:text-slate-200">
            {layout.rects.length.toLocaleString()} visible • {layout.culledCount.toLocaleString()} culled
          </span>
        </div>
      </div>

      {/* Main Split: Directory/Extensions Explorer + Cushion Canvas */}
      <div className="grid min-h-0 flex-1 grid-cols-1 gap-3 lg:grid-cols-12">
        {/* Interactive WebGL2 / Cushion Canvas */}
        <div
          ref={containerRef}
          className="liquid-glass-card relative min-h-[320px] overflow-hidden rounded-3xl p-2 lg:col-span-8"
        >
          <canvas
            ref={canvasRef}
            data-testid="treemap-canvas"
            width={canvasSize.width}
            height={canvasSize.height}
            onMouseMove={handleCanvasMouseMove}
            onMouseLeave={() => setHoveredRect(null)}
            onClick={(e) => {
              const pt = toCanvasCoords(e.clientX, e.clientY);
              const hit = hitTestTreemap(layout.rects, pt.x, pt.y);
              if (hit) setSelectedNodeId(hit.node.id);
            }}
            onDoubleClick={handleCanvasDoubleClick}
            onContextMenu={handleCanvasContextMenu}
            className="h-full w-full cursor-crosshair rounded-2xl"
          />

          {/* Floating Liquid Glass Hover Inspector Pill */}
          {hoveredRect && (
            <div className="liquid-glass pointer-events-none absolute bottom-4 left-4 right-4 flex items-center justify-between gap-3 rounded-2xl px-3.5 py-2 text-xs">
              <div className="min-w-0">
                <div className="truncate font-semibold text-slate-900 dark:text-white">
                  {hoveredRect.node.name}
                  {hoveredRect.isAggregated && (
                    <span className="ml-2 rounded bg-indigo-500/25 px-1.5 py-0.5 text-[10px] text-indigo-700 dark:text-indigo-200">
                      Aggregated ({hoveredRect.node.fileCount.toLocaleString()} files)
                    </span>
                  )}
                </div>
                <div className="truncate font-mono text-[10px] text-slate-600 dark:text-slate-300">
                  {hoveredRect.node.path}
                </div>
              </div>
              <div className="shrink-0 text-right">
                <div className="font-mono font-bold text-sky-700 dark:text-sky-300">
                  {formatBytes(hoveredRect.node.size)}
                </div>
                <div className="text-[10px] text-slate-600 dark:text-slate-400">
                  {((hoveredRect.node.size / Math.max(1, activeRoot.size)) * 100).toFixed(1)}% of view
                </div>
              </div>
            </div>
          )}
        </div>

        {/* Classic WinDirStat Side Panel: Subdirectories & Extension Groups */}
        <div className="flex min-h-0 flex-col gap-3 lg:col-span-4">
          <div className="liquid-glass-card flex min-h-0 flex-1 flex-col rounded-3xl p-3">
            <div className="mb-2 flex items-center justify-between px-1">
              <span className="text-xs font-semibold text-slate-900 dark:text-white">
                Directory Tree
              </span>
              <span className="text-[10px] text-slate-600 dark:text-slate-400">
                Double-click folder to zoom • Right-click actions
              </span>
            </div>
            <div className="glass-scroll min-h-0 flex-1 space-y-1 overflow-y-auto pr-1">
              {visibleChildren.map((child) => {
                const pct = (child.size / Math.max(1, activeRoot.size)) * 100;
                const isSelected = selectedNodeId === child.id;
                return (
                  <div
                    key={child.id}
                    data-testid={child.isDir ? `dir-node-${child.name}` : `file-node-${child.name}`}
                    onClick={() => setSelectedNodeId(child.id)}
                    onDoubleClick={() => {
                      if (child.isDir) zoomIntoNode(child);
                    }}
                    onContextMenu={(e) => {
                      e.preventDefault();
                      onContextMenuTarget(e.clientX, e.clientY, nodeToContextTarget(child));
                    }}
                    className={`flex cursor-pointer items-center justify-between gap-2 rounded-xl px-2.5 py-1.5 text-xs transition ${
                      isSelected
                        ? 'bg-sky-500/25 text-slate-950 dark:text-white'
                        : 'hover:bg-white/15 text-slate-800 dark:text-slate-200'
                    }`}
                  >
                    <div className="flex min-w-0 items-center gap-2">
                      {child.isDir ? (
                        <Folder className="h-3.5 w-3.5 shrink-0 text-amber-400" />
                      ) : (
                        <FileIcon className="h-3.5 w-3.5 shrink-0 text-sky-400" />
                      )}
                      <span className="truncate font-medium">{child.name}</span>
                    </div>
                    <div className="shrink-0 text-right font-mono text-[11px]">
                      <span>{formatBytes(child.size)}</span>
                      <span className="ml-1.5 text-[10px] text-slate-500 dark:text-slate-400">
                        {pct.toFixed(1)}%
                      </span>
                    </div>
                  </div>
                );
              })}
            </div>
          </div>

          {/* Extension Legend (Right-Clickable Extension Groups) */}
          <div className="liquid-glass-card flex max-h-52 flex-col rounded-3xl p-3">
            <div className="mb-2 flex items-center gap-1.5 px-1 text-xs font-semibold text-slate-900 dark:text-white">
              <Layers className="h-3.5 w-3.5 text-fuchsia-400" />
              Extension Groups
            </div>
            <div className="glass-scroll flex-1 space-y-1 overflow-y-auto pr-1">
              {extensions.slice(0, 12).map((ext) => (
                <div
                  key={ext.extension}
                  data-testid={`ext-group-${ext.extension}`}
                  onContextMenu={(e) => {
                    e.preventDefault();
                    onContextMenuTarget(e.clientX, e.clientY, {
                      name: `${ext.extension} files (${ext.fileCount})`,
                      path: ext.samplePath,
                      sizeBytes: ext.totalBytes,
                      isDir: false,
                      kind: 'extension',
                    });
                  }}
                  className="flex cursor-pointer items-center justify-between rounded-xl px-2.5 py-1 text-xs text-slate-800 hover:bg-white/15 dark:text-slate-200"
                >
                  <div className="flex items-center gap-2">
                    <span
                      className="h-2.5 w-2.5 rounded-sm shadow"
                      style={{ backgroundColor: ext.color }}
                    />
                    <span className="font-mono font-semibold">{ext.extension}</span>
                    <span className="text-[10px] text-slate-500 dark:text-slate-400">
                      ({ext.fileCount.toLocaleString()})
                    </span>
                  </div>
                  <span className="font-mono text-[11px]">
                    {formatBytes(ext.totalBytes)}
                  </span>
                </div>
              ))}
            </div>
          </div>
        </div>
      </div>
    </div>
  );
};
