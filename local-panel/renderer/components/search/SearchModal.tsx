import React, { useState, useEffect, useRef, useMemo, useCallback } from "react";
import { AppConfig, ServiceInfo } from "@/types";
import { Panel } from "@/lib/panelRegistry";
import { Search, X, Folder, Layers, ExternalLink, Globe, ArrowRight } from "@/lib/icons";
import { methodColor, methodBg } from "@/lib/utils";
import {
  searchEntities,
  SearchResultItem,
  getPanelDisplayName,
  normalizePanel,
} from "./searchUtils";

interface Props {
  open: boolean;
  initialMode: "current" | "global";
  activePanel: Panel;
  config: AppConfig;
  services?: ServiceInfo[];
  onClose: () => void;
  onSelectTab: (panel: Panel, tabId: string) => void;
  onOpenEntity: (panel: Panel, entityId: string, entityType: string, original?: any) => void;
  onNavigatePanel: (panel: Panel, target?: any) => void;
  onRefreshServices?: () => void;
}

export default function SearchModal({
  open,
  initialMode,
  activePanel,
  config,
  services = [],
  onClose,
  onSelectTab,
  onOpenEntity,
  onNavigatePanel,
  onRefreshServices,
}: Props) {
  const [mode, setMode] = useState<"current" | "global">(initialMode);
  const [query, setQuery] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [activeIndex, setActiveIndex] = useState(0);

  const inputRef = useRef<HTMLInputElement>(null);
  const previousActiveElementRef = useRef<HTMLElement | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const itemRefs = useRef<(HTMLDivElement | null)[]>([]);

  // When initialMode changes when modal is triggered, sync mode
  useEffect(() => {
    if (open) {
      onRefreshServices?.();
      setMode(initialMode);
      setQuery("");
      setDebouncedQuery("");
      setActiveIndex(0);
      previousActiveElementRef.current = document.activeElement as HTMLElement | null;
      // Focus instantly on open
      requestAnimationFrame(() => {
        inputRef.current?.focus();
        inputRef.current?.select();
      });
    }
  }, [open, initialMode, onRefreshServices]);

  // Debounce query filtering with 300ms
  useEffect(() => {
    const timer = setTimeout(() => {
      setDebouncedQuery(query);
    }, 300);
    return () => clearTimeout(timer);
  }, [query]);

  // Compute search results
  const { sections, flatItems } = useMemo(() => {
    if (!open) return { sections: [], flatItems: [] };
    return searchEntities({
      query: debouncedQuery,
      mode,
      activePanel,
      config,
      services,
      maxPerSection: 8,
    });
  }, [open, debouncedQuery, mode, activePanel, config, services]);

  // Reset active index when query, mode, or flatItems change
  useEffect(() => {
    setActiveIndex(0);
  }, [debouncedQuery, mode]);

  // Scroll active item into view
  useEffect(() => {
    if (flatItems.length > 0 && itemRefs.current[activeIndex]) {
      itemRefs.current[activeIndex]?.scrollIntoView({
        block: "nearest",
        behavior: "smooth",
      });
    }
  }, [activeIndex, flatItems.length]);

  // Toggle mode helper
  const toggleMode = useCallback(() => {
    setMode((prev) => (prev === "current" ? "global" : "current"));
    setActiveIndex(0);
    // Keep input focused
    inputRef.current?.focus();
  }, []);

  // Handle closing modal and restoring focus
  const handleClose = useCallback(() => {
    onClose();
    if (previousActiveElementRef.current && typeof previousActiveElementRef.current.focus === "function") {
      previousActiveElementRef.current.focus();
    }
  }, [onClose]);

  // Handle executing the selected action
  const handleSelect = useCallback(
    (item: SearchResultItem) => {
      handleClose();

      if (item.isOpenTab && item.tabId) {
        // Focus existing open tab
        onSelectTab(item.panel, item.tabId);
      } else if (item.entityType === "tab" && item.tabId) {
        onSelectTab(item.panel, item.tabId);
      } else if (
        item.entityType === "request" ||
        item.entityType === "mock" ||
        item.entityType === "rule" ||
        item.entityType === "websocket" ||
        item.entityType === "webhook"
      ) {
        // Open saved entity in new active tab
        onOpenEntity(item.panel, item.id, item.entityType, item.original);
      } else if (item.entityType === "service" && item.original?.port) {
        // Services action: Map the service in Mappings panel
        onNavigatePanel("mappings", `localhost:${item.original.port}`);
      } else {
        // Non-tab entity: switch to respective panel
        onNavigatePanel(item.panel, item.original);
      }
    },
    [handleClose, onSelectTab, onOpenEntity, onNavigatePanel]
  );

  // Keyboard navigation & controls
  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Tab") {
      e.preventDefault();
      toggleMode();
      return;
    }

    if (e.key === "Escape") {
      e.preventDefault();
      handleClose();
      return;
    }

    if (e.key === "ArrowDown") {
      e.preventDefault();
      if (flatItems.length === 0) return;
      setActiveIndex((prev) => (prev + 1) % flatItems.length);
      return;
    }

    if (e.key === "ArrowUp") {
      e.preventDefault();
      if (flatItems.length === 0) return;
      setActiveIndex((prev) => (prev - 1 + flatItems.length) % flatItems.length);
      return;
    }

    if (e.key === "Enter") {
      e.preventDefault();
      if (flatItems.length > 0 && flatItems[activeIndex]) {
        handleSelect(flatItems[activeIndex]);
      }
    }
  };

  if (!open) return null;

  const currentScreenName = getPanelDisplayName(activePanel);
  let globalFlatIndexCounter = 0;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center p-4 bg-black/60 backdrop-blur-sm animate-in fade-in duration-150"
      onClick={(e) => {
        if (e.target === e.currentTarget) handleClose();
      }}
    >
      <div
        className="w-full max-w-2xl h-[480px] bg-surface border border-border rounded-xl shadow-2xl flex flex-col overflow-hidden text-foreground"
        onKeyDown={handleKeyDown}
        onClick={(e) => e.stopPropagation()}
      >
        {/* Top Search Input Container */}
        <div className="flex-shrink-0 p-3 border-b border-border/70 bg-surface-2/30">
          <div className="flex items-center gap-2.5 px-3 py-2 bg-card border border-border rounded-lg focus-within:border-signal/70 focus-within:ring-2 focus-within:ring-signal/20 transition-all">
            <Search size={15} className="text-muted-foreground flex-shrink-0" />

            {/* Mode Indicator Badge inside search bar - fixed width to accommodate longest name (Environments) */}
            <button
              type="button"
              onClick={toggleMode}
              title="Press Tab to switch search mode"
              className={`w-[160px] flex items-center justify-between px-2.5 py-1 rounded-md text-xs font-semibold select-none flex-shrink-0 transition-colors border cursor-pointer ${
                mode === "current"
                  ? "bg-signal/15 border-signal/40 text-signal hover:bg-signal/25"
                  : "bg-surface-2 border-border text-foreground hover:bg-surface-3"
              }`}
            >
              <div className="flex items-center gap-1.5 min-w-0 flex-1">
                {mode === "current" ? (
                  <span className="w-1.5 h-1.5 rounded-full bg-signal flex-shrink-0" />
                ) : (
                  <Globe size={12} className="text-signal flex-shrink-0" />
                )}
                <span className="truncate">
                  {mode === "current" ? currentScreenName : "Global"}
                </span>
              </div>
              <kbd className="ml-1 px-1 py-0.2 text-[9px] bg-surface border border-border/80 rounded font-mono text-muted-foreground flex-shrink-0">
                Tab ⇥
              </kbd>
            </button>

            {/* Search Input */}
            <input
              ref={inputRef}
              type="text"
              className="flex-1 bg-transparent border-0 outline-none text-sm text-foreground placeholder:text-muted-foreground/60 min-w-0"
              placeholder={
                mode === "current"
                  ? `Search ${currentScreenName.toLowerCase()}...`
                  : "Search across all requests, mocks, rules, services..."
              }
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />

            {/* Clear query button */}
            {query && (
              <button
                type="button"
                onClick={() => {
                  setQuery("");
                  setDebouncedQuery("");
                  inputRef.current?.focus();
                }}
                className="text-muted-foreground hover:text-foreground p-1 rounded hover:bg-surface-2 transition-colors cursor-pointer flex-shrink-0"
                title="Clear search"
              >
                <X size={14} />
              </button>
            )}
          </div>
        </div>

        {/* Results List */}
        <div ref={listRef} className="flex-1 min-h-0 overflow-y-auto divide-y divide-border/30">
          {flatItems.length === 0 ? (
            <div className="h-full flex flex-col items-center justify-center p-8 text-center">
              <div className="inline-flex items-center justify-center w-10 h-10 rounded-full bg-surface-2 text-muted-foreground mb-3">
                <Search size={18} />
              </div>
              <div className="text-sm font-medium text-foreground">
                {debouncedQuery.trim()
                  ? `No entities found matching '${debouncedQuery.trim()}'`
                  : mode === "current"
                  ? `No open tabs or entities in ${currentScreenName}`
                  : "Type to search entities across workspace"}
              </div>
              <div className="text-xs text-muted-foreground mt-1">
                {debouncedQuery.trim()
                  ? mode === "current"
                    ? "Press Tab to switch to Global Search"
                    : "Try searching for another name, method, or URL"
                  : "Search requests, mocks, proxy rules, sockets, webhooks, and services"}
              </div>
            </div>
          ) : (
            sections.map((sec) => (
              <div key={sec.title} className="py-2">
                <div className="px-4 py-1 flex items-center justify-between text-[11px] font-semibold tracking-wider text-muted-foreground uppercase">
                  <span>{sec.title}</span>
                  <span className="text-[10px] font-normal lowercase opacity-70">
                    {sec.items.length} {sec.items.length === 1 ? "result" : "results"}
                  </span>
                </div>
                <div className="mt-1 space-y-0.5 px-2">
                  {sec.items.map((item) => {
                    const thisIndex = globalFlatIndexCounter++;
                    const isSelected = thisIndex === activeIndex;

                    return (
                      <div
                        key={item.id + item.section}
                        ref={(el) => {
                          itemRefs.current[thisIndex] = el;
                        }}
                        onClick={() => handleSelect(item)}
                        onMouseEnter={() => setActiveIndex(thisIndex)}
                        className={`flex items-center gap-3 px-3 py-2 rounded-lg cursor-pointer transition-colors text-xs select-none ${
                          isSelected
                            ? "bg-signal/15 border-l-2 border-signal text-foreground pl-[10px]"
                            : "hover:bg-surface-2 text-foreground/90 pl-3"
                        }`}
                      >
                        {/* Method / Entity Badge */}
                        {item.method ? (
                          <span
                            className="px-1.5 py-0.5 rounded font-mono font-bold text-[10px] flex-shrink-0 uppercase"
                            style={{
                              color: methodColor(item.method),
                              backgroundColor: methodBg(item.method),
                            }}
                          >
                            {item.method}
                          </span>
                        ) : (
                          <span className="px-1.5 py-0.5 rounded font-mono font-bold text-[10px] bg-surface-2 text-muted-foreground flex-shrink-0">
                            {item.entityType.toUpperCase()}
                          </span>
                        )}

                        {/* Title & Subtitle */}
                        <div className="flex-1 min-w-0 flex flex-col justify-center">
                          <div className="flex items-center gap-2">
                            <span className="font-medium text-foreground truncate">
                              {item.title}
                            </span>
                            {item.isDraft && (
                              <span className="text-[9px] px-1 py-0.2 rounded bg-amber/15 text-amber border border-amber/30">
                                Draft
                              </span>
                            )}
                          </div>
                          {item.subtitle && (
                            <div className="text-[11px] text-muted-foreground truncate font-mono">
                              {item.subtitle}
                            </div>
                          )}
                        </div>

                        {/* Folder badge if present */}
                        {item.folderName && (
                          <span className="flex items-center gap-1 text-[10px] text-muted-foreground bg-surface-2 px-1.5 py-0.5 rounded flex-shrink-0">
                            <Folder size={10} />
                            <span className="truncate max-w-[100px]">{item.folderName}</span>
                          </span>
                        )}

                        {/* Service Map button or Mapped indicator */}
                        {item.entityType === "service" && item.original?.port && (
                          <div className="flex items-center gap-1.5 flex-shrink-0">
                            {item.isMapped ? (
                              <span className="px-1.5 py-0.5 text-[10px] font-medium rounded bg-emerald-500/15 text-emerald-400 border border-emerald-500/30">
                                Mapped
                              </span>
                            ) : (
                              <button
                                type="button"
                                onClick={(e) => {
                                  e.stopPropagation();
                                  handleClose();
                                  onNavigatePanel("mappings", `localhost:${item.original.port}`);
                                }}
                                title={`Map localhost:${item.original.port}`}
                                className="px-2 py-0.5 text-[11px] font-semibold rounded bg-signal/15 hover:bg-signal/25 text-signal border border-signal/30 hover:border-signal/50 transition-all cursor-pointer flex items-center gap-1"
                              >
                                <span>Map</span>
                                <ArrowRight size={10} />
                              </button>
                            )}
                          </div>
                        )}

                        {/* Open Tab Indicator */}
                        {item.isOpenTab ? (
                          <span className="px-1.5 py-0.5 rounded text-[10px] font-semibold bg-signal/20 text-signal border border-signal/30 flex-shrink-0">
                            OPEN TAB
                          </span>
                        ) : mode === "global" ? (
                          <span className="px-1.5 py-0.5 rounded text-[10px] bg-surface-2 text-muted-foreground flex-shrink-0">
                            {getPanelDisplayName(item.panel)}
                          </span>
                        ) : null}

                        {/* Enter arrow hint when selected */}
                        {isSelected && (
                          <div className="flex items-center text-muted-foreground flex-shrink-0">
                            <kbd className="px-1 py-0.5 text-[9px] bg-card border border-border rounded font-mono">
                              ↵
                            </kbd>
                          </div>
                        )}
                      </div>
                    );
                  })}
                </div>
              </div>
            ))
          )}
        </div>

        {/* Footer Shortcut Bar */}
        <div className="flex-shrink-0 flex items-center justify-between px-4 py-2 bg-surface border-t border-border text-[11px] text-muted-foreground">
          <div className="flex items-center gap-3">
            <span className="flex items-center gap-1">
              <kbd className="px-1 py-0.5 bg-card border border-border rounded font-mono text-[9px]">
                ↑
              </kbd>
              <kbd className="px-1 py-0.5 bg-card border border-border rounded font-mono text-[9px]">
                ↓
              </kbd>
              navigate
            </span>
            <span className="flex items-center gap-1">
              <kbd className="px-1.5 py-0.5 bg-card border border-border rounded font-mono text-[9px]">
                ↵
              </kbd>
              select
            </span>
            <span className="flex items-center gap-1">
              <kbd className="px-1.5 py-0.5 bg-card border border-border rounded font-mono text-[9px]">
                Tab
              </kbd>
              switch mode
            </span>
          </div>

          <div className="flex items-center gap-2">
            <span>
              Mode:{" "}
              <strong className="text-foreground">
                {mode === "current" ? currentScreenName : "Global"}
              </strong>
            </span>
          </div>
        </div>
      </div>
    </div>
  );
}
