import { useCallback, useMemo } from "react";
import { AppConfig, RequestLogEntry } from "@/types";
import { blockKey, findBlocksFolder, ensureBlocksFolderId, buildBlockMock, blockedKeySet } from "@/lib/blocks";
import { buildMockInitial, reqToHeadersBody } from "./captureUtils";

export function useCaptureActions({
  wsConfig,
  reloadConfig,
}: {
  wsConfig: AppConfig;
  reloadConfig: () => Promise<void>;
}) {
  const blockedKeys = useMemo(() => blockedKeySet(wsConfig), [wsConfig.mocks, wsConfig.mockFolders]);

  const mockEntries = useCallback(async (list: RequestLogEntry[], inFolder: boolean) => {
    if (list.length === 0) return;
    let folderId: string | null = null;
    if (inFolder) {
      const folderName = `Capture ${new Date().toLocaleString().replace(/[/:]/g, "-")}`;
      const folder = await window.api.addFolder("mock", { name: folderName, parentId: null });
      folderId = folder.id;
    }
    for (const entry of list) {
      const mock = buildMockInitial(entry);
      await window.api.addMock({
        name: `${entry.method} ${entry.url}`,
        method: mock.method ?? "GET",
        urlPattern: mock.urlPattern ?? entry.url,
        useRegex: mock.useRegex ?? false,
        responseStatus: mock.responseStatus ?? 200,
        responseHeaders: mock.responseHeaders ?? {},
        responseBody: mock.responseBody ?? "",
        responseBodyEncoding: mock.responseBodyEncoding,
        enabled: true,
        folderId,
      } as any);
    }
    window.api.getConfig();
  }, []);

  const saveEntries = useCallback(async (list: RequestLogEntry[], inFolder: boolean) => {
    if (list.length === 0) return;
    let folderId: string | null = null;
    if (inFolder) {
      const folderName = `Capture ${new Date().toLocaleString().replace(/[/:]/g, "-")}`;
      const folder = await window.api.addFolder("request", { name: folderName, parentId: null });
      folderId = folder.id;
    }
    for (const entry of list) {
      await window.api.addRequest({ ...reqToHeadersBody(entry), name: `${entry.method} ${entry.url}`, folderId } as any);
    }
    window.api.getConfig();
  }, []);

  const blockEntries = useCallback(async (list: RequestLogEntry[]) => {
    if (list.length === 0) return;
    const folderId = await ensureBlocksFolderId(wsConfig.mockFolders ?? []);
    const existing = blockedKeySet(wsConfig);
    for (const entry of list) {
      if (existing.has(blockKey(entry.method, entry.url))) continue;
      await window.api.addMock(buildBlockMock(entry.method, entry.url, folderId) as any);
    }
    await reloadConfig();
  }, [wsConfig, reloadConfig]);

  const unblockEntries = useCallback(async (list: RequestLogEntry[]) => {
    const blocks = findBlocksFolder(wsConfig.mockFolders ?? []);
    if (!blocks) return;
    const keys = new Set(list.map((e) => blockKey(e.method, e.url)));
    const toDelete = (wsConfig.mocks ?? []).filter(
      (m) => m.folderId === blocks.id && keys.has(blockKey(m.method, m.urlPattern)),
    );
    for (const m of toDelete) await window.api.deleteMock(m.id);
    await reloadConfig();
  }, [wsConfig, reloadConfig]);

  const shareEntries = useCallback((list: RequestLogEntry[]) => {
    if (list.length === 0) return;
    const name = list.length === 1
      ? `capture-${list[0].method}-${list[0].id}.json`
      : `captured-requests-${list.length}.json`;
    window.api.shareCaptureJson(list, name);
  }, []);

  return { blockedKeys, mockEntries, saveEntries, blockEntries, unblockEntries, shareEntries };
}
