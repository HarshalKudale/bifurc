import React, { useState } from "react";
import { ChevronDown, Folder, FolderOpen, X } from "@/lib/icons";
import { strings } from "@/lib/strings";

interface Props {
  label: string;
  draftTabIds: string[];
  activeTab: string | null;
  onOpenTab(id: string): void;
  onCloseTab(id: string): void;
  tabLabel(id: string): string;
}

const LINE_COLOR  = "var(--c-border)";
const CONNECTOR_W = 12;
const INDENT      = 14;

export default function DraftsFolder({ label, draftTabIds, activeTab, onOpenTab, onCloseTab, tabLabel }: Props) {
  const [expanded, setExpanded] = useState(true);

  return (
    <div style={{ minWidth: "max-content" }}>
      <div
        style={{ display: "flex", alignItems: "center", height: 30, paddingRight: 8, paddingLeft: 4, gap: 5, borderRadius: 4, marginLeft: 2, marginRight: 4, cursor: "pointer", userSelect: "none", whiteSpace: "nowrap" }}
        onClick={() => setExpanded((v) => !v)}
        onMouseEnter={(e) => { (e.currentTarget as HTMLDivElement).style.background = "var(--c-card)"; }}
        onMouseLeave={(e) => { (e.currentTarget as HTMLDivElement).style.background = ""; }}
      >
        <span style={{ flexShrink: 0, width: 14, display: "flex", alignItems: "center", justifyContent: "center", color: "var(--c-muted-foreground)", transition: "transform 0.15s ease", transform: expanded ? "rotate(0deg)" : "rotate(-90deg)" }}>
          <ChevronDown size={11} />
        </span>
        <span style={{ display: "flex", alignItems: "center", flexShrink: 0, color: "var(--c-muted-foreground)", marginRight: 2 }}>
          {expanded ? <FolderOpen size={13} /> : <Folder size={13} />}
        </span>
        <span style={{ fontSize: 12, fontWeight: 500, color: "var(--c-amber)", flex: 1, overflow: "hidden", textOverflow: "ellipsis" }}>
          {label}
        </span>
        <span style={{ fontSize: 9, color: "var(--c-muted-foreground)", flexShrink: 0, fontFamily: "monospace" }}>{draftTabIds.length}</span>
      </div>
      {expanded && (
        <div style={{ paddingLeft: CONNECTOR_W, marginLeft: INDENT, borderLeft: `1px solid ${LINE_COLOR}` }}>
          {draftTabIds.map((tabId) => {
            const isActive = activeTab === tabId;
            return (
              <div key={tabId} title={tabLabel(tabId)}
                style={{ position: "relative", display: "flex", alignItems: "center", height: 30, paddingRight: 8, gap: 5, borderRadius: 4, marginLeft: 2, marginRight: 4, cursor: "pointer", userSelect: "none", whiteSpace: "nowrap", background: isActive ? "var(--c-surface-2)" : undefined }}
                onClick={() => onOpenTab(tabId)}
                onMouseEnter={(e) => { if (!isActive) (e.currentTarget as HTMLDivElement).style.background = "var(--c-card)"; }}
                onMouseLeave={(e) => { if (!isActive) (e.currentTarget as HTMLDivElement).style.background = ""; }}
              >
                <div style={{ position: "absolute", left: -CONNECTOR_W, top: "50%", width: CONNECTOR_W - 2, height: 1, background: LINE_COLOR, transform: "translateY(-50%)", pointerEvents: "none" }} />
                <span style={{ flexShrink: 0, width: 14, display: "flex", alignItems: "center", justifyContent: "center" }}>
                  <span style={{ display: "inline-block", width: 7, height: 7, borderRadius: "50%", background: "var(--c-amber)", opacity: 0.8 }} />
                </span>
                <span style={{ fontSize: 12, lineHeight: 1, flex: 1, overflow: "hidden", textOverflow: "ellipsis", color: isActive ? "var(--c-signal)" : "var(--c-muted-foreground)", fontStyle: "italic" }}>
                  {tabLabel(tabId)}
                </span>
                <button onClick={(e) => { e.stopPropagation(); onCloseTab(tabId); }} title={strings.sidebar.discardDraft}
                  style={{ flexShrink: 0, width: 14, height: 14, display: "flex", alignItems: "center", justifyContent: "center", borderRadius: 3, lineHeight: 1, color: "var(--c-muted-foreground)", background: "transparent", border: "none", cursor: "pointer" }}
                  onMouseEnter={(e) => { (e.currentTarget as HTMLButtonElement).style.background = "var(--c-surface-2)"; (e.currentTarget as HTMLButtonElement).style.color = "var(--c-destructive)"; }}
                  onMouseLeave={(e) => { (e.currentTarget as HTMLButtonElement).style.background = "transparent"; (e.currentTarget as HTMLButtonElement).style.color = "var(--c-muted-foreground)"; }}
                ><X size={10} /></button>
              </div>
            );
          })}
        </div>
      )}
    </div>
  );
}
