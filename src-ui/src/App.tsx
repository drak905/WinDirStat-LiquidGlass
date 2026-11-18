import { useCallback, useEffect, useState } from 'react';
import { createDemoDrives, createDemoDuplicates, createDemoSnapshot, createInitialVaultItems } from './data/demoSnapshot';
import { getBridge, isTauriRuntime } from './services/tauriBridge';
import type {
  AgentAdvisorResponse,
  AgentProvider,
  AgentProviderStatus,
  DriveInfo,
  DuplicateGroup,
  NavTabId,
  ScanSnapshot,
  StorageNode,
  VaultItem,
} from './types/storage';
import { formatBytes } from './utils/formatters';
import { LiquidGlassSvgFilters } from './components/LiquidGlassSvgFilters';
import { TitleBar } from './components/TitleBar';
import { GlassDock } from './components/GlassDock';
import { TreemapView } from './components/TreemapView';
import { SunburstView } from './components/SunburstView';
import { FlamegraphView } from './components/FlamegraphView';
import { DuplicatesView } from './components/DuplicatesView';
import { AgentAdvisorPanel } from './components/AgentAdvisorPanel';
import { VaultManagerPanel } from './components/VaultManagerPanel';
import { ContextMenu, type ContextMenuTarget } from './components/ContextMenu';

interface ContextState {
  x: number;
  y: number;
  target: ContextMenuTarget | null;
}

