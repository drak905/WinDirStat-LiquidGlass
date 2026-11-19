export type NavTabId =
  | 'treemap'
  | 'sunburst'
  | 'flamegraph'
  | 'duplicates'
  | 'advisor'
  | 'vault';

export interface StorageNode {
  id: string;
  name: string;
  path: string;
  size: number;
  isDir: boolean;
  extension?: string;
  fileCount: number;
  modifiedAt?: string;
  hash?: string;
  children?: StorageNode[];
}

export interface ExtensionSummary {
  extension: string;
  color: string;
  totalBytes: number;
  fileCount: number;
  percent: number;
  samplePath: string;
}

export interface DriveInfo {
  id: string;
  label: string;
  mountPoint: string;
  totalBytes: number;
  usedBytes: number;
  freeBytes: number;
  kind: 'nvme' | 'ssd' | 'hdd' | 'network' | 'snapshot';
}

export interface DuplicateFileEntry {
  id: string;
  name: string;
  path: string;
  modifiedAt: string;
}

export interface DuplicateGroup {
  id: string;
  hash: string;
  hashAlgorithm: 'xxHash64+SHA256' | 'SHA-256';
  fileSize: number;
  wastedBytes: number;
  extension: string;
  files: DuplicateFileEntry[];
}

export type AgentProvider =
  | 'antigravity'
  | 'kiro'
  | 'claude'
  | 'codex'
  | 'opencode'
  | 'custom';

export interface AgentProviderStatus {
  provider: AgentProvider;
  displayName: string;
  available: boolean;
  command: string;
  latencyMs?: number;
  resolvedPath?: string | null;
  installHint?: string;
}

export type AgentVerdict =
  | 'SAFE_TO_DELETE'
  | 'QUARANTINE_RECOMMENDED'
  | 'REVIEW_CAREFULLY'
  | 'KEEP';

export interface AgentSourceReference {
  title: string;
  url: string;
  domain: string;
  verified: boolean;
  snippet: string;
}

export interface AgentAdvisorResponse {
  id: string;
  targetPath: string;
  targetName: string;
  targetSizeBytes: number;
  provider: AgentProvider;
  verdict: AgentVerdict;
  risk_score: number;
  reclaimable_bytes: number;
  usage_value_assessment: string;
  rationale: string;
  sources: AgentSourceReference[];
  createdAt: string;
  userPrompt?: string;
  responseMode?: 'verdict' | 'chat' | 'error';
  assistantReply?: string;
  isLiveCli?: boolean;
  resolvedCommand?: string;
  durationMs?: number;
  error?: string;
  installHint?: string;
}

export interface VaultItem {
  id: string;
  batchId: string;
  originalPath: string;
  fileName: string;
  sizeBytes: number;
  quarantinedAt: string;
  expiresAt: string;
  remainingHours: number;
  sha256: string;
  xxhash64: string;
  hashVerified: boolean;
  status: 'quarantined' | 'restored' | 'purged';
}

export interface ScanSnapshot {
  driveId: string;
  root: StorageNode;
  totalNodes: number;
  scanDurationMs: number;
  extensions: ExtensionSummary[];
}

export interface AskAgentAdvisorParams {
  provider: AgentProvider;
  prompt: string;
  targetPath?: string;
  targetName?: string;
  targetSizeBytes?: number;
  customCommand?: string;
  activeDrive?: string;
  model?: string;
  scanSummary?: string;
}

export interface AgentCliTestResult {
  ok: boolean;
  command?: string;
  output?: string;
  error?: string;
  durationMs?: number;
}

export interface QuarantineItemParams {
  path: string;
  name: string;
  sizeBytes: number;
  batchId?: string;
}

export interface WdsBridge {
  listDrives(): Promise<DriveInfo[]>;
  scanDirectory(target: string): Promise<ScanSnapshot>;
  findDuplicates(target: string): Promise<DuplicateGroup[]>;
  detectAgentClis(): Promise<AgentProviderStatus[]>;
  askAgentAdvisor(params: AskAgentAdvisorParams): Promise<AgentAdvisorResponse>;
  listOpenCodeModels?(): Promise<string[]>;
  testAgentCli?(provider: AgentProvider, customCommand?: string): Promise<AgentCliTestResult>;
  quarantineItem(params: QuarantineItemParams): Promise<VaultItem>;
  listVaultItems(): Promise<VaultItem[]>;
  rollbackVaultItem(id: string): Promise<VaultItem>;
  purgeVaultItem(id: string): Promise<void>;
}

declare global {
  interface Window {
    __WDS_TEST_BRIDGE__?: Partial<WdsBridge>;
    __TAURI_INTERNALS__?: unknown;
  }
}
