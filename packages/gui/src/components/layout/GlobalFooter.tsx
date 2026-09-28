import React, { useState, useMemo, useRef } from "react";
import { GitBranch, RefreshCw, Cloud, Settings, Variable, AlertCircle, X } from "@/lib/icons";
import { Button } from "@/components/ui";
import EnvSelector from "@/components/sidebar/EnvSelector";
import { strings } from "@/lib/strings";
import { panelEntityKind, folderGitRollup } from "@/lib/entityGit";
import type { Environment, SyncStatus, Workspace, WorkspaceGitState } from "@/types";

// -- Props ------------------------------------------------------------------

interface Props {
    /** Current active panel name */
    panel: string;
    /** Active workspace (for syncConfig / branch name) */
    workspace: Workspace | null;
    /** Entity-level sync statuses from git porcelain */
    entitySyncStatus: Record<string, "clean" | "modified" | "new" | "deleted">;
    /**
     * The workspace's position relative to its remote. `null` until the first status refresh lands.
     *
     * The footer needs it for the same reason every other surface does: a panel whose entities are all
     * committed but unpushed has `changed === 0`, so the old footer said **"up to date"** and offered no
     * Publish button while the commits sat locally. `folderGitRollup` folds both axes into one answer.
     */
    workspaceGitState: WorkspaceGitState | null;
    /** Overall sync operation state */
    syncStatus: SyncStatus;
    /** Publish / sync all local changes for the current panel */
    onPublishPanel: () => Promise<unknown>;
    /**
     * The most recent git failure, rendered here because this bar is on **every** screen.
     *
     * It exists because the folder tree's context-menu actions are fire-and-forget, so before this
     * there was no surface at all for "the publish you just asked for failed" — the old handler
     * ignored the engine's `{ok:false}` and nothing said anything.
     */
    gitError?: string | null;
    onDismissGitError?: () => void;
    /** Optional right-aligned panel-specific stats */
    rightContent?: React.ReactNode;
    environments: Environment[];
    activeEnvironmentId: string | null;
    onEnvChange: (id: string | null) => Promise<void>;
    onManageEnvs: () => void;
    variablesOpen: boolean;
    onToggleVariables: () => void;
}

// -- Component --------------------------------------------------------------