export function App() {
  const [darkMode, setDarkMode] = useState(true);
  const [drives, setDrives] = useState<DriveInfo[]>(() => createDemoDrives());
  const [selectedDrive, setSelectedDrive] = useState<string>('C:\\');
  const [snapshot, setSnapshot] = useState<ScanSnapshot>(() => createDemoSnapshot('C:\\'));
  const [duplicates, setDuplicates] = useState<DuplicateGroup[]>(() => createDemoDuplicates());
  const [vaultItems, setVaultItems] = useState<VaultItem[]>(() => createInitialVaultItems());
  const [providers, setProviders] = useState<AgentProviderStatus[]>([]);
  const [selectedProvider, setSelectedProvider] = useState<AgentProvider>('antigravity');
  const [advisorResponses, setAdvisorResponses] = useState<AgentAdvisorResponse[]>([]);
  const [isStreamingAdvice, setIsStreamingAdvice] = useState(false);
  const [isScanning, setIsScanning] = useState(false);
  const [activeTab, setActiveTab] = useState<NavTabId>('treemap');
  const [contextMenu, setContextMenu] = useState<ContextState>({ x: 0, y: 0, target: null });
  const [externalZoomTarget, setExternalZoomTarget] = useState<StorageNode | null>(null);
  const [toastMessage, setToastMessage] = useState<string | null>(null);

  const findNodeByPath = useCallback((node: StorageNode, targetPath: string): StorageNode | null => {
    if (node.path === targetPath) return node;
    if (!node.children) return null;
    for (const child of node.children) {
      const hit = findNodeByPath(child, targetPath);
      if (hit) return hit;
    }
    return null;
  }, []);

  const showToast = useCallback((msg: string) => {
    setToastMessage(msg);
  }, []);

  useEffect(() => {
    if (!toastMessage) return;
    const timer = setTimeout(() => setToastMessage(null), 4000);
    return () => clearTimeout(timer);
  }, [toastMessage]);

  useEffect(() => {
    const rootEl = document.documentElement;
    if (darkMode) rootEl.classList.add('dark');
    else rootEl.classList.remove('dark');
  }, [darkMode]);

  // Hydrate initial state via Tauri IPC bridge or browser simulation
  useEffect(() => {
    let active = true;
    const bridge = getBridge();
    void (async () => {
      const [driveList, detectedClis, vaultList, dupList] = await Promise.all([
        bridge.listDrives(),
        bridge.detectAgentClis(),
        bridge.listVaultItems(),
        bridge.findDuplicates('C:\\'),
      ]);
      if (!active) return;
      setDrives(driveList);
      setProviders(detectedClis);
      setVaultItems(vaultList);
      setDuplicates(dupList);
    })();
    return () => {
      active = false;
    };
  }, []);

  const handleScan = useCallback(
    async (overrideTarget?: string) => {
      const target = overrideTarget ?? selectedDrive;
      if (overrideTarget) setSelectedDrive(overrideTarget);
      setIsScanning(true);
      try {
        const bridge = getBridge();
        const [newSnap, newDups] = await Promise.all([
          bridge.scanDirectory(target),
          bridge.findDuplicates(target),
        ]);
        setSnapshot(newSnap);
        setDuplicates(newDups);
        showToast(
          `Scanned ${newSnap.totalNodes.toLocaleString()} nodes (${formatBytes(newSnap.root.size)}) in ${newSnap.scanDurationMs}ms`
        );
      } finally {
        setIsScanning(false);
      }
    },
    [selectedDrive, showToast]
  );

  const handleAskAgent = useCallback(
    async (params: {
      provider: AgentProvider;
      prompt: string;
      targetPath?: string;
      targetName?: string;
      targetSizeBytes?: number;
      customCommand?: string;
    }) => {
      setIsStreamingAdvice(true);
      try {
        const bridge = getBridge();
        const response = await bridge.askAgentAdvisor(params);
        setAdvisorResponses((prev) => [response, ...prev]);
        return response;
      } finally {
        setIsStreamingAdvice(false);
      }
    },
    []
  );

  const handleQuarantine = useCallback(
    async (params: { path: string; name: string; sizeBytes: number }) => {
      const bridge = getBridge();
      const item = await bridge.quarantineItem(params);
      setVaultItems((prev) => [item, ...prev.filter((v) => v.id !== item.id)]);
      const refreshedDups = await bridge.findDuplicates(selectedDrive);
      setDuplicates(refreshedDups);
      showToast(`Quarantined "${item.fileName}" with 168h rollback protection.`);
    },
    [selectedDrive, showToast]
  );

  const handleRollback = useCallback(
    async (id: string) => {
      const bridge = getBridge();
      const restored = await bridge.rollbackVaultItem(id);
      setVaultItems((prev) => prev.map((v) => (v.id === id ? restored : v)));
      showToast(`Rolled back "${restored.fileName}" to ${restored.originalPath}`);
    },
    [showToast]
  );

  const handlePurge = useCallback(
    async (id: string) => {
      const bridge = getBridge();
      await bridge.purgeVaultItem(id);
      setVaultItems((prev) => prev.filter((v) => v.id !== id));
      showToast('Permanently purged quarantined item.');
    },
    [showToast]
  );

  const handleOpenContextMenu = useCallback(
    (x: number, y: number, target: ContextMenuTarget) => {
      setContextMenu({ x, y, target });
    },
    []
  );

  const activeVaultCount = vaultItems.filter((v) => v.status === 'quarantined').length;
  const inTauri = isTauriRuntime();

  return (
    <div
      className={`relative flex h-screen w-screen flex-col gap-3 p-3 text-slate-900 dark:text-slate-100 ${
        inTauri ? 'bg-transparent' : 'browser-ambient-backdrop'
      }`}
    >
      <LiquidGlassSvgFilters />

      <TitleBar
        darkMode={darkMode}
        onToggleTheme={() => setDarkMode((d) => !d)}
        activeDrive={snapshot.driveId}
        scanStatusText={`${snapshot.totalNodes.toLocaleString()} nodes • ${formatBytes(snapshot.root.size)}`}
      />

      <div className="flex min-h-0 flex-1 gap-3">
        <GlassDock
          drives={drives}
          selectedDrive={selectedDrive}
          onSelectDrive={(id) => {
            setSelectedDrive(id);
            void handleScan(id);
          }}
          onTriggerScan={handleScan}
          isScanning={isScanning}
          activeTab={activeTab}
          onSelectTab={setActiveTab}
          duplicateCount={duplicates.length}
          vaultCount={activeVaultCount}
          totalScannedBytes={snapshot.root.size}
          totalNodeCount={snapshot.totalNodes}
        />

        <main className="min-w-0 flex-1">
          {activeTab === 'treemap' && (
            <TreemapView
              root={snapshot.root}
              extensions={snapshot.extensions}
              onContextMenuTarget={handleOpenContextMenu}
              externalZoomTarget={externalZoomTarget}
            />
          )}
          {activeTab === 'sunburst' && (
            <SunburstView
              root={snapshot.root}
              onContextMenuNode={handleOpenContextMenu}
            />
          )}
          {activeTab === 'flamegraph' && (
            <FlamegraphView
              root={snapshot.root}
              onContextMenuNode={handleOpenContextMenu}
            />
          )}
          {activeTab === 'duplicates' && (
            <DuplicatesView
              groups={duplicates}
              onQuarantineFile={handleQuarantine}
              onAskAgent={(target) => {
                setActiveTab('advisor');
                void handleAskAgent({
                  provider: selectedProvider,
                  prompt: `Should I delete duplicate file ${target.path}?`,
                  targetPath: target.path,
                  targetName: target.name,
                  targetSizeBytes: target.sizeBytes,
                });
              }}
            />
          )}
          {activeTab === 'advisor' && (
            <AgentAdvisorPanel
              providers={providers}
              selectedProvider={selectedProvider}
              onProviderChange={setSelectedProvider}
              responses={advisorResponses}
              isStreaming={isStreamingAdvice}
              onSendPrompt={handleAskAgent}
              onQuarantineRecommended={(resp) =>
                handleQuarantine({
                  path: resp.targetPath,
                  name: resp.targetName,
                  sizeBytes: resp.targetSizeBytes,
                })
              }
            />
          )}
          {activeTab === 'vault' && (
            <VaultManagerPanel
              items={vaultItems}
              onRollback={handleRollback}
              onPurge={handlePurge}
              onRefresh={async () => {
                const list = await getBridge().listVaultItems();
                setVaultItems(list);
              }}
            />
          )}
        </main>
      </div>

      <ContextMenu
        x={contextMenu.x}
        y={contextMenu.y}
        target={contextMenu.target}
        onClose={() => setContextMenu((prev) => ({ ...prev, target: null }))}
        onAskAgent={(target) => {
          setActiveTab('advisor');
          void handleAskAgent({
            provider: selectedProvider,
            prompt: `Should I delete or keep ${target.path}?`,
            targetPath: target.path,
            targetName: target.name,
            targetSizeBytes: target.sizeBytes,
          });
        }}
        onQuarantine={(target) => {
          void handleQuarantine({
            path: target.path,
            name: target.name,
            sizeBytes: target.sizeBytes,
          });
        }}
        onZoomIn={(target) => {
          const found = findNodeByPath(snapshot.root, target.path);
          if (found && found.isDir) {
            setActiveTab('treemap');
            setExternalZoomTarget(found);
          }
        }}
      />

      {toastMessage && (
        <div
          role="status"
          className="liquid-glass liquid-spring-enter fixed bottom-5 right-5 z-50 rounded-2xl px-4 py-2.5 text-xs font-semibold text-slate-900 dark:text-white"
        >
          {toastMessage}
        </div>
      )}
    </div>
  );
}

export default App;
