import React from "react";
import { ChevronLeft, ChevronRight } from "@/lib/icons";
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from "react-resizable-panels";
import { strings } from "@/lib/strings";

interface SidebarLayoutProps {
  sidebarOpen: boolean;
  onToggle: () => void;
  sidebar: React.ReactNode;
  children: React.ReactNode;
  collapsedBadge?: React.ReactNode;
  collapseTitle?: string;
  expandTitle?: string;
  /** Default sidebar size as percentage (default 20) */
  defaultSize?: number;
  /** Minimum sidebar size as percentage (default 15) */
  minSize?: number;
  /** Maximum sidebar size as percentage (default 40) */
  maxSize?: number;
  /** Storage key for persisting panel sizes */
  storageKey?: string;
}

export default function SidebarLayout({
  sidebarOpen,
  onToggle,
  sidebar,
  children,
  collapsedBadge,
  collapseTitle = strings.titleBar.collapseSidebar,
  expandTitle = strings.titleBar.expandSidebar,
  defaultSize = 230,
  minSize = 230,
  maxSize = 400,
  storageKey,
}: SidebarLayoutProps) {
  return (
    <PanelGroup
      orientation="horizontal"
      className="flex flex-1 overflow-hidden"
      {...(storageKey ? { autoSaveId: storageKey } : {})}
    >
      {sidebarOpen ? (
        <>
          {/* Resizable sidebar when open */}
          <Panel
            defaultSize={defaultSize}
            minSize={minSize}
            maxSize={maxSize}
            className="flex flex-col bg-surface border-r border-border overflow-hidden"
          >
            {sidebar}
          </Panel>

          <PanelResizeHandle className="w-1 bg-border hover:bg-signal/40 active:bg-signal/60 transition-colors cursor-col-resize flex-shrink-0" />

          {/* Main content area */}
          <Panel defaultSize={100 - defaultSize} minSize={60} className="flex flex-col overflow-hidden">
            {children}
          </Panel>
        </>
      ) : (
        <>
          {/* Collapsed strip when sidebar is closed */}
          <div className="flex flex-col items-center py-2 gap-2 border-r border-border bg-surface flex-shrink-0 w-9">
            <button
              onClick={onToggle}
              title={expandTitle}
              className="w-7 h-7 flex items-center justify-center rounded hover:bg-card text-muted-foreground hover:text-foreground transition-colors cursor-pointer"
            >
              <ChevronRight size={13} />
            </button>
            {collapsedBadge}
          </div>

          {/* Main content takes full width when collapsed */}
          <Panel defaultSize={100} className="flex flex-col overflow-hidden">
            {children}
          </Panel>
        </>
      )}
    </PanelGroup>
  );
}

interface SidebarHeaderProps {
  children: React.ReactNode;
  onCollapse: () => void;
  collapseTitle?: string;
}

export function SidebarHeader({ children, onCollapse, collapseTitle = strings.titleBar.collapseSidebar }: SidebarHeaderProps) {
  return (
    <div className="flex items-center gap-1.5 px-2 py-2 border-b border-border flex-shrink-0">
      <div className="flex-1 min-w-0">{children}</div>
      <button
        onClick={onCollapse}
        title={collapseTitle}
        className="w-7 h-7 flex items-center justify-center rounded hover:bg-card text-muted-foreground hover:text-foreground transition-colors flex-shrink-0 cursor-pointer"
      >
        <ChevronLeft size={13} />
      </button>
    </div>
  );
}
