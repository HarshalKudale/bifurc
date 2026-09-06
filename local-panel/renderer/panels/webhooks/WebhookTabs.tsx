import React, { useMemo } from "react";
import TabBar from "@/components/editor/TabBar";
import { strings } from "@/lib/strings";
import { Webhook } from "@/lib/icons";

interface Props {
  openTabs: string[];
  activeTab: string | null;
  dirtyTabs: Record<string, boolean>;
  tabLabel: (id: string) => string;
  isDraftId: (id: string) => boolean;
  activeTabs: Set<string>;
  openTab: (id: string) => void;
  closeTab: (id: string) => void;
  openNewTab: () => void;
  handleReorderTabs: (oldI: number, newI: number) => void;
  handleDuplicate: (id: string) => void;
}

export default function WebhookTabs({
  openTabs,
  activeTab,
  dirtyTabs,
  tabLabel,
  isDraftId,
  activeTabs,
  openTab,
  closeTab,
  openNewTab,
  handleReorderTabs,
  handleDuplicate,
}: Props) {
  const tabsItems = useMemo(() => {
    return openTabs.map((id) => {
      const isActivated = activeTabs.has(id);
      const isDraft = isDraftId(id);
      return {
        id,
        label: tabLabel(id),
        isDraft,
        isModified: dirtyTabs[id],
        renderTab: ({ isActive }: { isActive?: boolean }) => (
          <div className="flex items-center gap-1.5 min-w-0" title={tabLabel(id)}>
            {!isDraft && isActivated && (
              <span className="w-2 h-2 rounded-full bg-green-500/80 animate-pulse flex-shrink-0" />
            )}
            {!isDraft && !isActivated && (
              <Webhook size={12} className="text-muted-foreground flex-shrink-0" />
            )}
            {isDraft && (
              <span className="text-[9px] font-bold font-mono px-1.5 py-0.5 rounded bg-muted text-muted-foreground flex-shrink-0 leading-none">
                +
              </span>
            )}
            <span className={`truncate text-xs font-medium ${isActive ? "text-foreground" : "text-muted-foreground"}`}>
              {tabLabel(id)}
            </span>
          </div>
        ),
      };
    });
  }, [openTabs, activeTabs, isDraftId, dirtyTabs, tabLabel]);

  return (
    <TabBar
      tabs={tabsItems}
      activeTab={activeTab}
      onTabClick={openTab}
      onTabClose={closeTab}
      onNewTab={openNewTab}
      onReorderTabs={handleReorderTabs}
      newTabTitle={strings.webhooks.newTab}
      onCloseOthers={(id) => {
        openTabs.filter((t) => t !== id).forEach(closeTab);
      }}
      onCloseAll={() => {
        [...openTabs].forEach(closeTab);
      }}
      onTabDuplicate={handleDuplicate}
    />
  );
}
