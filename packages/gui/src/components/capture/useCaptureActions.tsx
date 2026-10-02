import { useCallback, useMemo } from "react";
import { AppConfig, RequestLogEntry } from "@/types";
import { blockKey, findBlocksFolder, ensureBlocksFolderId, buildBlockMock, blockedKeySet } from "@/lib/blocks";

export function useCaptureActions({
  wsConfig,
  reloadConfig,
}: {
  wsConfig: AppConfig;
  reloadConfig: () => Promise<void>;
}) {
  const blockedKeys = useMemo(() => blockedKeySet(wsConfig), [wsConfig.mocks, wsConfig.mockFolders]);

  /**
   * The two builders are the **engine's** now (`plan/19` WI 5, item 2 — `capture.buildMock` /
   * `capture.buildRequest`). This hook used to call `buildMockInitial` / `reqToHeadersBody` from
   * `./captureUtils`; those decided which headers to drop, how to decode a body and how a response is
   * stored, which is engine work, and `@bifurc/gui` cannot import the engine.
   *
   * **The field lists below are unchanged on purpose.** `capture.buildMock` returns the whole `mocks` body
   * — captured headers and body included — but this path has always stored only the response side plus the
   * method/url, so spreading the payload would silently start persisting the captured request too. The
   * picking is the caller's, and it stays exactly as it was.
   */
  const mockEntries = useCallback(async (list: RequestLogEntry[], inFolder: boolean) => {
    if (list.length === 0) return;
    let folderId: string | null = null;
    if (inFolder) {
      const folderName = `Capture ${new Date().toLocaleString().replace(/[/:]/g, "-")}`;
      const folder = await window.api.addFolder("mock", { name: folderName, parentId: null });
      folderId = folder.id;
    }
    for (const entry of list) {
      const { mock } = await window.api.captureBuildMock(entry);
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
      const { request } = await window.api.captureBuildRequest(entry);
      await window.api.addRequest({ ...request, name: `${entry.method} ${entry.url}`, folderId } as any);
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
