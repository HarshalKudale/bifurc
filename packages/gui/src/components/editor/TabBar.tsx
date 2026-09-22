import React, { useRef, useState, useEffect, useCallback } from "react";
import { Plus, ChevronLeft, ChevronRight, X, Copy } from "@/lib/icons";
import ContextMenu, { ContextMenuItem } from "@/components/ui/ContextMenu";
import { strings } from "@/lib/strings";

export interface TabBarTab {
  id: string;
  label: string;
  isDraft?: boolean;
  isModified?: boolean;
  /** Optional custom renderer for the tab pill content. Receives active state. */
  renderTab?: (isActive: boolean) => React.ReactNode;
}

interface Props {
  tabs: TabBarTab[];
  activeTab: string | null;
  onTabClick(id: string): void;
  onTabClose(id: string): void;
  onNewTab(): void;
  onTabDuplicate?(id: string): void;
  onCloseOthers?(id: string): void;
  onCloseAll?(): void;
  onReorderTabs?(fromId: string, toId: string, edge?: "left" | "right"): void;
  newTabTitle?: string;
  closeTabTitle?: string;
}

export default function TabBar({
  tabs,
  activeTab,
  onTabClick,
  onTabClose,
  onNewTab,
  onTabDuplicate,
  onCloseOthers,
  onCloseAll,
  onReorderTabs,
  newTabTitle = "New tab",
  closeTabTitle = "Close tab",
}: Props) {
  const scrollRef = useRef<HTMLDivElement>(null);
  const [canLeft, setCanLeft] = useState(false);
  const [canRight, setCanRight] = useState(false);
  const [tabCtxMenu, setTabCtxMenu] = useState<{ x: number; y: number; tabId: string } | null>(null);

  // Drag and drop state
  const [draggedTabId, setDraggedTabId] = useState<string | null>(null);
  const [dropTarget, setDropTarget] = useState<{ id: string; edge: "left" | "right" } | null>(null);

  const sync = useCallback(() => {
    const el = scrollRef.current;
    if (!el) return;
    setCanLeft(el.scrollLeft > 0);
    setCanRight(el.scrollLeft + el.clientWidth < el.scrollWidth - 1);
  }, []);

  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    const ro = new ResizeObserver(sync);
    ro.observe(el);
    el.addEventListener("scroll", sync, { passive: true });
    sync();
    return () => {
      ro.disconnect();
      el.removeEventListener("scroll", sync);
    };
  }, [sync]);

  // Re-check whenever tab list changes
  useEffect(sync, [tabs, sync]);

  // Scroll active tab into view when it changes
  useEffect(() => {
    if (!activeTab || !scrollRef.current) return;
    const el = scrollRef.current.querySelector<HTMLElement>(`[data-tab-id="${activeTab}"]`);
    el?.scrollIntoView({ block: "nearest", inline: "nearest" });
  }, [activeTab]);

  return (
    <div className="flex items-end border-b border-border bg-surface flex-shrink-0 h-[38px] min-h-[38px] max-h-[38px] px-1.5 gap-1 select-none">
      {/* Sticky add button */}
      <button
        type="button"
        onClick={onNewTab}
        title={newTabTitle}
        className="flex items-center justify-center w-7 h-[28px] mb-[3px] rounded-md text-muted-foreground hover:text-signal hover:bg-card transition-colors cursor-pointer flex-shrink-0"
      >
        <Plus size={14} />
      </button>

      {/* Left chevron */}
      {canLeft && (
        <button
          type="button"
          className="flex items-center justify-center w-5 h-[28px] mb-[3px] rounded-md text-muted-foreground hover:text-foreground hover:bg-card transition-colors cursor-pointer flex-shrink-0"
          onClick={() => scrollRef.current?.scrollBy({ left: -120, behavior: "smooth" })}
        >
          <ChevronLeft size={12} />
        </button>
      )}

      {/* Scrollable tabs */}
      <div
        ref={scrollRef}
        className="tab-bar-scroll flex items-end flex-1 overflow-x-auto min-w-0 h-full gap-1 pt-1"
        onWheel={(e) => {
          const el = scrollRef.current;
          if (!el) return;
          e.preventDefault();
          el.scrollBy({ left: e.deltaY !== 0 ? e.deltaY : e.deltaX, behavior: "smooth" });
        }}
      >
        {tabs.map((tab) => {
          const isActive = tab.id === activeTab;
          const isDragging = draggedTabId === tab.id;
          const isTargetLeft = dropTarget?.id === tab.id && dropTarget.edge === "left";
          const isTargetRight = dropTarget?.id === tab.id && dropTarget.edge === "right";

          const baseClass = `group relative flex items-center gap-1.5 px-2.5 h-[32px] min-h-[32px] max-h-[32px] w-[160px] min-w-[160px] max-w-[160px] text-xs font-medium cursor-pointer flex-shrink-0 transition-all rounded-t-lg select-none ${
            isActive
              ? "bg-background text-foreground border-t border-x border-border -mb-px z-10 shadow-sm font-medium"
              : "bg-surface-2/40 text-muted-foreground hover:bg-surface-2 hover:text-foreground border-t border-x border-transparent hover:border-border/40"
          } ${isDragging ? "opacity-40 scale-[0.98]" : ""}`;

          const handleCtxMenu = (e: React.MouseEvent) => {
            e.preventDefault();
            e.stopPropagation();
            onTabClick(tab.id);
            setTabCtxMenu({ x: e.clientX, y: e.clientY, tabId: tab.id });
          };

          return (
            <div
              key={tab.id}
              data-tab-id={tab.id}
              draggable={Boolean(onReorderTabs)}
              onDragStart={(e) => {
                setDraggedTabId(tab.id);
                e.dataTransfer.effectAllowed = "move";
                e.dataTransfer.setData("text/plain", tab.id);
              }}
              onDragOver={(e) => {
                if (!draggedTabId || draggedTabId === tab.id) return;
                e.preventDefault();
                e.dataTransfer.dropEffect = "move";
                const rect = e.currentTarget.getBoundingClientRect();
                const edge = e.clientX < rect.left + rect.width / 2 ? "left" : "right";
                if (!dropTarget || dropTarget.id !== tab.id || dropTarget.edge !== edge) {
                  setDropTarget({ id: tab.id, edge });
                }
              }}
              onDragLeave={(e) => {
                if (e.currentTarget.contains(e.relatedTarget as Node)) return;
                if (dropTarget?.id === tab.id) setDropTarget(null);
              }}
              onDrop={(e) => {
                e.preventDefault();
                if (draggedTabId && onReorderTabs && draggedTabId !== tab.id) {
                  onReorderTabs(draggedTabId, tab.id, dropTarget?.edge ?? "left");
                }
                setDraggedTabId(null);
                setDropTarget(null);
              }}
              onDragEnd={() => {
                setDraggedTabId(null);
                setDropTarget(null);
              }}
              onClick={() => onTabClick(tab.id)}
              onAuxClick={(e) => {
                // Middle-click to close tab
                if (e.button === 1) {
                  e.preventDefault();
                  e.stopPropagation();
                  onTabClose(tab.id);
                }
              }}
              onContextMenu={handleCtxMenu}
              className={baseClass}
              title={tab.label}
            >
              {/* Drop target indicator lines */}
              {isTargetLeft && (
                <div className="absolute -left-[2px] top-1.5 bottom-1.5 w-[3px] bg-signal rounded-full shadow-[0_0_8px_var(--c-signal)] z-30 pointer-events-none" />
              )}
              {isTargetRight && (
                <div className="absolute -right-[2px] top-1.5 bottom-1.5 w-[3px] bg-signal rounded-full shadow-[0_0_8px_var(--c-signal)] z-30 pointer-events-none" />
              )}

              {/* Active top neon accent line */}
              {isActive && (
                              <div className="absolute bottom-0 left-0 right-0 h-[2px] bg-signal rounded-full shadow-[0_0_8px_var(--c-signal)] z-20" />
                            )}

              {/* Status / modified / draft indicators */}
              {tab.isModified && (
                <span className="text-[10px] text-signal font-bold flex-shrink-0 leading-none mr-0.5" title="Modified">
                  *
                </span>
              )}
              {tab.isDraft && (
                <span className="text-[8px] text-amber opacity-80 flex-shrink-0 mr-0.5" title="Unsaved draft">
                  ●
                </span>
              )}

              {/* Tab main content */}
              <div className="flex-1 min-w-0 overflow-hidden flex items-center">
                {tab.renderTab ? tab.renderTab(isActive) : <span className="truncate">{tab.label}</span>}
              </div>

              {/* Tab close button - visible on tab hover */}
              <button
                type="button"
                onClick={(e) => {
                  e.stopPropagation();
                  onTabClose(tab.id);
                }}
                className="w-4 h-4 rounded flex items-center justify-center text-muted-foreground/60 hover:text-foreground hover:bg-surface-2 transition-all ml-auto flex-shrink-0 cursor-pointer opacity-0 group-hover:opacity-100 focus:opacity-100"
                title={closeTabTitle}
                aria-label={closeTabTitle}
              >
                <X size={11} />
              </button>
            </div>
          );
        })}
      </div>

      {/* Right chevron */}
      {canRight && (
        <button
          type="button"
          className="flex items-center justify-center w-5 h-[28px] mb-[3px] rounded-md text-muted-foreground hover:text-foreground hover:bg-card transition-colors cursor-pointer flex-shrink-0"
          onClick={() => scrollRef.current?.scrollBy({ left: 120, behavior: "smooth" })}
        >
          <ChevronRight size={12} />
        </button>
      )}

      {/* Context Menu */}
      {tabCtxMenu && (() => {
        const { tabId } = tabCtxMenu;
        const tab = tabs.find((t) => t.id === tabId);
        const isDraft = tab?.isDraft ?? false;
        const items: ContextMenuItem[] = [
          { label: strings.editor.ctxClose, icon: <X size={11} />, action: () => { onTabClose(tabId); setTabCtxMenu(null); } },
          ...(tabs.length > 1 && onCloseOthers ? [{ label: strings.editor.ctxCloseOthers, action: () => { onCloseOthers(tabId); setTabCtxMenu(null); } } as ContextMenuItem] : []),
          ...(onCloseAll ? [{ label: strings.editor.ctxCloseAll, action: () => { onCloseAll(); setTabCtxMenu(null); } } as ContextMenuItem] : []),
          ...(!isDraft && onTabDuplicate ? [{ sep: true, action: () => {} } as ContextMenuItem, { label: strings.editor.ctxDuplicate, icon: <Copy size={11} />, action: () => { onTabDuplicate(tabId); setTabCtxMenu(null); } } as ContextMenuItem] : []),
        ];
        return <ContextMenu x={tabCtxMenu.x} y={tabCtxMenu.y} items={items} onClose={() => setTabCtxMenu(null)} minWidth={160} />;
      })()}
    </div>
  );
}
