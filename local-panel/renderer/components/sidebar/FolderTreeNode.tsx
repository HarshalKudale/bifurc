import React, { memo } from "react";
import { FolderNode } from "./FolderTree.types";
import { useFolderTreeContext } from "./FolderTreeContext";
import { FolderTreeItemNode } from "./FolderTreeItemNode";
import { ChevronDown, Folder, FolderOpen } from "@/lib/icons";
import ActiveDot from "@/components/ui/ActiveDot";
import { InlineInput } from "./modals/InlineInput";
import { strings } from "@/lib/strings";

const LINE_COLOR = "var(--c-border)";
const CONNECTOR_W = 12;
const INDENT = 14;

interface Props {
  node: FolderNode;
  depth: number;
}

function countItems(node: FolderNode): number {
  return node.items.length + node.children.reduce((a, c) => a + countItems(c), 0);
}

export const FolderTreeNode = memo(function FolderTreeNode({ node, depth }: Props) {
  const ctx = useFolderTreeContext();
  const isRoot = node.folder === null;
  const nodeKey = isRoot ? "__root__" : node.folder!.id;
  const isExpanded = ctx.expanded.has(nodeKey);
  const isSel = !isRoot && ctx.selectedFolderIds.has(nodeKey);
  const isDragOverThis = ctx.dragOver === nodeKey;

  const newFolderRow = ctx.newFolderParent !== undefined && ctx.newFolderParent === (isRoot ? null : node.folder?.id) && (
    <div key="new-folder-input" style={{ display: "flex", alignItems: "center", gap: 6, height: 28, paddingLeft: depth > 0 ? CONNECTOR_W + 4 : 4, paddingRight: 8, marginLeft: 2, marginRight: 4 }}>
      <Folder size={11} style={{ color: "var(--c-muted-foreground)", flexShrink: 0 }} />
      <InlineInput
        value=""
        onCommit={(v) => ctx.handleNewFolder(v, isRoot ? null : node.folder!.id)}
        onCancel={() => ctx.cancelNewFolder()}
      />
    </div>
  );

  const sortedItems = [...node.items].sort((a, b) => {
    const aDeleted = ctx.getItemStatus(a) === "deleted" ? 1 : 0;
    const bDeleted = ctx.getItemStatus(b) === "deleted" ? 1 : 0;
    return aDeleted - bDeleted;
  });

  return (
    <div key={nodeKey}>
      <div
        draggable={!isRoot && !!ctx.hasMoveFolder}
        style={{
          position: "relative",
          display: "flex", alignItems: "center",
          height: 32, paddingRight: 8, paddingLeft: depth > 0 ? 0 : 4,
          gap: 4, borderRadius: 4, marginLeft: 2, marginRight: 4,
          cursor: "pointer", userSelect: "none", whiteSpace: "nowrap",
          background: isDragOverThis
            ? "oklch(var(--signal) / 0.15)"
            : isSel ? "oklch(var(--signal) / 0.1)" : undefined,
          outline: isDragOverThis
            ? "1px solid oklch(var(--signal) / 0.5)"
            : isSel ? "1px solid oklch(var(--signal) / 0.25)" : undefined,
        }}
        onClick={(e) => {
          e.stopPropagation();
          if (isRoot) { 
            ctx.onSelectFolder(nodeKey, false, false);
            return;
          }
          if (e.shiftKey) {
            ctx.onSelectFolder(node.folder!.id, false, true);
          } else if (e.ctrlKey || e.metaKey) {
            ctx.onSelectFolder(node.folder!.id, true, false);
          } else {
            ctx.onSelectFolder(node.folder!.id, false, false);
          }
        }}
        onContextMenu={(e) => ctx.onFolderContextMenu(e, isRoot ? null : node.folder!.id)}
        onMouseEnter={(e) => { if (!isSel && !isDragOverThis) (e.currentTarget as HTMLDivElement).style.background = "var(--c-card)"; }}
        onMouseLeave={(e) => { if (!isSel && !isDragOverThis) (e.currentTarget as HTMLDivElement).style.background = ""; }}
        onDragStart={!isRoot ? (e) => ctx.onDragStartFolder(e, node.folder!.id) : undefined}
        onDragOver={(e) => ctx.onDragOverNode(e, nodeKey)}
        onDragLeave={ctx.onDragLeaveNode}
        onDrop={(e) => ctx.onDropNode(e, isRoot ? null : node.folder!.id)}
      >
        {depth > 0 && (
          <div style={{ position: "absolute", left: -CONNECTOR_W, top: "50%", width: CONNECTOR_W - 2, height: 1, background: LINE_COLOR, transform: "translateY(-50%)", pointerEvents: "none" }} />
        )}
        <span style={{ flexShrink: 0, width: 14, display: "flex", alignItems: "center", justifyContent: "center", color: "var(--c-muted-foreground)", transition: "transform 0.15s ease", transform: isExpanded ? "rotate(0deg)" : "rotate(-90deg)" }}>
          <ChevronDown size={12} />
        </span>
        {ctx.folderStatusMap && !isRoot && node.folder && ctx.folderStatusMap[node.folder.id] && (() => {
          const fs = ctx.folderStatusMap[node.folder.id];
          return (
            <ActiveDot
              active={fs !== "disabled"}
              color={fs === "enabled" ? "green" : fs === "mixed" ? "yellow" : "red"}
              size="sm"
            />
          );
        })()}
        <span style={{ display: "flex", alignItems: "center", flexShrink: 0, color: "var(--c-muted-foreground)" }}>
          {isExpanded ? <FolderOpen size={13} /> : <Folder size={13} />}
        </span>
        {ctx.renaming === nodeKey ? (
          <span style={{ fontSize: 13, fontWeight: 500, color: "var(--c-signal)", flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis", fontStyle: "italic" }}>
            {node.folder!.name}
          </span>
        ) : (
          <span style={{ fontSize: 13, fontWeight: 500, color: "var(--c-foreground)", flex: 1, minWidth: 0, overflow: "hidden", textOverflow: "ellipsis" }}>
            {isRoot ? strings.folderTree.root : node.folder!.name}
          </span>
        )}
        {(node.items.length > 0 || node.children.length > 0) && (
          <span style={{ fontSize: 9, color: "var(--c-muted-foreground)", flexShrink: 0, fontFamily: "monospace" }}>
            {node.items.length + node.children.reduce((a, c) => a + countItems(c), 0)}
          </span>
        )}
      </div>
      <div
        className="tree-folder-body"
        style={{ gridTemplateRows: isExpanded ? "1fr" : "0fr" }}
      >
        <div className="tree-folder-inner" style={{ paddingLeft: CONNECTOR_W, marginLeft: INDENT, borderLeft: `1px solid ${LINE_COLOR}` }}>
          {newFolderRow}
          {node.children.map((child) => <FolderTreeNode key={child.folder!.id} node={child} depth={depth + 1} />)}
          {sortedItems.map((item) => <FolderTreeItemNode key={item.id} item={item} depth={depth + 1} />)}
        </div>
      </div>
    </div>
  );
});
