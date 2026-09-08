import React, { memo } from "react";
import { FolderTreeItem } from "./FolderTree.types";
import { useFolderTreeContext } from "./FolderTreeContext";
import { Play, Ban } from "@/lib/icons";
import ActiveDot from "@/components/ui/ActiveDot";
import SyncIndicator from "@/components/ui/SyncIndicator";
import { methodColor, methodBg } from "@/lib/utils";

const LINE_COLOR = "var(--c-border)";
const CONNECTOR_W = 12;

interface Props {
  item: FolderTreeItem;
  depth: number;
}

export const FolderTreeItemNode = memo(function FolderTreeItemNode({ item, depth }: Props) {
  const ctx = useFolderTreeContext();

  const isActive = !!item.isActive;
  const isEnabled = item.isEnabled !== false;
  const isSel = ctx.selectedItemIds.has(item.id);
  const syncSt = ctx.getItemStatus(item);
  const isDeleted = syncSt === "deleted";

  return (
    <div
      title={item.name}
      draggable={!item.isRunner && !item.isBlock && !!ctx.hasMoveItems}
      style={{
        position: "relative", display: "flex", alignItems: "center",
        height: 32, paddingLeft: 4, paddingRight: 8, gap: 5, borderRadius: 4, marginLeft: 2, marginRight: 4,
        cursor: "pointer", userSelect: "none", whiteSpace: "nowrap",
        background: isActive ? "var(--c-surface-2)" : isSel ? "oklch(var(--signal) / 0.1)" : undefined,
        outline: isSel && !isActive ? "1px solid oklch(var(--signal) / 0.25)" : undefined,
        transition: "background 0.1s ease",
      }}
      onDragStart={!item.isRunner && !item.isBlock ? (e) => ctx.onDragStartItem(e, item.id) : undefined}
      onClick={(e) => {
        e.stopPropagation();
        if (item.isRunner) {
          if (ctx.onOpenRunner && item.folderId) ctx.onOpenRunner(item.folderId);
          else ctx.onOpenItem(item.id);
          return;
        }
        if (item.isBlock) {
          return; // non-editable
        }
        if (e.shiftKey) {
          ctx.onSelectItem(item.id, false, true);
        } else if (e.ctrlKey || e.metaKey) {
          ctx.onSelectItem(item.id, true, false);
        } else if (isDeleted) {
          ctx.setDeletedItemPopup(item);
        } else {
          ctx.onSelectItem(item.id, false, false);
        }
      }}
      onContextMenu={(e) => ctx.onItemContextMenu(e, item)}
      onMouseEnter={(e) => {
        if (!isActive && !isSel) (e.currentTarget as HTMLDivElement).style.background = "var(--c-card)";
        ctx.onHoverItem(item.id);
      }}
      onMouseLeave={(e) => {
        if (!isActive && !isSel) (e.currentTarget as HTMLDivElement).style.background = "";
        ctx.onHoverItem(null);
      }}
    >
      {depth > 0 && (
        <div style={{ position: "absolute", left: -CONNECTOR_W, top: "50%", width: CONNECTOR_W - 2, height: 1, background: LINE_COLOR, transform: "translateY(-50%)", pointerEvents: "none" }} />
      )}
      {item.isRunner ? (
        <>
          <span style={{ flexShrink: 0, display: "flex", alignItems: "center", color: "var(--c-signal)", opacity: 0.8 }}>
            <Play size={11} fill="currentColor" />
          </span>
          <span style={{ fontSize: 12, lineHeight: 1, flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", color: "var(--c-signal)", fontStyle: "italic", opacity: 0.85 }}>
            {item.name}
          </span>
        </>
      ) : (
        <>
          <span style={{ flexShrink: 0, display: "flex", alignItems: "center", gap: 2 }}>
            {item.isBlock ? (
              <Ban size={12} style={{ color: "var(--c-destructive)" }} />
            ) : (
              <>
                {!item.hideDot && <ActiveDot active={isEnabled} color="green" size="sm" />}
                <SyncIndicator status={syncSt} />
              </>
            )}
          </span>
          {item.method && ctx.hoveredItemId !== item.id && (
            <span style={{ flexShrink: 0, fontSize: 10, fontFamily: "monospace", fontWeight: 700, padding: "2px 4px", borderRadius: 3, lineHeight: 1, color: methodColor(item.method), background: methodBg(item.method) }}>
              {item.method === "*" ? "ANY" : item.method}
            </span>
          )}
          <span style={{
            fontSize: 13, lineHeight: 1, flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis",
            color: "var(--c-foreground)",
            textDecoration: isDeleted || item.isBlock ? "line-through" : "none",
            opacity: !isEnabled || item.isBlock ? 0.6 : 1,
          }}>
            {item.name}
          </span>
        </>
      )}
    </div>
  );
});
