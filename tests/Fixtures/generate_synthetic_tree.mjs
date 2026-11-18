#!/usr/bin/env node
/**
 * Deterministic Synthetic Disk Tree Generator (/SaveTo-compatible JSON & Nested Tree)
 *
 * Generates realistic Windows filesystem trees (default: 100,000 nodes) formatted
 * identically to WinDirStat's `/SaveTo <file.json>` output (`windirstat/CsvLoader.cpp`),
 * while also supporting nested tree JSON for frontend WebGL treemap benchmarks.
 *
 * Usage:
 *   node tests/fixtures/generate_synthetic_tree.mjs --verify
 *   node tests/fixtures/generate_synthetic_tree.mjs --count 100000 --out tests/fixtures/synthetic-tree-100k.json
 *   node tests/fixtures/generate_synthetic_tree.mjs --count 500 --out tests/fixtures/synthetic-tree-sample.json
 */

import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

// WinDirStat ITEMTYPE flags from windirstat/Item.h
export const ITEM_TYPE = Object.freeze({
  IT_DRIVE: 1 << 1,
  IT_DIRECTORY: 1 << 2,
  IT_FILE: 1 << 3,
  ITF_ROOTITEM: 1 << 28,
  ITF_DONE: 1 << 29,
});

const WDS_ATTR_DRIVE_ROOT = formatHex32(ITEM_TYPE.IT_DRIVE | ITEM_TYPE.ITF_ROOTITEM | ITEM_TYPE.ITF_DONE);
const WDS_ATTR_DIRECTORY = formatHex32(ITEM_TYPE.IT_DIRECTORY | ITEM_TYPE.ITF_DONE);
const WDS_ATTR_FILE = formatHex32(ITEM_TYPE.IT_FILE | ITEM_TYPE.ITF_DONE);

export const DEFAULT_SEED = 0x57445331; // "WDS1"
export const DEFAULT_NODE_COUNT = 100_000;

function formatHex32(num) {
  return `0x${(num >>> 0).toString(16).toUpperCase().padStart(8, '0')}`;
}

function formatHex64(num) {
  return `0x${BigInt(num).toString(16).toUpperCase().padStart(16, '0')}`;
}

/**
 * Deterministic Mulberry32 PRNG.
 * @param {number} seed
 * @returns {() => number}
 */
