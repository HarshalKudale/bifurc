import React, { useState, useEffect, useCallback, useRef } from "react";
import { usePersistedState } from "@/lib/usePersistedState";
import { clearDraft, getDraftIds, loadDraft } from "@/lib/useDraftPersist";
import type { SavedRequest, MockRule } from "@/types";

export interface PersistableTabHandle {
  refresh?: (entity: unknown) => void;
  save?: () => void;
}

interface Options<T> {
  storageKey: string;
  draftPrefix: string;
  extraDraftPrefixes?: string[];
  workspaceId: string;
  entityKind: "mocks" | "requests" | "rules" | "runners" | string;
  resolveEntityKind?: (id: string) => string;
  entities: T[];
}

interface EntityTabsResult<T> {
  openTabs: string[];
  activeTab: string | null;
  setActiveTab: React.Dispatch<React.SetStateAction<string | null>>;
  loadedEntities: Record<string, T>;
  setLoadedEntities: React.Dispatch<React.SetStateAction<Record<string, T>>>;
  tabRefs: React.MutableRefObject<Record<string, PersistableTabHandle | null>>;
  isDraft: (id: string) => boolean;
  openTab: (id: string) => void;
  openNewTab: () => void;
  closeTab: (tabId: string) => void;
  closeOtherTabs: (keepId: string) => void;
  closeAllTabs: () => void;
  /** Replace a draft tab id with the newly saved entity id. */
  replaceTab: (draftId: string, savedId: string) => void;
  /** Reorder open tabs by dragging fromId to toId position. */
  reorderTabs: (fromId: string, toId: string, edge?: "left" | "right") => void;
}

