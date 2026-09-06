import React, { useState, useRef, useEffect } from "react";
import { Panel, PanelEntry } from "@/lib/panelRegistry";
import { Workspace } from "@/types";
import NavItem from "@/components/sidebar/NavItem";
import { Plus, Pencil, Trash2, Settings } from "@/lib/icons";
import { strings } from "@/lib/strings";
import { Button, Input } from "@/components/ui";

interface Props {
    entries: PanelEntry[];
    activePanel: Panel;
    onPanelSelect: (id: Panel) => void;
    badges: Partial<Record<Panel, number | undefined>>;
    workspaces: Workspace[];
    activeWorkspaceId: string;
    onWorkspaceChange: (id: string) => void;
    onWorkspaceCreate: () => void;
    onWorkspaceRename: (id: string, name: string) => void;
    onWorkspaceDelete: (id: string) => void;
    collapsed?: boolean;
}

/** Generate up to 2-letter initials from a workspace name */
function wsInitials(name: string): string {
    const words = name.trim().split(/\s+/);
    return words.slice(0, 2).map((w) => w[0] ?? "").join("").toUpperCase() || "WS";
}

export default function AppSidebar({
    entries, activePanel, onPanelSelect, badges,
    workspaces, activeWorkspaceId,
    onWorkspaceChange, onWorkspaceCreate, onWorkspaceRename, onWorkspaceDelete,
}: Props) {
    const [wsSwitcherOpen, setWsSwitcherOpen] = useState(false);
    const [renamingId, setRenamingId] = useState<string | null>(null);
    const [renameValue, setRenameValue] = useState("");
    const [confirmDeleteId, setConfirmDeleteId] = useState<string | null>(null);

    const switcherRef = useRef<HTMLDivElement>(null);
    const renameRef = useRef<HTMLInputElement>(null);

    const activeWs = workspaces.find((w) => w.id === activeWorkspaceId);
    const canDelete = workspaces.length > 1;

    // Close dropdown on outside click
    useEffect(() => {
        if (!wsSwitcherOpen) return;
        const handler = (e: MouseEvent) => {
            if (wsSwitcherOpen && switcherRef.current && !switcherRef.current.contains(e.target as Node)) {
                setWsSwitcherOpen(false);
                setRenamingId(null);
                setConfirmDeleteId(null);
            }
        };
        document.addEventListener("mousedown", handler);
        return () => document.removeEventListener("mousedown", handler);
    }, [wsSwitcherOpen]);

    // Focus rename input when rename mode activates
    useEffect(() => {
        if (renamingId) renameRef.current?.focus();
    }, [renamingId]);

    const handleRenameCommit = (id: string) => {
        const name = renameValue.trim();
        if (name) onWorkspaceRename(id, name);
        setRenamingId(null);
        setRenameValue("");
    };

    const handleDeleteConfirm = (id: string) => {
        onWorkspaceDelete(id);
        setConfirmDeleteId(null);
        setWsSwitcherOpen(false);
    };

    function renderWorkspaceSwitcher() {
        return (
            <>
                <div
                    className="fixed inset-0 z-40"
                    onClick={() => { setWsSwitcherOpen(false); setRenamingId(null); setConfirmDeleteId(null); }}
                />
                <div
                    ref={switcherRef}
                    className="absolute bottom-full left-1 z-50 bg-card border border-border rounded-md shadow-2xl py-1 animate-scale-in"
                    style={{ minWidth: "192px" }}
                >
                    <Button
                        variant="ghost"
                        size="sm"
                        onClick={() => { onWorkspaceCreate(); setWsSwitcherOpen(false); }}
                        className="w-full justify-start rounded-none border-b border-border/60 px-3 text-sm text-signal hover:bg-surface-2"
                    >
                        <Plus size={12} />
                        {strings.sidebar.createWorkspace}
                    </Button>

                    <div className="px-3 py-1 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                        {strings.sidebar.workspaces}
                    </div>

                    {workspaces.map((ws) => {
                        const isConfirming = confirmDeleteId === ws.id;
                        return (
                            <div key={ws.id}>
                                <div
                                    className={`group flex items-center gap-2 px-3 py-1.5 text-xs cursor-pointer hover:bg-surface-2 ${activeWorkspaceId === ws.id ? "text-signal font-semibold" : "text-foreground"}`}
                                    onClick={() => {
                                        if (renamingId !== ws.id && !isConfirming) {
                                            onWorkspaceChange(ws.id);
                                            setWsSwitcherOpen(false);
                                            setRenamingId(null);
                                            setConfirmDeleteId(null);
                                        }
                                    }}
                                >
                                    <span
                                        className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${activeWorkspaceId === ws.id ? "bg-signal" : "bg-muted-foreground/30"}`}
                                        style={{ boxShadow: activeWorkspaceId === ws.id ? "0 0 4px var(--c-signal)" : "none" }}
                                    />
                                    {renamingId === ws.id ? (
                                        <Input
                                            ref={renameRef}
                                            inputSize="sm"
                                            className="flex-1 min-w-0 bg-surface-2 border-signal/40"
                                            value={renameValue}
                                            onChange={(e) => setRenameValue(e.target.value)}
                                            onClick={(e) => e.stopPropagation()}
                                            onKeyDown={(e) => {
                                                e.stopPropagation();
                                                if (e.key === "Enter") handleRenameCommit(ws.id);
                                                if (e.key === "Escape") { setRenamingId(null); setRenameValue(""); }
                                            }}
                                            onBlur={() => handleRenameCommit(ws.id)}
                                        />
                                    ) : (
                                        <span className="flex-1 truncate">{ws.name}</span>
                                    )}
                                    {renamingId !== ws.id && (
                                        <div className="flex items-center gap-0.5 opacity-0 group-hover:opacity-100 transition-all">
                                            <button
                                                type="button"
                                                className="text-muted-foreground hover:text-signal text-[10px] px-1 py-0.5 rounded transition-colors cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-signal/35"
                                                title={strings.sidebar.renameWorkspace}
                                                onClick={(e) => {
                                                    e.stopPropagation();
                                                    setRenamingId(ws.id);
                                                    setRenameValue(ws.name);
                                                    setConfirmDeleteId(null);
                                                }}
                                            >
                                                <Pencil size={10} />
                                            </button>
                                            {canDelete && (
                                                <button
                                                    type="button"
                                                    className="text-muted-foreground hover:text-destructive text-[10px] px-1 py-0.5 rounded transition-colors cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-destructive/35"
                                                    title={strings.sidebar.deleteWorkspace}
                                                    onClick={(e) => {
                                                        e.stopPropagation();
                                                        setConfirmDeleteId(isConfirming ? null : ws.id);
                                                        setRenamingId(null);
                                                    }}
                                                >
                                                    <Trash2 size={10} />
                                                </button>
                                            )}
                                        </div>
                                    )}
                                </div>

                                {isConfirming && (
                                    <div
                                        className="mx-3 mb-1.5 px-2.5 py-2 rounded border border-destructive/30 bg-destructive/5"
                                        onClick={(e) => e.stopPropagation()}
                                    >
                                        <p className="text-[10px] text-muted-foreground leading-snug mb-2">
                                            {strings.sidebar.deleteWorkspacePrefix} <span className="font-semibold text-foreground">"{ws.name}"</span>{strings.sidebar.deleteWorkspaceSuffix}
                                        </p>
                                        <div className="flex gap-1.5">
                                            <Button
                                                variant="danger"
                                                size="sm"
                                                onClick={() => handleDeleteConfirm(ws.id)}
                                                className="flex-1 border border-destructive/40 bg-destructive/15 justify-center text-[10px] hover:bg-destructive/25"
                                            >
                                                {strings.common.delete}
                                            </Button>
                                            <Button
                                                variant="secondary"
                                                size="sm"
                                                onClick={() => setConfirmDeleteId(null)}
                                                className="flex-1 justify-center text-[10px]"
                                            >
                                                {strings.common.cancel}
                                            </Button>
                                        </div>
                                    </div>
                                )}
                            </div>
                        );
                    })}
                </div>
            </>
        );
    }

    return (
        <div className="flex flex-col h-full overflow-hidden">
            {/* Flat list of individual nav items (scrollable) */}
            <div className="flex-1 overflow-y-auto overflow-x-hidden p-1.5 flex flex-col items-center gap-1.5">
                {entries.map((n) => (
                    <NavItem
                        key={n.id}
                        id={n.id}
                        label={n.label}
                        icon={n.icon}
                        active={activePanel === n.id}
                        badge={badges[n.id]}
                        onClick={() => onPanelSelect(n.id)}
                    />
                ))}
            </div>

            {/* Sticky workspace footer */}
            <div className="flex-shrink-0 border-t border-border bg-surface p-1.5 flex flex-col items-center gap-1 relative w-full">
                {/* Settings icon button */}
                <button
                    type="button"
                    className={`w-full min-h-[40px] py-1 px-1 flex flex-col items-center justify-center rounded-lg transition-colors cursor-pointer select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-signal/35 ${
                        activePanel === "settings"
                            ? "bg-signal/15 text-signal border border-signal/25"
                            : "text-muted-foreground hover:text-foreground hover:bg-surface-2/70 border border-transparent"
                    }`}
                    onClick={() => onPanelSelect("settings")}
                    title={strings.nav.settings}
                    aria-label={strings.nav.settings}
                    style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
                >
                    <Settings size={18} />
                    <span className="text-[9px] font-medium leading-tight mt-0.5 tracking-tight truncate max-w-full">
                        {strings.nav.settings}
                    </span>
                </button>

                {/* Workspace avatar button */}
                <button
                    type="button"
                    className="w-full min-h-[40px] py-1 px-1 flex flex-col items-center justify-center rounded-lg hover:bg-surface-2/70 transition-colors cursor-pointer select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-signal/35"
                    onClick={() => setWsSwitcherOpen((v) => !v)}
                    aria-haspopup="menu"
                    aria-expanded={wsSwitcherOpen}
                    title={activeWs?.name ?? strings.sidebar.workspace}
                    style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
                >
                    <span
                        className="w-5 h-5 rounded flex items-center justify-center text-[9px] font-bold flex-shrink-0 select-none shadow-sm"
                        style={{ background: "var(--c-signal)", color: "var(--c-background)" }}
                    >
                        {wsInitials(activeWs?.name ?? "")}
                    </span>
                    <span className="text-[9px] text-muted-foreground font-medium leading-tight mt-0.5 tracking-tight truncate max-w-full">
                        {activeWs?.name ?? strings.sidebar.workspace}
                    </span>
                </button>

                {/* Workspace switcher dropdown */}
                {wsSwitcherOpen && renderWorkspaceSwitcher()}
            </div>
        </div>
    );
}
