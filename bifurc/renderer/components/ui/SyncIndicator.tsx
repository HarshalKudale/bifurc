import React from "react";
import { Pencil, Plus, Trash2 } from "@/lib/icons";

export type EntitySyncStatus = "clean" | "modified" | "new" | "deleted";

interface SyncIndicatorProps {
  status: EntitySyncStatus | undefined;
  size?: number;
}

export default function SyncIndicator({ status, size = 9 }: SyncIndicatorProps) {
  if (!status || status === "clean") return null;
  const icon = (() => {
    if (status === "modified") return <Pencil size={size} style={{ color: "var(--c-amber)" }} />;
    if (status === "new")      return <Plus   size={size} style={{ color: "var(--c-destructive)" }} />;
    if (status === "deleted")  return <Trash2 size={size} style={{ color: "var(--c-amber)" }} />;
    return null;
  })();
  return (
    <span style={{ display: "inline-flex", alignItems: "center", justifyContent: "center", width: 14, flexShrink: 0 }}>
      {icon}
    </span>
  );
}