export function useEntityTabs<T extends { id: string }>({
  storageKey,
  draftPrefix,
  extraDraftPrefixes = [],
  workspaceId,
  entityKind,
  resolveEntityKind,
  entities,
}: Options<T>): EntityTabsResult<T> {
  const isDraft = useCallback(
    (id: string) => id.startsWith(draftPrefix) || extraDraftPrefixes.some((p) => id.startsWith(p)),
    [draftPrefix, extraDraftPrefixes],
  );

  const [openTabs, setOpenTabs] = usePersistedState<string[]>(
    `${storageKey}:openTabs`, [],
    (tabs) => tabs.filter((id) => {
      if (isDraft(id)) return getDraftIds(draftPrefix).includes(id) || extraDraftPrefixes.some((p) => id.startsWith(p));
      // When entities haven't loaded yet, keep all persisted tabs to avoid premature eviction
      if (entities.length === 0) return true;
      return entities.some((e) => e.id === id);
    }),
  );

  const [activeTab, setActiveTab] = usePersistedState<string | null>(
    `${storageKey}:activeTab`, null,
    (id) => {
      if (id === null) return null;
      if (isDraft(id)) return getDraftIds(draftPrefix).includes(id) || extraDraftPrefixes.some((p) => id.startsWith(p)) ? id : null;
      // Same: keep the active tab when entities haven't loaded yet
      if (entities.length === 0) return id;
      return entities.some((e) => e.id === id) ? id : null;
    },
  );

  // Once entities load, prune any tabs that no longer exist
  useEffect(() => {
    if (entities.length === 0) return;
    setOpenTabs((prev) => prev.filter((id) => {
      if (isDraft(id)) return true;
      return entities.some((e) => e.id === id);
    }));
    setActiveTab((cur) => {
      if (cur === null || isDraft(cur)) return cur;
      return entities.some((e) => e.id === cur) ? cur : null;
    });
  }, [entities.length]);

  const [loadedEntities, setLoadedEntities] = useState<Record<string, T>>({});
  const tabRefs = useRef<Record<string, PersistableTabHandle | null>>({});

  useEffect(() => {
    if (!activeTab || isDraft(activeTab)) return;
    if (loadedEntities[activeTab]) return;
    const kind = resolveEntityKind ? resolveEntityKind(activeTab) : entityKind;
    window.api.loadEntity(workspaceId, kind, activeTab).then((res) => {
      if (res.ok && res.entity) {
        const entity = res.entity as T;
        setLoadedEntities((prev) => ({ ...prev, [activeTab]: entity }));
        tabRefs.current[activeTab]?.refresh?.(entity as unknown as MockRule | SavedRequest);
      }
    }).catch(() => {});
  }, [activeTab, workspaceId, resolveEntityKind, entityKind]);

  const openTab = useCallback((id: string) => {
    setOpenTabs((prev) => (prev.includes(id) ? prev : [...prev, id]));
    setActiveTab(id);
  }, []);

  useEffect(() => {
    const handler = (e: Event) => {
      const custom = e as CustomEvent<{ panel: string; tabId: string }>;
      if (!custom.detail?.tabId) return;
      const targetPanel = custom.detail.panel;
      if (
        targetPanel === storageKey ||
        targetPanel === entityKind ||
        (storageKey === "requests" && targetPanel.startsWith("req")) ||
        (storageKey === "mocks" && targetPanel.startsWith("mock"))
      ) {
        openTab(custom.detail.tabId);
      }
    };
    window.addEventListener("localpanel:open-tab", handler);
    return () => window.removeEventListener("localpanel:open-tab", handler);
  }, [storageKey, entityKind, openTab]);

  const openNewTab = useCallback(() => {
    const existingEmpty = openTabs.find((id) => isDraft(id) && !id.startsWith(extraDraftPrefixes[0] ?? "__none__") && !loadDraft(id));
    if (existingEmpty) { setActiveTab(existingEmpty); return; }
    const tabId = `${draftPrefix}${Date.now()}`;
    setOpenTabs((p) => [...p, tabId]);
    setActiveTab(tabId);
  }, [openTabs, draftPrefix]);

  const closeTab = useCallback((tabId: string) => {
    if (isDraft(tabId)) clearDraft(tabId);
    delete tabRefs.current[tabId];
    setOpenTabs((prev) => {
      const next = prev.filter((id) => id !== tabId);
      setActiveTab((cur) => {
        if (cur !== tabId) return cur;
        return next.length > 0 ? next[next.length - 1] : null;
      });
      return next;
    });
  }, []);

  const closeOtherTabs = useCallback((keepId: string) => {
    setOpenTabs((prev) => {
      const toClose = prev.filter((id) => id !== keepId);
      toClose.forEach((id) => { if (isDraft(id)) clearDraft(id); delete tabRefs.current[id]; });
      setActiveTab(keepId);
      return [keepId];
    });
  }, [isDraft]);

  const closeAllTabs = useCallback(() => {
    setOpenTabs((prev) => {
      prev.forEach((id) => { if (isDraft(id)) clearDraft(id); delete tabRefs.current[id]; });
      setActiveTab(null);
      return [];
    });
  }, [isDraft]);

  const replaceTab = useCallback((draftId: string, savedId: string) => {
    if (isDraft(draftId)) clearDraft(draftId);
    delete tabRefs.current[draftId];
    setOpenTabs((prev) => [...prev.filter((id) => id !== draftId), savedId]);
    setActiveTab(savedId);
  }, []);

  const reorderTabs = useCallback((fromId: string, toId: string, edge: "left" | "right" = "left") => {
    if (fromId === toId) return;
    setOpenTabs((prev) => {
      const fromIndex = prev.indexOf(fromId);
      if (fromIndex === -1) return prev;
      const withoutFrom = prev.filter((id) => id !== fromId);
      let targetIndex = withoutFrom.indexOf(toId);
      if (targetIndex === -1) return prev;
      if (edge === "right") {
        targetIndex += 1;
      }
      const next = [...withoutFrom];
      next.splice(targetIndex, 0, fromId);
      return next;
    });
  }, []);

  return {
    openTabs,
    activeTab,
    setActiveTab,
    loadedEntities,
    setLoadedEntities,
    tabRefs,
    isDraft,
    openTab,
    openNewTab,
    closeTab,
    closeOtherTabs,
    closeAllTabs,
    replaceTab,
    reorderTabs,
  };
}
