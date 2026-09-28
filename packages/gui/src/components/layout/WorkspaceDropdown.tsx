import React, { useRef, useEffect, useMemo, useState } from "react";
import { Workspace } from "@/types";
import { Plus, Pencil, Trash2, Search } from "@/lib/icons";
import { strings } from "@/lib/strings";
import { Button, Input } from "@/components/ui";

interface DropdownProps {
  workspaces: Workspace[];
  activeId: string;
  search: string;
  setSearch: (s: string) => void;
  renamingId: string | null;
  setRenamingId: (id: string | null) => void;
  renameValue: string;
  setRenameValue: (s: string) => void;
  confirmDeleteId: string | null;
  setConfirmDeleteId: (id: string | null) => void;
  close: () => void;
  onSelect: (id: string) => void;
  onCreate: () => void;
  onRename: (id: string, name: string) => void;
  onDelete: (id: string) => void;
  canDelete: boolean;
}

export default function WorkspaceDropdown({
  workspaces,
  activeId,
  search,
  setSearch,
  renamingId,
  setRenamingId,
  renameValue,
  setRenameValue,
  confirmDeleteId,
  setConfirmDeleteId,
  close,
  onSelect,
  onCreate,
  onRename,
  onDelete,
  canDelete,
}: DropdownProps) {
  const searchInputRef = useRef<HTMLInputElement>(null);
  const renameRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    setTimeout(() => searchInputRef.current?.focus(), 50);
  }, []);

  useEffect(() => {
    if (renamingId) renameRef.current?.focus();
  }, [renamingId]);

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
    <>
      <div className="fixed inset-0 z-40" onClick={close} />
      <div
        className="absolute top-full mt-1.5 left-0 z-50 bg-card border border-border rounded-lg shadow-2xl py-1.5 w-64 animate-scale-in flex flex-col"
        style={{ maxHeight: "360px" }}
      >
        <div className="px-2 pb-1.5 border-b border-border/60 flex items-center gap-1.5">
          <Search size={12} className="text-muted-foreground flex-shrink-0 ml-1" />
          <input
            ref={searchInputRef}
            type="text"
            placeholder="Search workspaces…"
            value={search}
            onChange={(e) => setSearch(e.target.value)}
            className="w-full bg-transparent text-xs text-foreground placeholder:text-muted-foreground/60 outline-none px-1 py-1"
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                if (search) {
                  e.stopPropagation();
                  setSearch("");
                }
              }
            }}
          />
          {search && (
            <button
              type="button"
              onClick={() => setSearch("")}
              className="text-[10px] text-muted-foreground hover:text-foreground px-1"
            >
              ✕
            </button>
          )}
        </div>

        <div className="px-1.5 py-1 border-b border-border/60">
          <Button
            variant="ghost"
            size="sm"
            onClick={() => {
              onCreate();
              close();
            }}
            className="w-full justify-start text-xs text-signal hover:bg-surface-2 px-2 py-1 gap-1.5"
          >
            <Plus size={12} />
            {strings.titleBar.createWorkspace}
          </Button>
        </div>

        <div className="px-3 pt-1.5 pb-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
          {strings.titleBar.workspaces} ({filteredWorkspaces.length})
        </div>

        <div className="flex-1 overflow-y-auto px-1">
          {filteredWorkspaces.length === 0 ? (
            <div className="px-3 py-4 text-center text-xs text-muted-foreground">
              No workspaces found
            </div>
          ) : (
            filteredWorkspaces.map((ws) => {
              const isConfirming = confirmDeleteId === ws.id;
              const isRenaming = renamingId === ws.id;
              const isActive = activeId === ws.id;

              return (
                <div key={ws.id} className="my-0.5">
                  <div
                    className={`group flex items-center gap-2 px-2.5 py-1.5 rounded text-xs cursor-pointer hover:bg-surface-2 ${
                      isActive ? "text-signal font-semibold bg-signal/5" : "text-foreground"
                    }`}
                    onClick={() => {
                      if (!isRenaming && !isConfirming) {
                        onSelect(ws.id);
                        close();
                      }
                    }}
                  >
                    <span
                      className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${
                        isActive ? "bg-signal" : "bg-muted-foreground/30"
                      }`}
                      style={{
                        boxShadow: isActive ? "0 0 4px var(--c-signal)" : "none",
                      }}
                    />

                    {isRenaming ? (
                      <Input
                        ref={renameRef}
                        inputSize="sm"
                        className="flex-1 min-w-0 bg-surface-2 border-signal/40 py-0.5 text-xs"
                        value={renameValue}
                        onChange={(e) => setRenameValue(e.target.value)}
                        onClick={(e) => e.stopPropagation()}
                        onKeyDown={(e) => {
                          e.stopPropagation();
                          if (e.key === "Enter") handleRenameCommit(ws.id);
                          if (e.key === "Escape") {
                            setRenamingId(null);
                            setRenameValue("");
                          }
                        }}
                        onBlur={() => handleRenameCommit(ws.id)}
                      />
                    ) : (
                      <span className="flex-1 truncate">{ws.name}</span>
                    )}

                    {!isRenaming && (
                      <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-opacity">
                        <button
                          type="button"
                          className="text-muted-foreground hover:text-signal text-[10px] p-1 rounded transition-colors cursor-pointer"
                          title={strings.sidebar.renameWorkspace}
                          onClick={(e) => {
                            e.stopPropagation();
                            setRenamingId(ws.id);
                            setRenameValue(ws.name);
                            setConfirmDeleteId(null);
                          }}
                        >
                          <Pencil size={11} />
                        </button>
                        {canDelete && (
                          <button
                            type="button"
                            className="text-muted-foreground hover:text-destructive text-[10px] p-1 rounded transition-colors cursor-pointer"
                            title={strings.sidebar.deleteWorkspace}
                            onClick={(e) => {
                              e.stopPropagation();
                              setConfirmDeleteId(isConfirming ? null : ws.id);
                              setRenamingId(null);
                            }}
                          >
                            <Trash2 size={11} />
                          </button>
                        )}
                      </div>
                    )}
                  </div>

                  {isConfirming && (
                    <div
                      className="mx-2 my-1 p-2 rounded border border-destructive/30 bg-destructive/5"
                      onClick={(e) => e.stopPropagation()}
                    >
                      <p className="text-[10px] text-muted-foreground leading-snug mb-2">
                        {strings.sidebar.deleteWorkspacePrefix}{" "}
                        <span className="font-semibold text-foreground">"{ws.name}"</span>
                        {strings.sidebar.deleteWorkspaceSuffix}
                      </p>
                      <div className="flex gap-1.5">
                        <Button
                          variant="danger"
                          size="sm"
                          onClick={() => handleDeleteConfirm(ws.id)}
                          className="flex-1 justify-center text-[10px] py-0.5"
                        >
                          {strings.common.delete}
                        </Button>
                        <Button
                          variant="secondary"
                          size="sm"
                          onClick={() => setConfirmDeleteId(null)}
                          className="flex-1 justify-center text-[10px] py-0.5"
                        >
                          {strings.common.cancel}
                        </Button>
                      </div>
                    </div>
                  )}
                </div>
              );
            })
          )}
        </div>
      </div>
    </>
  );
}
