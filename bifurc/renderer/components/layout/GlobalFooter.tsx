import React, { useState, useMemo } from "react";
import { GitBranch, RefreshCw, Cloud } from "@/lib/icons";
import { Button } from "@/components/ui";
import { strings } from "@/lib/strings";
import type { SyncStatus, Workspace } from "@/types";

// -- Panel -> git path prefix ------------------------------------------------

const PANEL_PREFIX: Record<string, string> = {
    mocks: "mocks/",
    requests: "requests/",
    sockets: "sockets/",
    mappings: "mappings/",
    rules: "rules/",
    environments: "environments/",
    healthbar: "healthbar/",
};

const PANEL_ENTITY_LABEL: Record<string, string> = {
    mocks: "mock",
    requests: "request",
    sockets: "socket",
    mappings: "mapping",
    rules: "rule",
    environments: "environment",
    healthbar: "health check",
};

// -- Props ------------------------------------------------------------------

interface Props {
    /** Current active panel name */
    panel: string;
    /** Active workspace (for syncConfig / branch name) */
    workspace: Workspace | null;
    /** Entity-level sync statuses from git porcelain */
    entitySyncStatus: Record<string, "clean" | "modified" | "new" | "deleted">;
    /** Overall sync operation state */
    syncStatus: SyncStatus;
    /** Publish all local changes for the current panel */
    onPublishPanel: () => Promise<void>;
    /** Optional right-aligned panel-specific stats */
    rightContent?: React.ReactNode;
}

// -- Component --------------------------------------------------------------

export default function GlobalFooter({
    panel,
    workspace,
    entitySyncStatus,
    syncStatus,
    onPublishPanel,
    rightContent,
}: Props) {
    const [publishing, setPublishing] = useState(false);

    const syncConfig = workspace?.syncConfig ?? null;
    // Use the configured remote branch when connected, otherwise fall back to "local"
    const branch = syncConfig?.branch ?? "local";
    const hasRemote = !!syncConfig?.remote;
    const prefix = PANEL_PREFIX[panel] ?? null;
    const entityLabel = PANEL_ENTITY_LABEL[panel] ?? panel;

    // isSyncing only applies when a remote is connected (local-only repos don't push)
    const isSyncing =
        hasRemote &&
        (syncStatus === "pulling" || syncStatus === "pushing" || syncStatus === "cloning");

    // Count uncommitted (modified / new / deleted) entities for the current panel
    const outgoingCount = useMemo(() => {
        if (!prefix) return 0;
        return Object.entries(entitySyncStatus).filter(
            ([p, s]) => p.startsWith(prefix) && s !== "clean",
        ).length;
    }, [entitySyncStatus, prefix]);

    // Publish is available whenever there are local changes - no remote required
    // (backend commits locally and only pushes if a remote is configured)
    const publishDisabled = outgoingCount === 0 || publishing || isSyncing;

    const handlePublish = async () => {
        setPublishing(true);
        try {
            await onPublishPanel();
        } finally {
            setPublishing(false);
        }
    };

    const pluralLabel = outgoingCount !== 1 ? `${entityLabel}s` : entityLabel;

    // -- Left sync info -----------------------------------------------------

    const renderSyncInfo = () => {
        // Syncing (remote push/pull in flight)
        if (isSyncing) {
            return (
                <div className="flex items-center gap-1.5">
                    <RefreshCw size={10} className="animate-spin text-amber flex-shrink-0" />
                    <span className="text-amber text-[10px]">{strings.footer.syncing}</span>
                    <GitBranch size={10} className="text-signal flex-shrink-0" />
                    <span className="font-mono text-signal text-[10px]">{branch}</span>
                    {outgoingCount > 0 && prefix && (
                        <>
                            <span className="text-muted-foreground/50 text-[10px]">·</span>
                            <span className="font-semibold text-foreground text-[10px]">{outgoingCount}</span>
                            <span className="text-muted-foreground text-[10px]">
                                {pluralLabel} {strings.footer.changesWaitingToPublish}
                            </span>
                        </>
                    )}
                </div>
            );
        }

        // Outgoing uncommitted changes
        if (outgoingCount > 0 && prefix) {
            // No remote: commit-only (no push)
            const changeLabel = hasRemote
                ? `${pluralLabel} ${strings.footer.changesWaitingToPublish}`
                : `${pluralLabel} ${strings.footer.changesNotYetCommitted}`;
            return (
                <div className="flex items-center gap-1.5">
                    {/* Publish button - shown for all panels with publishable entities */}
                    {prefix && (
                        <Button
                            variant="secondary"
                            size="sm"
                            icon={<Cloud size={10} />}
                            disabled={publishDisabled}
                            onClick={handlePublish}
                            className="flex-shrink-0 !text-[10px] !py-0.5 !px-2"
                            title={!hasRemote ? strings.footer.commitLocalChangesTitle : undefined}
                        >
                            {publishing ? strings.footer.publishing : strings.footer.publish}
                        </Button>
                    )}
                    <GitBranch size={10} className="text-signal flex-shrink-0" />
                    <span className="text-muted-foreground text-[10px]">{hasRemote ? strings.footer.sync : strings.footer.local}</span>
                    <span className="font-mono text-signal text-[10px]">{branch}</span>
                    <span className="text-muted-foreground/50 text-[10px]">·</span>
                    <span className="font-semibold text-amber text-[10px]">{outgoingCount}</span>
                    <span className="text-muted-foreground text-[10px]">{changeLabel}</span>

                </div>
            );
        }

        // Clean / up to date
        return (
            <div className="flex items-center gap-1.5">
                <GitBranch size={10} className="text-muted-foreground flex-shrink-0" />
                <span className="text-muted-foreground text-[10px]">{hasRemote ? strings.footer.sync : strings.footer.local}</span>
                <span className="font-mono text-muted-foreground text-[10px]">{branch}</span>
                <span className="text-muted-foreground/50 text-[10px]">·</span>
                <span className="text-signal text-[10px]">{strings.footer.upToDate}</span>
            </div>
        );
    };

    // -- Render -------------------------------------------------------------

    return (
        <div className="border-t border-border bg-background flex min-h-11 h-11 items-center gap-3 px-4 flex-shrink-0 select-none z-20">
            {/* Left: git / sync status */}
            <div className="flex items-center gap-1.5 flex-1 min-w-0 overflow-hidden">
                {renderSyncInfo()}
            </div>


            {/* Right: panel-specific stats */}
            {rightContent && (
                <div className="flex items-center gap-2 text-[10px] text-muted-foreground font-mono flex-shrink-0">
                    {rightContent}
                </div>
            )}
        </div>
    );
}
