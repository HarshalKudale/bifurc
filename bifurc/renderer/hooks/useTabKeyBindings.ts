import { useEffect } from "react";
import type { PersistableTabHandle } from "@/hooks/useEntityTabs";

interface Options {
  activeTab: string | null;
  tabRefs: React.MutableRefObject<Record<string, PersistableTabHandle | null>>;
  closeTab: (id: string) => void;
  openNewTab: () => void;
}

/**
 * Registers panel-level keyboard shortcuts:
 *   Ctrl+S  — save the active tab
 *   Ctrl+W  — close the active tab
 *   Ctrl+T  — open a new tab
 *
 * Ctrl+F is handled natively by CodeMirror's searchKeymap when the editor is focused.
 */
export function useTabKeyBindings({ activeTab, tabRefs, closeTab, openNewTab }: Options) {
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (!e.ctrlKey && !e.metaKey) return;

      if (e.key === "s" || e.key === "S") {
        if (!activeTab) return;
        const ref = tabRefs.current[activeTab];
        if (!ref?.save) return;
        e.preventDefault();
        ref.save();
        return;
      }

      if (e.key === "w" || e.key === "W") {
        if (!activeTab) return;
        e.preventDefault();
        closeTab(activeTab);
        return;
      }

      if (e.key === "t" || e.key === "T") {
        e.preventDefault();
        openNewTab();
        return;
      }
    };

    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [activeTab, tabRefs, closeTab, openNewTab]);
}
