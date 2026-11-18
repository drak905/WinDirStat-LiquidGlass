import type {
  DriveInfo,
  DuplicateGroup,
  ExtensionSummary,
  ScanSnapshot,
  StorageNode,
  VaultItem,
} from '../types/storage';
import { getExtensionColor } from '../utils/formatters';

export function createDemoDrives(): DriveInfo[] {
  return [
    {
      id: 'C:\\',
      label: 'C:\\ System NVMe (Demo Scan)',
      mountPoint: 'C:\\',
      totalBytes: 1_000_000_000_000,
      usedBytes: 642_500_000_000,
      freeBytes: 357_500_000_000,
      kind: 'nvme',
    },
    {
      id: 'D:\\',
      label: 'D:\\ Dev & Media Array',
      mountPoint: 'D:\\',
      totalBytes: 2_000_000_000_000,
      usedBytes: 1_180_000_000_000,
      freeBytes: 820_000_000_000,
      kind: 'ssd',
    },
    {
      id: 'demo://100k-snapshot',
      label: '100k Node Benchmark Snapshot',
      mountPoint: 'demo://100k-snapshot',
      totalBytes: 2_400_000_000_000,
      usedBytes: 1_650_000_000_000,
      freeBytes: 750_000_000_000,
      kind: 'snapshot',
    },
  ];
}

function makeFile(
  id: string,
  name: string,
  parentPath: string,
  size: number,
  extension: string,
  modifiedAt = '2026-11-12T08:15:00Z'
): StorageNode {
  return {
    id,
    name,
    path: `${parentPath}\\${name}`,
    size,
    isDir: false,
    extension,
    fileCount: 1,
    modifiedAt,
  };
}

function makeDir(id: string, name: string, parentPath: string, children: StorageNode[]): StorageNode {
  let size = 0;
  let fileCount = 0;
  for (const child of children) {
    size += child.size;
    fileCount += child.fileCount;
  }
  children.sort((a, b) => b.size - a.size);
  return {
    id,
    name,
    path: parentPath ? `${parentPath}\\${name}` : name,
    size,
    isDir: true,
    fileCount,
    children,
  };
}

export function summarizeExtensions(root: StorageNode): ExtensionSummary[] {
  const map = new Map<string, { totalBytes: number; fileCount: number; samplePath: string }>();
  const stack: StorageNode[] = [root];

  while (stack.length > 0) {
    const cur = stack.pop()!;
    if (cur.isDir && cur.children) {
      for (const child of cur.children) {
        stack.push(child);
      }
    } else if (!cur.isDir) {
      const ext = (cur.extension || 'other').toLowerCase();
      const prev = map.get(ext);
      if (prev) {
        prev.totalBytes += cur.size;
        prev.fileCount += 1;
      } else {
        map.set(ext, {
          totalBytes: cur.size,
          fileCount: 1,
          samplePath: cur.path,
        });
      }
    }
  }

  const total = Math.max(1, root.size);
  return Array.from(map.entries())
    .map(([extension, stats]) => ({
      extension,
      color: getExtensionColor(extension),
      totalBytes: stats.totalBytes,
      fileCount: stats.fileCount,
      percent: (stats.totalBytes / total) * 100,
      samplePath: stats.samplePath,
    }))
    .sort((a, b) => b.totalBytes - a.totalBytes);
}

