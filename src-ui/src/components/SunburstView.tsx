import React, { useEffect, useMemo, useState } from 'react';
import { ArrowUp, Disc } from 'lucide-react';
import type { StorageNode } from '../types/storage';
import { formatBytes, getExtensionColor } from '../utils/formatters';
import { nodeToContextTarget, type ContextMenuTarget } from './ContextMenu';

interface SunburstViewProps {
  root: StorageNode;
  onContextMenuNode: (x: number, y: number, target: ContextMenuTarget) => void;
}

interface SunburstArc {
  node: StorageNode;
  depth: number;
  startAngle: number;
  endAngle: number;
  innerR: number;
  outerR: number;
  color: string;
}

function polarToCartesian(cx: number, cy: number, r: number, angleRad: number) {
  return {
    x: cx + r * Math.cos(angleRad),
    y: cy + r * Math.sin(angleRad),
  };
}

function describeArc(
  cx: number,
  cy: number,
  innerR: number,
  outerR: number,
  startAngle: number,
  endAngle: number
): string {
  const span = Math.min(endAngle - startAngle, Math.PI * 1.999);
  const adjustedEnd = startAngle + span;
  const p1 = polarToCartesian(cx, cy, outerR, startAngle);
  const p2 = polarToCartesian(cx, cy, outerR, adjustedEnd);
  const p3 = polarToCartesian(cx, cy, innerR, adjustedEnd);
  const p4 = polarToCartesian(cx, cy, innerR, startAngle);
  const largeArc = span > Math.PI ? 1 : 0;

  return [
    `M ${p1.x.toFixed(2)} ${p1.y.toFixed(2)}`,
    `A ${outerR} ${outerR} 0 ${largeArc} 1 ${p2.x.toFixed(2)} ${p2.y.toFixed(2)}`,
    `L ${p3.x.toFixed(2)} ${p3.y.toFixed(2)}`,
    `A ${innerR} ${innerR} 0 ${largeArc} 0 ${p4.x.toFixed(2)} ${p4.y.toFixed(2)}`,
    'Z',
  ].join(' ');
}

export const SunburstView: React.FC<SunburstViewProps> = ({
  root,
  onContextMenuNode,
}) => {
  const [zoomStack, setZoomStack] = useState<StorageNode[]>([root]);
  const [hoveredArc, setHoveredArc] = useState<SunburstArc | null>(null);

  useEffect(() => {
    setZoomStack([root]);
    setHoveredArc(null);
  }, [root]);

  const activeRoot = zoomStack[zoomStack.length - 1] ?? root;

  const arcs = useMemo(() => {
    const out: SunburstArc[] = [];
    const maxDepth = 4;
    const baseR = 68;
    const ringThickness = 44;

    function walk(node: StorageNode, depth: number, a0: number, a1: number) {
      if (depth > maxDepth || !node.children || node.children.length === 0 || node.size <= 0) {
        return;
      }
      let cursor = a0;
      const span = a1 - a0;
      for (const child of node.children) {
        if (child.size <= 0) continue;
        const childSpan = (child.size / node.size) * span;
        // Angle-threshold pruning keeps sunburst responsive even on 100k snapshots
        if (childSpan < 0.015) {
          cursor += childSpan;
          continue;
        }
        const leadExt = child.isDir ? child.children?.[0]?.extension : child.extension;
        out.push({
          node: child,
          depth,
          startAngle: cursor,
          endAngle: cursor + childSpan,
          innerR: baseR + (depth - 1) * ringThickness,
          outerR: baseR + depth * ringThickness - 3,
          color: getExtensionColor(leadExt),
        });
        if (child.isDir && depth < maxDepth) {
          walk(child, depth + 1, cursor, cursor + childSpan);
        }
        cursor += childSpan;
      }
    }

    walk(activeRoot, 1, -Math.PI / 2, Math.PI * 1.5);
    return out;
  }, [activeRoot]);

  const centerTarget = hoveredArc?.node ?? activeRoot;

  return (
    <div className="flex h-full min-h-0 flex-col gap-3">
      <div className="liquid-glass-card flex items-center justify-between rounded-2xl px-4 py-2.5 text-xs">
        <div className="flex items-center gap-2">
          <Disc className="h-4 w-4 text-sky-400" />
          <span className="font-semibold text-slate-900 dark:text-white">
            Radial Sunburst — {activeRoot.name}
          </span>
          {zoomStack.length > 1 && (
            <button
              type="button"
              onClick={() => setZoomStack((prev) => prev.slice(0, -1))}
              className="flex items-center gap-1 rounded-lg border border-white/25 bg-white/15 px-2 py-0.5 text-[11px] font-medium hover:bg-white/25"
            >
              <ArrowUp className="h-3 w-3" />
              Zoom Out
            </button>
          )}
        </div>
        <span className="font-mono text-slate-700 dark:text-slate-300">
          {arcs.length} visible rings • {formatBytes(activeRoot.size)}
        </span>
      </div>

      <div className="liquid-glass-card relative flex min-h-0 flex-1 items-center justify-center overflow-hidden rounded-3xl p-4">
        <svg
          data-testid="sunburst-svg"
          viewBox="0 0 520 520"
          className="h-full max-h-[520px] w-full max-w-[520px] select-none"
        >
          {arcs.map((arc) => {
            const isHovered = hoveredArc?.node.id === arc.node.id;
            return (
              <path
                key={arc.node.id}
                data-node-id={arc.node.id}
                d={describeArc(260, 260, arc.innerR, arc.outerR, arc.startAngle, arc.endAngle)}
                fill={arc.color}
                fillOpacity={isHovered ? 0.96 : 0.76}
                stroke="rgba(255,255,255,0.42)"
                strokeWidth={isHovered ? 2 : 0.75}
                className="cursor-pointer transition-opacity duration-150"
                onMouseEnter={() => setHoveredArc(arc)}
                onMouseLeave={() => setHoveredArc(null)}
                onClick={() => {
                  if (arc.node.isDir && arc.node.children?.length) {
                    setZoomStack((prev) => [...prev, arc.node]);
                  }
                }}
                onContextMenu={(e) => {
                  e.preventDefault();
                  onContextMenuNode(e.clientX, e.clientY, nodeToContextTarget(arc.node));
                }}
              />
            );
          })}

          {/* Center Glass Hub */}
          <circle
            cx={260}
            cy={260}
            r={62}
            fill="rgba(15, 23, 42, 0.55)"
            stroke="rgba(255, 255, 255, 0.35)"
            strokeWidth={1.5}
            className="cursor-pointer"
            onClick={() => {
              if (zoomStack.length > 1) setZoomStack((prev) => prev.slice(0, -1));
            }}
          />
          <text
            x={260}
            y={252}
            textAnchor="middle"
            className="fill-white text-[11px] font-semibold"
          >
            {centerTarget.name.slice(0, 16)}
          </text>
          <text
            x={260}
            y={272}
            textAnchor="middle"
            className="fill-sky-300 font-mono text-[12px] font-bold"
          >
            {formatBytes(centerTarget.size)}
          </text>
        </svg>
      </div>
    </div>
  );
};
