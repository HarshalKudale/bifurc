export type EntityKind =
  | "workspace"
  | "requests"
  | "mocks"
  | "environments"
  | "mappings"
  | "proxyRules"
  | "websockets"
  | "webhooks";

export type CollisionStrategy = "keep" | "override" | "new";

export interface FormatDefinition {
  id: string;
  label: string;
  /** File extensions accepted/produced (without leading dot) */
  extensions: string[];
  supportsExport: boolean;
  supportsImport: boolean;
}

export interface ExportRequest {
  kind: EntityKind;
  format: string;
  wsId: string;
}

export interface ExportResult {
  ok: boolean;
  filePath?: string;
  error?: string;
}

export interface PreflightRequest {
  kind: EntityKind;
  format: string;
  wsId: string;
}

export interface PreflightResult {
  ok: boolean;
  /** Absolute path to the file the user picked */
  filePath?: string;
  itemCount?: number;
  collisionIds?: string[];
  error?: string;
  canceled?: boolean;
}

export interface ImportRequest {
  kind: EntityKind;
  format: string;
  wsId: string;
  filePath: string;
  collisionStrategy: CollisionStrategy;
}

export interface ImportResult {
  ok: boolean;
  imported?: number;
  skipped?: number;
  error?: string;
}

export interface FormatsMap {
  [kind: string]: FormatDefinition[];
}