export function createDemoSnapshot(driveId = 'C:\\'): ScanSnapshot {
  const rootPath = driveId.endsWith('\\') ? driveId.slice(0, -1) : driveId;

  const usersDir = makeDir('dir-Users', 'Users', rootPath, [
    makeDir('dir-thinh', 'thinh', `${rootPath}\\Users`, [
      makeDir('dir-Downloads', 'Downloads', `${rootPath}\\Users\\thinh`, [
        makeFile('f-iso-1', 'ubuntu-24.04.1-desktop-amd64.iso', `${rootPath}\\Users\\thinh\\Downloads`, 6_120_000_000, '.iso'),
        makeFile('f-iso-2', 'Win11_24H2_English_x64.iso', `${rootPath}\\Users\\thinh\\Downloads`, 5_840_000_000, '.iso'),
        makeFile('f-zip-1', 'old-workspace-backup-2025.zip', `${rootPath}\\Users\\thinh\\Downloads`, 14_200_000_000, '.zip'),
        makeFile('f-mp4-1', 'keynote-capture-4k.mp4', `${rootPath}\\Users\\thinh\\Downloads`, 8_950_000_000, '.mp4'),
      ]),
      makeDir('dir-Projects', 'Projects', `${rootPath}\\Users\\thinh`, [
        makeDir('dir-node-modules', 'node_modules', `${rootPath}\\Users\\thinh\\Projects`, [
          makeFile('f-nm-1', 'typescript-bundle.js.bin', `${rootPath}\\Users\\thinh\\Projects\\node_modules`, 2_450_000_000, '.bin'),
          makeFile('f-nm-2', 'playwright-chromium.zip', `${rootPath}\\Users\\thinh\\Projects\\node_modules`, 1_880_000_000, '.zip'),
          makeFile('f-nm-3', 'esbuild-cache.tmp', `${rootPath}\\Users\\thinh\\Projects\\node_modules`, 1_340_000_000, '.tmp'),
        ]),
        makeDir('dir-target', 'target-rust', `${rootPath}\\Users\\thinh\\Projects`, [
          makeFile('f-rs-1', 'tauri_debug_symbols.pdb', `${rootPath}\\Users\\thinh\\Projects\\target-rust`, 19_400_000_000, '.pdb'),
          makeFile('f-rs-2', 'incremental-artifacts.bin', `${rootPath}\\Users\\thinh\\Projects\\target-rust`, 11_700_000_000, '.bin'),
          makeFile('f-rs-3', 'main.rs', `${rootPath}\\Users\\thinh\\Projects\\target-rust`, 420_000_000, '.rs'),
        ]),
      ]),
      makeDir('dir-Videos', 'Videos', `${rootPath}\\Users\\thinh`, [
        makeFile('f-vid-1', 'gameplay-raw-hdr.mkv', `${rootPath}\\Users\\thinh\\Videos`, 28_600_000_000, '.mkv'),
        makeFile('f-vid-2', 'studio-broll-master.mov', `${rootPath}\\Users\\thinh\\Videos`, 18_300_000_000, '.mov'),
      ]),
    ]),
  ]);

  const windowsDir = makeDir('dir-Windows', 'Windows', rootPath, [
    makeDir('dir-WinTemp', 'Temp', `${rootPath}\\Windows`, [
      makeFile('f-tmp-1', 'installer-cache-bundle.tmp', `${rootPath}\\Windows\\Temp`, 9_400_000_000, '.tmp'),
      makeFile('f-log-1', 'dism-update-trace.log', `${rootPath}\\Windows\\Temp`, 4_850_000_000, '.log'),
      makeFile('f-log-2', 'cbs-archive-rollover.log', `${rootPath}\\Windows\\Temp`, 3_720_000_000, '.log'),
    ]),
    makeDir('dir-System32', 'System32', `${rootPath}\\Windows`, [
      makeFile('f-sys-1', 'ntoskrnl.exe', `${rootPath}\\Windows\\System32`, 14_500_000_000, '.exe'),
      makeFile('f-sys-2', 'kernelbase.dll', `${rootPath}\\Windows\\System32`, 11_200_000_000, '.dll'),
      makeFile('f-sys-3', 'dwmcore.dll', `${rootPath}\\Windows\\System32`, 7_800_000_000, '.dll'),
    ]),
  ]);

  const programDataDir = makeDir('dir-ProgramData', 'ProgramData', rootPath, [
    makeDir('dir-Docker', 'DockerDesktop', `${rootPath}\\ProgramData`, [
      makeFile('f-vhdx-1', 'docker_data.vhdx', `${rootPath}\\ProgramData\\DockerDesktop`, 46_200_000_000, '.vhdx'),
      makeFile('f-log-3', 'wsl-engine.log', `${rootPath}\\ProgramData\\DockerDesktop`, 2_900_000_000, '.log'),
    ]),
    makeDir('dir-Telemetry', 'PackageCache', `${rootPath}\\ProgramData`, [
      makeFile('f-cache-1', 'vs_community_offline.cache', `${rootPath}\\ProgramData\\PackageCache`, 12_400_000_000, '.cache'),
      makeFile('f-db-1', 'search_index.sqlite', `${rootPath}\\ProgramData\\PackageCache`, 6_450_000_000, '.sqlite'),
    ]),
  ]);

  const root = makeDir('dir-root', driveId, '', [programDataDir, usersDir, windowsDir]);

  return {
    driveId,
    root,
    totalNodes: root.fileCount + 12,
    scanDurationMs: 142,
    extensions: summarizeExtensions(root),
  };
}