export function createPrng(seed = DEFAULT_SEED) {
  let state = seed >>> 0;
  return () => {
    state = (state + 0x6d2b79f5) >>> 0;
    let t = Math.imul(state ^ (state >>> 15), 1 | state);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

function randomInt(rng, min, maxInclusive) {
  return min + Math.floor(rng() * (maxInclusive - min + 1));
}

function pick(rng, list) {
  return list[Math.floor(rng() * list.length)];
}

function deterministicTimestamp(rng) {
  // Deterministic UTC window: 2025-01-01T00:00:00Z .. 2026-11-18T23:59:59Z
  const startMs = Date.UTC(2025, 0, 1, 0, 0, 0);
  const endMs = Date.UTC(2026, 10, 18, 23, 59, 59);
  const ts = startMs + Math.floor(rng() * (endMs - startMs));
  return new Date(ts).toISOString().replace(/\.\d{3}Z$/, 'Z');
}

function alignCluster4K(bytes) {
  if (bytes === 0) return 0;
  return Math.ceil(bytes / 4096) * 4096;
}

const FILE_PROFILES = [
  { ext: '.dll', min: 16 * 1024, max: 8 * 1024 * 1024, attr: 'A' },
  { ext: '.exe', min: 64 * 1024, max: 32 * 1024 * 1024, attr: 'A' },
  { ext: '.pdb', min: 128 * 1024, max: 48 * 1024 * 1024, attr: 'A' },
  { ext: '.log', min: 1024, max: 4 * 1024 * 1024, attr: 'A' },
  { ext: '.tmp', min: 4096, max: 16 * 1024 * 1024, attr: 'HA' },
  { ext: '.js', min: 512, max: 256 * 1024, attr: 'A' },
  { ext: '.ts', min: 512, max: 192 * 1024, attr: 'A' },
  { ext: '.rs', min: 512, max: 128 * 1024, attr: 'A' },
  { ext: '.cpp', min: 1024, max: 256 * 1024, attr: 'A' },
  { ext: '.json', min: 256, max: 512 * 1024, attr: 'A' },
  { ext: '.bin', min: 32 * 1024, max: 64 * 1024 * 1024, attr: 'A' },
  { ext: '.pak', min: 512 * 1024, max: 128 * 1024 * 1024, attr: 'RA' },
  { ext: '.vhdx', min: 64 * 1024 * 1024, max: 1024 * 1024 * 1024, attr: 'A' },
  { ext: '.tar.gz', min: 256 * 1024, max: 96 * 1024 * 1024, attr: 'A' },
  { ext: '.sqlite', min: 16 * 1024, max: 24 * 1024 * 1024, attr: 'A' },
];

const ANCHOR_PATHS = [
  // Safe cleanup / quarantine targets
  { dir: 'C:\\Users\\Developer\\AppData\\Local\\Temp', attr: 'H', files: [
    { name: 'wds-crash-dump-202611.dmp', size: 268_435_456, attr: 'A' },
    { name: 'vite-optimize-cache.bin', size: 67_108_864, attr: 'A' },
    { name: 'npm-debug-archive.log', size: 33_554_432, attr: 'A' },
  ] },
  { dir: 'C:\\Users\\Developer\\AppData\\Local\\npm-cache', attr: '', files: [
    { name: '_cacache-index-v5.bin', size: 134_217_728, attr: 'A' },
  ] },
  { dir: 'C:\\Users\\Developer\\Projects\\WinDirStat-LiquidGlass\\node_modules', attr: '', files: [
    { name: 'typescript-bundle.js', size: 18_874_368, attr: 'A' },
  ] },
  { dir: 'C:\\Users\\Developer\\Projects\\WinDirStat-LiquidGlass\\src-tauri\\target\\debug', attr: '', files: [
    { name: 'win_dir_stat_ng.pdb', size: 188_743_680, attr: 'A' },
  ] },
  { dir: 'C:\\ProgramData\\Docker\\windowsfilter', attr: 'S', files: [
    { name: 'layer-base-win2025.vhdx', size: 1_073_741_824, attr: 'SA' },
  ] },
  { dir: 'C:\\Users\\Developer\\Downloads', attr: '', files: [
    { name: 'ubuntu-24.04-desktop-amd64.iso', size: 2_147_483_648, attr: 'A' },
    { name: 'llvm-19.1.0-win64-installer.exe', size: 402_653_184, attr: 'A' },
  ] },
  // Protected OS paths (CRITICAL_DO_NOT_DELETE targets)
  { dir: 'C:\\Windows\\System32', attr: 'S', files: [
    { name: 'ntoskrnl.exe', size: 12_582_912, attr: 'RSA' },
    { name: 'kernel32.dll', size: 1_572_864, attr: 'RSA' },
  ] },
  { dir: 'C:\\Windows\\WinSxS', attr: 'S', files: [
    { name: 'amd64_microsoft-windows-coreos.manifest', size: 4_194_304, attr: 'RSA' },
  ] },
  { dir: 'C:\\Program Files\\Microsoft Visual Studio\\2026\\BuildTools', attr: '', files: [
    { name: 'MSBuild.exe', size: 2_097_152, attr: 'RA' },
  ] },
];

const CATEGORY_STEMS = [
  'cache', 'build', 'dist', 'assets', 'logs', 'chunks', 'artifacts',
  'packages', 'modules', 'snapshots', 'shaders', 'textures', 'backups',
  'telemetry', 'indexes', 'blobs', 'toolchains', 'recordings',
];

/**
 * Build a deterministic synthetic disk tree with exact node count `count`.
 * Output entries are ordered parent-before-child and conform to WinDirStat `/SaveTo` JSON.
 *
 * @param {{ count?: number, seed?: number, drive?: string }} [options]
 * @returns {Array<Record<string, any>>}
 */
export function generateSaveToTree(options = {}) {
  const count = Number.isInteger(options.count) ? options.count : DEFAULT_NODE_COUNT;
  const seed = Number.isInteger(options.seed) ? options.seed : DEFAULT_SEED;
  const drive = (options.drive || 'C:\\').endsWith('\\') ? (options.drive || 'C:\\') : `${options.drive}\\`;

  if (count < 2) {
    throw new Error(`Node count must be >= 2 (got ${count})`);
  }

  const rng = createPrng(seed);
  let nextIndex = 1n;

  /** @type {Map<string, any>} */
  const dirByPath = new Map();
  /** @type {Array<any>} */
  const orderedNodes = [];

  const rootNode = {
    name: drive,
    parentPath: null,
    isRoot: true,
    isDir: true,
    files: 0,
    folders: 0,
    sizeLogical: 0,
    sizePhysical: 0,
    attributes: 'D',
    lastChange: deterministicTimestamp(rng),
    wdsAttr: WDS_ATTR_DRIVE_ROOT,
    index: formatHex64(nextIndex++),
  };
  dirByPath.set(drive, rootNode);
  orderedNodes.push(rootNode);

  function ensureDirChain(fullDirPath, dirAttr = '') {
    const normalized = fullDirPath.replace(/\\+$/, '');
    const drivePrefix = drive.replace(/\\+$/, '');
    if (normalized.toLowerCase() === drivePrefix.toLowerCase()) {
      return rootNode;
    }

    const existing = dirByPath.get(normalized);
    if (existing) return existing;

    // Split relative segments beneath drive (e.g. C:\Users\Developer)
    const rest = normalized.slice(drivePrefix.length + 1);
    const segments = rest.split('\\').filter(Boolean);
    let currentPath = drivePrefix;
    let parent = rootNode;

    for (let i = 0; i < segments.length; i++) {
      const seg = segments[i];
      currentPath = `${currentPath}\\${seg}`;
      let dirNode = dirByPath.get(currentPath);
      if (!dirNode) {
        if (orderedNodes.length >= count) {
          return parent;
        }
        dirNode = {
          name: currentPath,
          parentPath: parent.name,
          isRoot: false,
          isDir: true,
          files: 0,
          folders: 0,
          sizeLogical: 0,
          sizePhysical: 0,
          attributes: i === segments.length - 1 ? dirAttr : '',
          lastChange: deterministicTimestamp(rng),
          wdsAttr: WDS_ATTR_DIRECTORY,
          index: formatHex64(nextIndex++),
        };
        dirByPath.set(currentPath, dirNode);
        orderedNodes.push(dirNode);
      }
      parent = dirNode;
    }
    return parent;
  }

  function addFileNode(parentDir, fileName, sizeLogical, attr) {
    if (orderedNodes.length >= count) return false;
    const prefix = parentDir.name.endsWith('\\') ? parentDir.name.slice(0, -1) : parentDir.name;
    const filePath = `${prefix}\\${fileName}`;
    const sizePhysical = alignCluster4K(sizeLogical);
    const node = {
      name: filePath,
      parentPath: parentDir.name,
      isRoot: false,
      isDir: false,
      files: 0,
      folders: 0,
      sizeLogical,
      sizePhysical,
      attributes: attr,
      lastChange: deterministicTimestamp(rng),
      wdsAttr: WDS_ATTR_FILE,
      index: formatHex64(nextIndex++),
    };
    orderedNodes.push(node);
    return true;
  }

  // 1. Seed realistic anchor paths (for small and large trees within count budget)
  const dirPool = [rootNode];
  for (const anchor of ANCHOR_PATHS) {
    if (orderedNodes.length >= count - 1) break;
    const relativeAnchor = anchor.dir.replace(/^[A-Za-z]:\\/, drive);
    const dirNode = ensureDirChain(relativeAnchor, anchor.attr);
    if (!dirPool.includes(dirNode)) dirPool.push(dirNode);
    for (const f of anchor.files) {
      if (orderedNodes.length >= count) break;
      addFileNode(dirNode, f.name, f.size, f.attr);
    }
  }

  // Collect all created directories so far into dirPool
  for (const node of orderedNodes) {
    if (node.isDir && !dirPool.includes(node)) {
      dirPool.push(node);
    }
  }

  // 2. Determine target directory ratio (~14% directories, ~86% files) to reach exact `count`
  const targetTotalDirs = Math.max(dirPool.length, Math.min(count - 1, Math.floor(count * 0.14)));
  let dirSeq = 1;
  while (dirPool.length < targetTotalDirs && orderedNodes.length < count) {
    // Bias toward moderate depths (levels 2..6) for rich treemap hierarchy
    const parent = dirPool[Math.floor(rng() * dirPool.length)];
    const stem = pick(rng, CATEGORY_STEMS);
    const suffix = String(dirSeq++).padStart(5, '0');
    const prefix = parent.name.endsWith('\\') ? parent.name.slice(0, -1) : parent.name;
    const newDirPath = `${prefix}\\${stem}_${suffix}`;
    if (!dirByPath.has(newDirPath)) {
      const dirNode = {
        name: newDirPath,
        parentPath: parent.name,
        isRoot: false,
        isDir: true,
        files: 0,
        folders: 0,
        sizeLogical: 0,
        sizePhysical: 0,
        attributes: rng() < 0.08 ? 'H' : '',
        lastChange: deterministicTimestamp(rng),
        wdsAttr: WDS_ATTR_DIRECTORY,
        index: formatHex64(nextIndex++),
      };
      dirByPath.set(newDirPath, dirNode);
      orderedNodes.push(dirNode);
      dirPool.push(dirNode);
    }
  }

  // 3. Fill remaining slots with deterministic files spread across dirPool
  let fileSeq = 1;
  while (orderedNodes.length < count) {
    // Weighted directory choice: favor leafier / higher-index project/cache dirs for realistic hotspots
    const idx = rng() < 0.65
      ? randomInt(rng, Math.floor(dirPool.length * 0.2), dirPool.length - 1)
      : randomInt(rng, 0, dirPool.length - 1);
    const parent = dirPool[idx] || rootNode;
    const profile = pick(rng, FILE_PROFILES);
    // Skew file sizes logarithmically so a few large blobs and many small files produce realistic squarified treemaps
    const u = rng();
    const logMin = Math.log(profile.min);
    const logMax = Math.log(profile.max);
    const sizeLogical = Math.max(256, Math.round(Math.exp(logMin + (u * u) * (logMax - logMin))));
    const fileName = `item_${String(fileSeq++).padStart(6, '0')}${profile.ext}`;
    addFileNode(parent, fileName, sizeLogical, profile.attr);
  }

  // 4. Bottom-up recursive aggregation (orderedNodes has parents before children)
  for (let i = orderedNodes.length - 1; i >= 1; i--) {
    const node = orderedNodes[i];
    const parent = dirByPath.get(node.parentPath);
    if (!parent) continue;
    if (node.isDir) {
      parent.folders += 1 + node.folders;
      parent.files += node.files;
      parent.sizeLogical += node.sizeLogical;
      parent.sizePhysical += node.sizePhysical;
    } else {
      parent.files += 1;
      parent.sizeLogical += node.sizeLogical;
      parent.sizePhysical += node.sizePhysical;
    }
  }

  // 5. Materialize exact WinDirStat `/SaveTo` JSON objects
  return orderedNodes.map((n) => ({
    Name: n.name,
    Files: n.files,
    Folders: n.folders,
    'Logical Size': n.sizeLogical,
    'Physical Size': n.sizePhysical,
    Attributes: n.attributes,
    'Last Change': n.lastChange,
    'WinDirStat Attributes': n.wdsAttr,
    Index: n.index,
    ScanOutcome: 'complete',
    ScanError: '',
  }));
}

/**
 * Convert flat `/SaveTo` items array into a nested tree object for UI/WebGL consumption.
 * @param {Array<Record<string, any>>} items
 */
export function buildNestedTreeFromSaveTo(items) {
  if (!Array.isArray(items) || items.length === 0) {
    throw new Error('Cannot build nested tree from empty items array');
  }
  const byPath = new Map();
  let root = null;

  for (const item of items) {
    const wdsAttr = Number.parseInt(item['WinDirStat Attributes'], 16);
    const isDir = (wdsAttr & (ITEM_TYPE.IT_DRIVE | ITEM_TYPE.IT_DIRECTORY)) !== 0;
    const node = {
      id: item.Index,
      name: item.Name === 'C:\\' || /^[A-Za-z]:\\$/.test(item.Name)
        ? item.Name
        : item.Name.slice(item.Name.lastIndexOf('\\') + 1),
      path: item.Name,
      kind: isDir ? 'directory' : 'file',
      sizeLogical: item['Logical Size'],
      sizePhysical: item['Physical Size'],
      filesCount: item.Files,
      foldersCount: item.Folders,
      lastChange: item['Last Change'],
      attributes: item.Attributes,
      wdsAttributes: item['WinDirStat Attributes'],
      ...(isDir ? { children: [] } : {}),
    };

    if (root === null) {
      root = node;
    } else {
      const sep = item.Name.lastIndexOf('\\');
      let parentKey = sep >= 0 ? item.Name.slice(0, sep) : '';
      if (/^[A-Za-z]:$/.test(parentKey)) parentKey = `${parentKey}\\`;
      const parent = byPath.get(parentKey);
      if (!parent || !parent.children) {
        throw new Error(`Parent not found for ${item.Name} (expected ${parentKey})`);
      }
      parent.children.push(node);
    }

    if (isDir) {
      byPath.set(item.Name, node);
    }
  }
  return root;
}

/**
 * Format `/SaveTo` items using the exact line-by-line JSON layout expected by
 * `windirstat/CsvLoader.cpp` (`SaveResultsJson` / `JsonReadObject`).
 *
 * @param {Array<Record<string, any>>} items
 * @returns {string}
 */
export function formatSaveToJson(items) {
  const chunks = ['[\r\n'];
  const BATCH_SIZE = 2000;
  let batch = [];

  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    const comma = i + 1 < items.length ? ',' : '';
    batch.push(
      `{\r\n` +
      `  "Name": ${JSON.stringify(item.Name)},\r\n` +
      `  "Files": ${item.Files},\r\n` +
      `  "Folders": ${item.Folders},\r\n` +
      `  "Logical Size": ${item['Logical Size']},\r\n` +
      `  "Physical Size": ${item['Physical Size']},\r\n` +
      `  "Attributes": ${JSON.stringify(item.Attributes)},\r\n` +
      `  "Last Change": ${JSON.stringify(item['Last Change'])},\r\n` +
      `  "WinDirStat Attributes": ${JSON.stringify(item['WinDirStat Attributes'])},\r\n` +
      `  "Index": ${JSON.stringify(item.Index)},\r\n` +
      `  "ScanOutcome": ${JSON.stringify(item.ScanOutcome)},\r\n` +
      `  "ScanError": ${JSON.stringify(item.ScanError)}\r\n` +
      `}${comma}\r\n`
    );
    if (batch.length >= BATCH_SIZE) {
      chunks.push(batch.join(''));
      batch = [];
    }
  }
  if (batch.length > 0) {
    chunks.push(batch.join(''));
  }
  chunks.push(']\r\n');
  return chunks.join('');
}

/**
 * Validate structural, rollup, and `/SaveTo` schema invariants.
 * @param {Array<Record<string, any>>} items
 * @param {number} expectedCount
 */
export function verifySyntheticTree(items, expectedCount) {
  if (!Array.isArray(items) || items.length !== expectedCount) {
    throw new Error(`Expected ${expectedCount} nodes, got ${Array.isArray(items) ? items.length : 'non-array'}`);
  }

  const requiredKeys = [
    'Name',
    'Files',
    'Folders',
    'Logical Size',
    'Physical Size',
    'Attributes',
    'Last Change',
    'WinDirStat Attributes',
    'Index',
    'ScanOutcome',
    'ScanError',
  ];

  const seenDirs = new Set();
  const childTotals = new Map();

  for (let i = 0; i < items.length; i++) {
    const item = items[i];
    for (const key of requiredKeys) {
      if (!(key in item)) {
        throw new Error(`Node[${i}] missing required /SaveTo key "${key}"`);
      }
    }

    const wdsAttr = Number.parseInt(item['WinDirStat Attributes'], 16);
    const isRoot = (wdsAttr & ITEM_TYPE.ITF_ROOTITEM) !== 0;
    const isDir = (wdsAttr & (ITEM_TYPE.IT_DRIVE | ITEM_TYPE.IT_DIRECTORY)) !== 0;
    const isFile = (wdsAttr & ITEM_TYPE.IT_FILE) !== 0;

    if (i === 0) {
      if (!isRoot || !isDir) {
        throw new Error(`Root item at index 0 must have ITF_ROOTITEM and container type`);
      }
    } else {
      if (isRoot) {
        throw new Error(`Unexpected root flag on non-root item[${i}]: ${item.Name}`);
      }
      const sep = item.Name.lastIndexOf('\\');
      let parentKey = sep >= 0 ? item.Name.slice(0, sep) : '';
      if (/^[A-Za-z]:$/.test(parentKey)) parentKey = `${parentKey}\\`;
      if (!seenDirs.has(parentKey)) {
        throw new Error(`Parent "${parentKey}" not seen before child "${item.Name}" at index ${i}`);
      }

      const acc = childTotals.get(parentKey);
      if (isDir) {
        acc.directFolders += 1;
      } else if (isFile) {
        acc.directFiles += 1;
        acc.directLogical += item['Logical Size'];
        acc.directPhysical += item['Physical Size'];
      }
    }

    if (isDir) {
      seenDirs.add(item.Name);
      childTotals.set(item.Name, {
        directFiles: 0,
        directFolders: 0,
        directLogical: 0,
        directPhysical: 0,
        totalFiles: 0,
        totalFolders: 0,
        totalLogical: 0,
        totalPhysical: 0,
      });
    } else if (item.Files !== 0 || item.Folders !== 0) {
      throw new Error(`Leaf file "${item.Name}" must report Files=0 and Folders=0`);
    }
  }

  // Bottom-up verification of cumulative rollups
  for (let i = items.length - 1; i >= 0; i--) {
    const item = items[i];
    const wdsAttr = Number.parseInt(item['WinDirStat Attributes'], 16);
    const isDir = (wdsAttr & (ITEM_TYPE.IT_DRIVE | ITEM_TYPE.IT_DIRECTORY)) !== 0;
    if (!isDir) continue;

    const acc = childTotals.get(item.Name);
    acc.totalFiles += acc.directFiles;
    acc.totalFolders += acc.directFolders;
    acc.totalLogical += acc.directLogical;
    acc.totalPhysical += acc.directPhysical;

    if (
      item.Files !== acc.totalFiles ||
      item.Folders !== acc.totalFolders ||
      item['Logical Size'] !== acc.totalLogical ||
      item['Physical Size'] !== acc.totalPhysical
    ) {
      throw new Error(
        `Rollup mismatch on "${item.Name}": expected (files=${acc.totalFiles}, folders=${acc.totalFolders}, ` +
        `logical=${acc.totalLogical}, physical=${acc.totalPhysical}) but found ` +
        `(files=${item.Files}, folders=${item.Folders}, logical=${item['Logical Size']}, physical=${item['Physical Size']})`
      );
    }

    if (i > 0) {
      const sep = item.Name.lastIndexOf('\\');
      let parentKey = sep >= 0 ? item.Name.slice(0, sep) : '';
      if (/^[A-Za-z]:$/.test(parentKey)) parentKey = `${parentKey}\\`;
      const parentAcc = childTotals.get(parentKey);
      parentAcc.totalFiles += acc.totalFiles;
      parentAcc.totalFolders += acc.totalFolders;
      parentAcc.totalLogical += acc.totalLogical;
      parentAcc.totalPhysical += acc.totalPhysical;
    }
  }

  const root = items[0];
  if (1 + root.Files + root.Folders !== expectedCount) {
    throw new Error(
      `Root accounting mismatch: 1 + Files(${root.Files}) + Folders(${root.Folders}) != ${expectedCount}`
    );
  }
}

function parseArgs(argv) {
  const args = {
    count: DEFAULT_NODE_COUNT,
    seed: DEFAULT_SEED,
    format: 'saveto',
    out: null,
    verify: false,
    quiet: false,
  };

  for (let i = 0; i < argv.length; i++) {
    const token = argv[i];
    if (token === '--verify') {
      args.verify = true;
    } else if (token === '--quiet') {
      args.quiet = true;
    } else if (token === '--count' && i + 1 < argv.length) {
      args.count = Number.parseInt(argv[++i], 10);
    } else if (token === '--seed' && i + 1 < argv.length) {
      args.seed = Number.parseInt(argv[++i], 10);
    } else if (token === '--format' && i + 1 < argv.length) {
      args.format = argv[++i];
    } else if (token === '--out' && i + 1 < argv.length) {
      args.out = argv[++i];
    } else {
      throw new Error(`Unknown or incomplete argument: ${token}`);
    }
  }
  return args;
}

function runVerification() {
  const startedAt = performance.now();

  // 1. Verify 1,000-node tree determinism & nested conversion
  const smallA = generateSaveToTree({ count: 1000, seed: DEFAULT_SEED });
  const smallB = generateSaveToTree({ count: 1000, seed: DEFAULT_SEED });
  verifySyntheticTree(smallA, 1000);
  const hashSmallA = crypto.createHash('sha256').update(formatSaveToJson(smallA)).digest('hex');
  const hashSmallB = crypto.createHash('sha256').update(formatSaveToJson(smallB)).digest('hex');
  if (hashSmallA !== hashSmallB) {
    throw new Error('Determinism check failed on 1,000-node tree');
  }
  const nestedSmall = buildNestedTreeFromSaveTo(smallA);
  if (!nestedSmall || !Array.isArray(nestedSmall.children) || nestedSmall.children.length === 0) {
    throw new Error('Nested tree conversion failed on 1,000-node tree');
  }

  // 2. Verify full 100,000-node tree generation, rollups, and /SaveTo JSON parse roundtrip
  const largeA = generateSaveToTree({ count: 100_000, seed: DEFAULT_SEED });
  const largeB = generateSaveToTree({ count: 100_000, seed: DEFAULT_SEED });
  verifySyntheticTree(largeA, 100_000);

  const json100k = formatSaveToJson(largeA);
  const json100kRepeat = formatSaveToJson(largeB);
  const hash100kA = crypto.createHash('sha256').update(json100k).digest('hex');
  const hash100kB = crypto.createHash('sha256').update(json100kRepeat).digest('hex');
  if (hash100kA !== hash100kB) {
    throw new Error('Determinism check failed on 100,000-node tree');
  }

  // Confirm strict JSON.parse round-trip AND line-by-line JsonReadObject compatibility
  const parsed = JSON.parse(json100k);
  if (parsed.length !== 100_000) {
    throw new Error(`Parsed JSON length mismatch: ${parsed.length}`);
  }
  const firstLines = json100k.slice(0, 600).split(/\r?\n/);
  if (firstLines[0] !== '[' || firstLines[1] !== '{') {
    throw new Error('Output does not start with line-by-line WinDirStat /SaveTo JSON layout');
  }

  const elapsedMs = Math.round(performance.now() - startedAt);
  const root = largeA[0];
  console.log(
    JSON.stringify(
      {
        status: 'PASS',
        verifiedNodes: largeA.length,
        files: root.Files,
        folders: root.Folders,
        logicalBytes: root['Logical Size'],
        physicalBytes: root['Physical Size'],
        jsonBytes: Buffer.byteLength(json100k, 'utf8'),
        sha256: hash100kA,
        elapsedMs,
      },
      null,
      2
    )
  );
}

function main() {
  const args = parseArgs(process.argv.slice(2));
  if (args.verify) {
    runVerification();
    return;
  }

  const hereDir = path.dirname(fileURLToPath(import.meta.url));
  const outPath = args.out
    ? path.resolve(args.out)
    : path.join(hereDir, `synthetic-tree-${args.count === 100_000 ? '100k' : args.count}.json`);

  const items = generateSaveToTree({ count: args.count, seed: args.seed });
  verifySyntheticTree(items, args.count);

  let serialized;
  if (args.format === 'nested') {
    serialized = JSON.stringify(buildNestedTreeFromSaveTo(items), null, 2);
  } else if (args.format === 'saveto') {
    serialized = formatSaveToJson(items);
  } else {
    throw new Error(`Unsupported format "${args.format}". Use "saveto" or "nested".`);
  }

  fs.mkdirSync(path.dirname(outPath), { recursive: true });
  fs.writeFileSync(outPath, serialized, 'utf8');

  if (!args.quiet) {
    const root = items[0];
    console.log(
      `[SyntheticTree] Wrote ${items.length} nodes (${root.Files} files, ${root.Folders} folders, ` +
      `${(Buffer.byteLength(serialized, 'utf8') / (1024 * 1024)).toFixed(2)} MiB) -> ${outPath}`
    );
  }
}

const isEntrypoint = process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url));
if (isEntrypoint) {
  main();
}
