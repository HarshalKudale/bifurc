import { AppConfig, MockRule, Folder } from "@/types";

// Application-managed blocks are 403 mocks living in a single root-level folder
// named "Blocks". They are not user-editable - they exist only for visibility and
// can be toggled on/off from the Capture panel or the Mocks panel context menu.
export const BLOCKS_FOLDER_NAME = "Blocks";

export function findBlocksFolder(folders: Folder[]): Folder | undefined {
  return folders.find((f) => f.parentId === null && f.name === BLOCKS_FOLDER_NAME);
}

/** A mock is a block iff it lives in the Blocks folder. */
export function isBlockMock(mock: MockRule, folders: Folder[]): boolean {
  const blocks = findBlocksFolder(folders);
  return !!blocks && mock.folderId === blocks.id;
}

/** Stable key for matching a captured request against a block mock. */
export function blockKey(method: string, url: string): string {
  return `${method.toUpperCase()}|${url}`;
}

/** Set of block keys currently active, derived from the mocks in the Blocks folder. */
export function blockedKeySet(config: Pick<AppConfig, "mocks" | "mockFolders">): Set<string> {
  const blocks = findBlocksFolder(config.mockFolders ?? []);
  const keys = new Set<string>();
  if (!blocks) return keys;
  for (const m of config.mocks ?? []) {
    if (m.folderId === blocks.id) keys.add(blockKey(m.method, m.urlPattern));
  }
  return keys;
}

/** Return the Blocks folder id, creating the folder if it doesn't exist yet. */
export async function ensureBlocksFolderId(folders: Folder[]): Promise<string> {
  const existing = findBlocksFolder(folders);
  if (existing) return existing.id;
  const folder = await window.api.addFolder("mock", { name: BLOCKS_FOLDER_NAME, parentId: null });
  return folder.id;
}

/** Build the 403 block-mock payload for a given method+url. */
export function buildBlockMock(method: string, url: string, folderId: string): Omit<MockRule, "id" | "createdAt" | "workspaceId"> {
  return {
    name: `Block ${method} ${url}`,
    method,
    urlPattern: url,
    useRegex: false,
    capturedHeaders: {},
    capturedBody: "",
    responseStatus: 403,
    responseHeaders: {},
    responseBody: "",
    enabled: true,
    folderId,
  };
}