/**
 * Generates >= 100,000 realistic tree nodes deterministically in <80ms for benchmarking
 * squarified layout culling and 60fps cushion treemap rendering.
 */
export function generateSynthetic100kTree(): ScanSnapshot {
  const extensions = ['.bin', '.iso', '.mp4', '.log', '.tmp', '.zip', '.pdb', '.dll', '.ts', '.rs', '.cache'];
  const topFolders: StorageNode[] = [];
  let totalNodes = 1; // root

  const topCount = 20;
  const subCount = 25;
  const filesPerSub = 200; // 20 * 25 * 200 = 100,000 leaf files + 521 dirs = 100,521 nodes!

  for (let t = 0; t < topCount; t += 1) {
    const topName = `VolumeCluster_${String(t).padStart(2, '0')}`;
    const topPath = `demo://100k-snapshot\\${topName}`;
    const subDirs: StorageNode[] = [];
    let topSize = 0;
    let topFileCount = 0;
    // Steep Zipf-like weight so large clusters and tiny sub-pixel clusters coexist realistically
    const clusterWeight = Math.round(1_000_000_000 / Math.pow(t + 1, 1.35));

    for (let s = 0; s < subCount; s += 1) {
      const subName = `Shard_${String(s).padStart(2, '0')}`;
      const subPath = `${topPath}\\${subName}`;
      const files: StorageNode[] = new Array(filesPerSub);
      let subSize = 0;
      const subWeight = Math.max(10_000, Math.round(clusterWeight / Math.pow(s + 1, 1.25)));

      for (let f = 0; f < filesPerSub; f += 1) {
        const ext = extensions[(t + s + f) % extensions.length];
        const size = Math.max(4096, Math.round(subWeight / (f + 1)));
        const name = `artifact_${t}_${s}_${f}${ext}`;
        files[f] = {
          id: `n-${t}-${s}-${f}`,
          name,
          path: `${subPath}\\${name}`,
          size,
          isDir: false,
          extension: ext,
          fileCount: 1,
        };
        subSize += size;
      }

      totalNodes += 1 + filesPerSub;
      topSize += subSize;
      topFileCount += filesPerSub;
      subDirs.push({
        id: `d-${t}-${s}`,
        name: subName,
        path: subPath,
        size: subSize,
        isDir: true,
        fileCount: filesPerSub,
        children: files,
      });
    }

    subDirs.sort((a, b) => b.size - a.size);
    totalNodes += 1;
    topFolders.push({
      id: `d-${t}`,
      name: topName,
      path: topPath,
      size: topSize,
      isDir: true,
      fileCount: topFileCount,
      children: subDirs,
    });
  }

  topFolders.sort((a, b) => b.size - a.size);
  const totalSize = topFolders.reduce((acc, d) => acc + d.size, 0);
  const totalFiles = topFolders.reduce((acc, d) => acc + d.fileCount, 0);

  const root: StorageNode = {
    id: 'root-100k',
    name: 'demo://100k-snapshot',
    path: 'demo://100k-snapshot',
    size: totalSize,
    isDir: true,
    fileCount: totalFiles,
    children: topFolders,
  };

  const extSummary: ExtensionSummary[] = extensions.map((ext, idx) => ({
    extension: ext,
    color: getExtensionColor(ext),
    totalBytes: Math.round(totalSize / (idx + 2)),
    fileCount: Math.round(totalFiles / extensions.length),
    percent: Number((100 / extensions.length).toFixed(1)),
    samplePath: `${root.path}\\VolumeCluster_00\\Shard_00\\artifact_0_0_${idx}${ext}`,
  }));

  return {
    driveId: 'demo://100k-snapshot',
    root,
    totalNodes,
    scanDurationMs: 68,
    extensions: extSummary,
  };
}

