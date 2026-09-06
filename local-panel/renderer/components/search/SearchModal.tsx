import React, { useState, useEffect, useRef, useMemo, useCallback } from "react";
import { AppConfig, ServiceInfo } from "@/types";
import { Panel } from "@/lib/panelRegistry";
import { Search, X, Globe } from "@/lib/icons";
import {
  searchEntities,
  SearchResultItem,
  getPanelDisplayName,
} from "./searchUtils";
import { SearchResultsList } from "./SearchResultsList";

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

  useEffect(() => {
    if (open) {
      onRefreshServices?.();
      setMode(initialMode);
      setQuery("");
      setDebouncedQuery("");
      setActiveIndex(0);
      previousActiveElementRef.current = document.activeElement as HTMLElement | null;
      requestAnimationFrame(() => {
        inputRef.current?.focus();
        inputRef.current?.select();
      });
    }
  }, [open, initialMode, onRefreshServices]);

  useEffect(() => {
    const timer = setTimeout(() => setDebouncedQuery(query), 300);
    return () => clearTimeout(timer);
  }, [query]);

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

  useEffect(() => {
    setActiveIndex(0);
  }, [debouncedQuery, mode]);

  useEffect(() => {
    if (flatItems.length > 0 && itemRefs.current[activeIndex]) {
      itemRefs.current[activeIndex]?.scrollIntoView({
        block: "nearest",
        behavior: "smooth",
      });
    }
  }, [activeIndex, flatItems.length]);

  const toggleMode = useCallback(() => {
    setMode((prev) => (prev === "current" ? "global" : "current"));
    setActiveIndex(0);
    inputRef.current?.focus();
  }, []);

  const handleClose = useCallback(() => {
    onClose();
    if (previousActiveElementRef.current && typeof previousActiveElementRef.current.focus === "function") {
      previousActiveElementRef.current.focus();
    }
  }, [onClose]);

  const handleSelect = useCallback(
    (item: SearchResultItem) => {
      handleClose();

      if (item.isOpenTab && item.tabId) {
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
        onOpenEntity(item.panel, item.id, item.entityType, item.original);
      } else if (item.entityType === "service" && item.original?.port) {
        onNavigatePanel("mappings", `localhost:${item.original.port}`);
      } else {
        onNavigatePanel(item.panel, item.original);
      }
    },
    [handleClose, onSelectTab, onOpenEntity, onNavigatePanel]
  );

  const handleMapService = useCallback(
    (e: React.MouseEvent, port: number) => {
      e.stopPropagation();
      handleClose();
      onNavigatePanel("mappings", `localhost:${port}`);
    },
    [handleClose, onNavigatePanel]
  );

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
        <div className="flex-shrink-0 p-3 border-b border-border/70 bg-surface-2/30">
          <div className="flex items-center gap-2.5 px-3 py-2 bg-card border border-border rounded-lg focus-within:border-signal/70 focus-within:ring-2 focus-within:ring-signal/20 transition-all">
            <Search size={15} className="text-muted-foreground flex-shrink-0" />

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

        <SearchResultsList
          ref={listRef}
          sections={sections}
          flatItems={flatItems}
          activeIndex={activeIndex}
          mode={mode}
          debouncedQuery={debouncedQuery}
          currentScreenName={currentScreenName}
          itemRefs={itemRefs}
          onSelect={handleSelect}
          onHover={setActiveIndex}
          onMapService={handleMapService}
        />

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