export default function GlobalFooter({
    panel,
    workspace,
    entitySyncStatus,
    workspaceGitState,
    syncStatus,
    onPublishPanel,
    gitError,
    onDismissGitError,
    rightContent,
    environments,
    activeEnvironmentId,
    onEnvChange,
    onManageEnvs,
    variablesOpen,
    onToggleVariables,
}: Props) {
    const [publishing, setPublishing] = useState(false);
    const [envDropdownOpen, setEnvDropdownOpen] = useState(false);
    const envDropdownRef = useRef<HTMLDivElement>(null);

    const syncConfig = workspace?.syncConfig ?? null;
    // Use the configured remote branch when configured, otherwise fall back to the local HEAD's branch.
    const branch = syncConfig?.branch ?? workspaceGitState?.branch ?? "local";
    const hasRemote = !!syncConfig?.remote;

    /**
     * Which entities this panel publishes — from the **shared** map, not a local one.
     *
     * This file used to carry `PANEL_PREFIX` and `PANEL_ENTITY_LABEL`, and `usePublishHandlers`
     * carried a third (`PANEL_PUBLISH_KIND`). They disagreed: `webhooks` was in the handler and in
     * neither footer map, so the webhooks panel showed "up to date" with unpublishable work on screen
     * and no button to publish it with. `graphqlRequests` / `soapRequests` / `grpcRequests` were
     * missing from the prefix map too, so a pending GraphQL request was invisible to the count.
     */
    const pek = panelEntityKind(panel);

    // isSyncing only applies when a remote is connected (local-only repos don't push)
    const isSyncing =
        hasRemote &&
        (syncStatus === "pulling" || syncStatus === "pushing" || syncStatus === "cloning");

    /**
     * The panel's outstanding work, from the status map rather than from a fresh `git status` — so the
     * footer and the entity rows inside it can never disagree, even mid-publish.
     */
    const rollup = useMemo(
        () => (pek ? folderGitRollup(pek.prefixes, entitySyncStatus, workspaceGitState) : null),
        [pek, entitySyncStatus, workspaceGitState],
    );

    const outgoingCount = rollup?.changed ?? 0;
    const primary = rollup?.primary ?? null;
    const primaryLabel = primary === "publish" ? strings.entityGit.publish : strings.entityGit.sync;

    // Publish is available whenever there is something to do — no remote required (the backend
    // commits locally and only pushes if a remote is configured).
    const publishDisabled = primary === null || publishing || isSyncing;

    const handlePublish = async () => {
        setPublishing(true);
        try {
            await onPublishPanel();
        } finally {
            setPublishing(false);
        }
    };

    const pluralLabel = outgoingCount !== 1 ? `${pek?.label ?? "entity"}s` : (pek?.label ?? "entity");

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
                    {outgoingCount > 0 && (
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

        // Something to do: local changes, or commits that are not on the remote yet.
        if (primary !== null) {
            // No remote: commit-only (no push)
            const changeLabel = hasRemote
                ? `${pluralLabel} ${strings.footer.changesWaitingToPublish}`
                : `${pluralLabel} ${strings.footer.changesNotYetCommitted}`;
            const unpushedOnly = outgoingCount === 0;
            return (
                <div className="flex items-center gap-1.5">
                    <Button
                        variant="secondary"
                        size="sm"
                        icon={<Cloud size={10} />}
                        disabled={publishDisabled}
                        onClick={handlePublish}
                        className="flex-shrink-0 !text-[10px] !py-0.5 !px-2"
                        title={
                            unpushedOnly ? strings.entityGit.pushTooltip
                            : !hasRemote ? strings.footer.commitLocalChangesTitle
                            : strings.entityGit.syncTooltip
                        }
                    >
                        {publishing ? strings.footer.publishing : primaryLabel}
                    </Button>
                    <GitBranch size={10} className="text-signal flex-shrink-0" />
                    <span className="text-muted-foreground text-[10px]">{hasRemote ? strings.footer.sync : strings.footer.local}</span>
                    <span className="font-mono text-signal text-[10px]">{branch}</span>
                    {unpushedOnly ? (
                        <>
                            <span className="text-muted-foreground/50 text-[10px]">·</span>
                            <span className="text-amber text-[10px]">{strings.entityGit.stateUnpushed}</span>
                        </>
                    ) : (
                        <>
                            <span className="text-muted-foreground/50 text-[10px]">·</span>
                            <span className="font-semibold text-amber text-[10px]">{outgoingCount}</span>
                            <span className="text-muted-foreground text-[10px]">{changeLabel}</span>
                        </>
                    )}
                </div>
            );
        }

        // Clean / up to date
        return (
            <div className="flex items-center gap-1.5">
                <GitBranch size={10} className="text-muted-foreground flex-shrink-0" />
                <span className="text-muted-foreground text-[10px]">{hasRemote ? strings.footer.sync : strings.footer.local}</span>
                {hasRemote && (
                    <span className="font-mono text-muted-foreground text-[10px]">{branch}</span>
                )}
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
                {gitError && (
                    <div
                        role="alert"
                        className="flex items-center gap-1.5 min-w-0 text-destructive"
                        title={gitError}
                    >
                        <AlertCircle size={10} className="flex-shrink-0" />
                        <span className="text-[10px] truncate max-w-[38ch]">{gitError}</span>
                        {onDismissGitError && (
                            <button
                                type="button"
                                onClick={onDismissGitError}
                                aria-label={strings.common.close}
                                className="flex-shrink-0 text-muted-foreground hover:text-foreground cursor-pointer"
                            >
                                <X size={10} />
                            </button>
                        )}
                    </div>
                )}
            </div>


            {/* Right: panel-specific stats */}
            <div className="flex items-center gap-2 flex-shrink-0 min-w-0">
                {rightContent && (
                    <div className="flex items-center gap-2 text-[10px] text-muted-foreground font-mono min-w-0">
                        {rightContent}
                    </div>
                )}
                <EnvSelector
                    environments={environments.filter((e) => e.workspaceId === workspace?.id && e.id !== "__global__")}
                    activeId={activeEnvironmentId}
                    open={envDropdownOpen}
                    dropdownRef={envDropdownRef}
                    onToggle={() => setEnvDropdownOpen((v) => !v)}
                    onClose={() => setEnvDropdownOpen(false)}
                    onSelect={async (id) => {
                        await onEnvChange(id);
                        setEnvDropdownOpen(false);
                    }}
                />
                <button
                    type="button"
                    onClick={onToggleVariables}
                    title={strings.rightSidebar.toggleVariables}
                    aria-label={strings.rightSidebar.toggleVariables}
                    aria-pressed={variablesOpen}
                    className={`flex h-8 w-8 items-center justify-center rounded-md border transition-colors cursor-pointer flex-shrink-0 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-signal/35 ${variablesOpen ? "border-signal/40 bg-signal/15 text-signal" : "border-border/80 bg-card/60 text-muted-foreground hover:bg-surface-2 hover:text-foreground"}`}
                >
                    <Variable size={13} />
                </button>
                <button
                    type="button"
                    onClick={onManageEnvs}
                    title={strings.titleBar.manageEnvironments}
                    aria-label={strings.titleBar.manageEnvironments}
                    aria-current={panel === "environments" ? "page" : undefined}
                    className={`flex h-8 w-8 items-center justify-center rounded-md border transition-colors cursor-pointer flex-shrink-0 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-signal/35 ${panel === "environments" ? "border-signal/40 bg-signal/15 text-signal" : "border-border/80 bg-card/60 text-muted-foreground hover:bg-surface-2 hover:text-foreground"}`}
                >
                    <Settings size={13} />
                </button>
            </div>
        </div>
    );
}
