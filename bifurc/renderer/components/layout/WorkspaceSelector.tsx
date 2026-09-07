import React, { useState, useRef, useEffect, useMemo } from "react";
import { Workspace } from "@/types";
import { ChevronDown, Plus, Pencil, Trash2, Search } from "@/lib/icons";
import { strings } from "@/lib/strings";
import { Button, Input } from "@/components/ui";
import WorkspaceDropdown from "./WorkspaceDropdown";

interface Props {
  workspaces: Workspace[];
  activeId: string;
  onSelect: (id: string) => void;
  onCreate: () => void;
  onRename: (id: string, name: string) => void;
  onDelete: (id: string) => void;
}

/** Generate up to 2-letter initials from a workspace name */
function wsInitials(name: string): string {
  const words = name.trim().split(/\s+/);
  return words.slice(0, 2).map((w) => w[0] ?? "").join("").toUpperCase() || "WS";
}

export default function WorkspaceSelector({
  workspaces,
  activeId,
  onSelect,
  onCreate,
  onRename,
  onDelete,
}: Props) {
  const [open, setOpen] = useState(false);
  const [search, setSearch] = useState("");
  const [renamingId, setRenamingId] = useState<string | null>(null);
  const [renameValue, setRenameValue] = useState("");
  const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

  const dropdownRef = useRef<HTMLDivElement>(null);
  const searchInputRef = useRef<HTMLInputElement>(null);
  const renameRef = useRef<HTMLInputElement>(null);

  const active = workspaces.find((w) => w.id === activeId);
  const canDelete = workspaces.length > 1;

  // Auto-focus search input when opened
  useEffect(() => {
    if (open) {
      setSearch("");
      setRenamingId(null);
      setConfirmDeleteId(null);
      setTimeout(() => searchInputRef.current?.focus(), 50);
    }
  }, [open]);

  // Focus rename input when rename mode activates
  useEffect(() => {
    if (renamingId) renameRef.current?.focus();
  }, [renamingId]);

  // Close on outside click
  useEffect(() => {
    if (!open) return;
    const handler = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        close();
      }
    };
    document.addEventListener("mousedown", handler);
    return () => document.removeEventListener("mousedown", handler);
  }, [open]);

  // Close on Escape key
  useEffect(() => {
    if (!open) return;
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        close();
      }
    };
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [open]);

  const close = () => {
    setOpen(false);
    setSearch("");
    setRenamingId(null);
    setConfirmDeleteId(null);
  };

  const handleRenameCommit = (id: string) => {
    const name = renameValue.trim();
    if (name) onRename(id, name);
    setRenamingId(null);
    setRenameValue("");
  };

  const handleDeleteConfirm = (id: string) => {
    onDelete(id);
    setConfirmDeleteId(null);
    close();
  };

  const filteredWorkspaces = useMemo(() => {
    const q = search.trim().toLowerCase();
    if (!q) return workspaces;
    return workspaces.filter((ws) => ws.name.toLowerCase().includes(q));
  }, [workspaces, search]);

  return (
    <div
      ref={dropdownRef}
      className="relative flex-shrink-0"
      style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
    >
      {/* Trigger button */}
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-haspopup="menu"
        aria-expanded={open}
        title={active?.name ? `Workspace: ${active.name}` : strings.sidebar.workspace}
        className={`flex min-h-8 items-center gap-1.5 px-2.5 py-1 rounded-md border text-xs font-medium transition-colors cursor-pointer select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-signal/35 ${
          open
            ? "border-signal/40 bg-signal/10 text-signal"
            : "border-border/80 bg-card/60 text-muted-foreground hover:bg-surface-2 hover:text-foreground"
        }`}
      >
        <span
          className="w-4 h-4 rounded flex items-center justify-center text-[8.5px] font-bold flex-shrink-0 select-none shadow-xs"
          style={{ background: "var(--c-signal)", color: "var(--c-background)" }}
        >
          {wsInitials(active?.name ?? "")}
        </span>
        <span className="max-w-[130px] truncate text-foreground font-medium">
          {active?.name ?? strings.sidebar.workspace}
        </span>
        <ChevronDown size={11} className="opacity-60 flex-shrink-0" />
      </button>

      {/* Dropdown Menu */}
      {open && (
        <WorkspaceDropdown
          workspaces={workspaces}
          activeId={activeId}
          search={search}
          setSearch={setSearch}
          renamingId={renamingId}
          setRenamingId={setRenamingId}
          renameValue={renameValue}
          setRenameValue={setRenameValue}
          confirmDeleteId={confirmDeleteId}
          setConfirmDeleteId={setConfirmDeleteId}
          close={close}
          onSelect={onSelect}
          onCreate={onCreate}
          onRename={onRename}
          onDelete={onDelete}
          canDelete={canDelete}
        />
      )}
    </div>
  );
}