export function createDemoDuplicates(): DuplicateGroup[] {
  return [
    {
      id: 'dup-grp-1',
      hash: 'a8f5f167f44f4964e6c998dee827110c',
      hashAlgorithm: 'xxHash64+SHA256',
      fileSize: 6_120_000_000,
      wastedBytes: 12_240_000_000,
      extension: '.iso',
      files: [
        {
          id: 'dup-1a',
          name: 'ubuntu-24.04.1-desktop-amd64.iso',
          path: 'C:\\Users\\thinh\\Downloads\\ubuntu-24.04.1-desktop-amd64.iso',
          modifiedAt: '2026-10-14T11:20:00Z',
        },
        {
          id: 'dup-1b',
          name: 'ubuntu-24.04.1-desktop-amd64 (1).iso',
          path: 'D:\\Archives\\ISOs\\ubuntu-24.04.1-desktop-amd64 (1).iso',
          modifiedAt: '2026-11-02T15:40:00Z',
        },
        {
          id: 'dup-1c',
          name: 'ubuntu-desktop-copy.iso',
          path: 'C:\\Users\\thinh\\Desktop\\ubuntu-desktop-copy.iso',
          modifiedAt: '2026-11-09T09:05:00Z',
        },
      ],
    },
    {
      id: 'dup-grp-2',
      hash: '7c4a8d09ca3762af61e59520943dc264',
      hashAlgorithm: 'xxHash64+SHA256',
      fileSize: 4_850_000_000,
      wastedBytes: 4_850_000_000,
      extension: '.zip',
      files: [
        {
          id: 'dup-2a',
          name: 'sdk-assets-snapshot.zip',
          path: 'C:\\Users\\thinh\\Downloads\\sdk-assets-snapshot.zip',
          modifiedAt: '2026-09-21T16:10:00Z',
        },
        {
          id: 'dup-2b',
          name: 'sdk-assets-snapshot-backup.zip',
          path: 'D:\\Backups\\sdk-assets-snapshot-backup.zip',
          modifiedAt: '2026-10-03T18:45:00Z',
        },
      ],
    },
  ];
}

export function createInitialVaultItems(): VaultItem[] {
  return [
    {
      id: 'vault-seed-1',
      batchId: 'batch-20261117-a1',
      originalPath: 'C:\\Windows\\Temp\\stale-crash-dump-2026.dmp',
      fileName: 'stale-crash-dump-2026.dmp',
      sizeBytes: 4_280_000_000,
      quarantinedAt: '2026-11-17T14:20:00Z',
      expiresAt: '2026-11-24T14:20:00Z',
      remainingHours: 142.4,
      sha256: '9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08',
      xxhash64: 'd8a2f74c1e9043b2',
      hashVerified: true,
      status: 'quarantined',
    },
    {
      id: 'vault-seed-2',
      batchId: 'batch-20261118-b4',
      originalPath: 'C:\\Users\\thinh\\Downloads\\legacy-node-cache.tar',
      fileName: 'legacy-node-cache.tar',
      sizeBytes: 2_650_000_000,
      quarantinedAt: '2026-11-18T11:00:00Z',
      expiresAt: '2026-11-25T11:00:00Z',
      remainingHours: 165.2,
      sha256: 'e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855',
      xxhash64: '4c9e1a0b73f2881d',
      hashVerified: true,
      status: 'quarantined',
    },
  ];
}
