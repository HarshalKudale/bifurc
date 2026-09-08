import React, { useMemo } from "react";
import TabBar from "@/components/editor/TabBar";
import { strings } from "@/lib/strings";
import { methodColor, methodBg } from "@/lib/utils";

interface Props {
  openTabs: string[];
  activeTab: string | null;
  dirtyTabs: Record<string, boolean>;
  tabBadge: (id: string) => string;
  tabLabel: (id: string) => string;
  isDraft: (id: string) => boolean;
  setActiveTab: (id: string) => void;
  closeTab: (id: string) => void;
  openNewTabInFolder: () => void;
  reorderTabs: (oldI: number, newI: number) => void;
  closeOtherTabs: (id: string) => void;
  closeAllTabs: () => void;
  handleDuplicateImpl: (id: string) => void;
}

export default function MockTabs({
  openTabs,
  activeTab,
  dirtyTabs,
  tabBadge,
  tabLabel,
  isDraft,
  setActiveTab,
  closeTab,
  openNewTabInFolder,
  reorderTabs,
  closeOtherTabs,
  closeAllTabs,
  handleDuplicateImpl,
}: Props) {
  const tabsItems = useMemo(() => {
    return openTabs.map((id) => {
      const badge = tabBadge(id);
      const color = methodColor(badge);
      const bg = methodBg(badge);
      return {
        id,
        label: tabLabel(id),
        isDraft: isDraft(id),
        isModified: dirtyTabs[id],
        renderTab: () => (
          <div className="flex items-center gap-1.5 min-w-0">
            <span
              className="text-[9px] font-bold font-mono px-1.5 py-0.5 rounded flex-shrink-0 leading-none"
              style={{ color, background: bg }}
            >
              {badge}
            </span>
            <span className="truncate">{tabLabel(id)}</span>
          </div>
        ),
      };
    });
  }, [openTabs, tabBadge, tabLabel, isDraft, dirtyTabs, methodColor, methodBg]);

  return (
    <TabBar
      tabs={tabsItems}
      activeTab={activeTab}
      onTabClick={setActiveTab}
      onTabClose={closeTab}
      onNewTab={openNewTabInFolder}
      onReorderTabs={reorderTabs}
      newTabTitle={strings.mocks.newTab}
      closeTabTitle={strings.mocks.closeTab}
      onCloseOthers={closeOtherTabs}
      onCloseAll={closeAllTabs}
      onTabDuplicate={handleDuplicateImpl}
    />
  );
}
