import { useState, useCallback } from "react";
import { Panel, PANEL_REGISTRY, ALWAYS_VISIBLE_PANELS } from "@/lib/panelRegistry";
import { readStorage, writeStorage } from "@/lib/storage";

const STORAGE_KEY = "sidebar-visibility";

function getStoredVisibility(): Record<Panel, boolean> {
    const stored = readStorage<Record<Panel, boolean> | null>(STORAGE_KEY, null);
    if (stored) return stored;
    // Default: all panels visible
    return Object.fromEntries(
        PANEL_REGISTRY.filter((e) => e.enabled && e.showInSidebar !== false)
            .map((e) => [e.id, true])
    ) as Record<Panel, boolean>;
}

function saveVisibility(v: Record<Panel, boolean>) {
    writeStorage(STORAGE_KEY, v);
}

export function useSidebarVisibility() {
    const [visibility, setVisibility] = useState<Record<Panel, boolean>>(getStoredVisibility);

    const setPanelVisible = useCallback((id: Panel, visible: boolean) => {
        if (ALWAYS_VISIBLE_PANELS.includes(id)) return; // Cannot hide always-visible panels
        setVisibility((prev) => {
            const next = { ...prev, [id]: visible };
            saveVisibility(next);
            return next;
        });
    }, []);

    const isPanelVisible = useCallback((id: Panel): boolean => {
        if (ALWAYS_VISIBLE_PANELS.includes(id)) return true;
        return visibility[id] !== false; // default to visible
    }, [visibility]);

    return { visibility, setPanelVisible, isPanelVisible };
}
