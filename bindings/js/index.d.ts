/**
 * Byte range span within a `.tmt` document.
 */
export interface Span {
  start: number;
  end: number;
}

/**
 * Details of an unresolved `@conflict(...)` element.
 */
export interface ConflictInfo {
  index: number;
  kind: 'block' | 'inline' | 'scalar';
  /** Range of BYTE offsets into the UTF-8 source (not UTF-16 string indices). */
  span: Span;
  a: string;
  b: string;
}

/**
 * Warning produced during a save operation.
 */
export type SaveWarning = {
  code: 'ModifyDeleteConflict';
  path: string;
  message: string;
};

/**
 * Result returned upon a successful save.
 */
export interface SaveResult {
  revision: string;
  hasConflicts: boolean;
  warnings: SaveWarning[];
}

/**
 * Content of a file read from a host.
 */
export interface ReadFileResult {
  content: string;
  revision: string;
}

/**
 * Configuration options for initializing `GitHubCms`.
 */
export interface GitHubCmsOptions {
  owner: string;
  repo: string;
  branch?: string;
  token: string;
  baseUrl?: string;
  fetch?: typeof globalThis.fetch;
}

/**
 * Request payload for creating a new document.
 */
export interface CreateFileRequest {
  path: string;
  content: string;
  message: string;
  maxRetries?: number;
}

/**
 * Request payload for editing an existing document.
 */
export interface EditFileRequest {
  path: string;
  baseRev: string;
  content: string;
  message: string;
  maxRetries?: number;
}

/**
 * High-level content management client for GitHub repositories.
 */
export class GitHubCms {
  constructor(options: GitHubCmsOptions);

  /**
   * Fetches the target branch's current head commit revision OID.
   */
  getHead(): Promise<string>;

  /**
   * Reads a file at `path` from revision `rev`.
   * Returns `{ content, revision }` if present, or `null` if absent.
   */
  readFile(path: string, rev: string): Promise<ReadFileResult | null>;

  /**
   * Creates a fresh document at `path`.
   * Rejects if the file already exists at HEAD (with `err.code === "AlreadyExists"`).
   */
  createFile(req: CreateFileRequest): Promise<SaveResult>;

  /**
   * Edits an existing document at `path` against `baseRev`.
   * Performs 3-way AST merge and automatic retries on branch movement.
   */
  editFile(req: EditFileRequest): Promise<SaveResult>;
}

/**
 * Finds and lists all `@conflict` markers in `.tmt` source.
 */
export function listConflicts(source: string): ConflictInfo[];

/**
 * Resolves a single `@conflict` marker at `index` to the chosen `side` ('a' or 'b').
 */
export function resolveConflict(source: string, index: number, side: 'a' | 'b'): string;

/**
 * Asynchronously initializes the WebAssembly module.
 */
export function init(moduleOrPath?: string | URL | Request | BufferSource | WebAssembly.Module): Promise<void>;

/**
 * Synchronously initializes the WebAssembly module with bytes or compiled module.
 */
export function initSync(module?: BufferSource | WebAssembly.Module): void;

export default init;
