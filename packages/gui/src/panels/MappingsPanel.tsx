import React, { useState, useEffect, useMemo } from "react";
import { AppConfig, LocalMapping } from "@/types";
import Modal from "@/components/common/Modal";
import PanelHeader from "@/components/layout/PanelHeader";
import { strings } from "@/lib/strings";
import { useEntityPaths } from "@/lib/entityPaths";
import { entityGitView, entityGitDotColor, shortCommit } from "@/lib/entityGit";
import { ArrowLeftRight, History, AlertCircle, CheckCircle2, Plus, Globe, Server, RotateCcw, ArrowUp } from "@/lib/icons";
import { Button, IconButton, Input, FormField, EmptyState, ModalFooter, StatusDot, Switch } from "@/components/ui";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import { useDebouncedAutosave } from "@/hooks/useDebouncedAutosave";
import { DeletedItemDialog } from "@/components/sidebar/modals/DeletedItemDialog";
import type { GitWriteResult } from "@/hooks/useEntityGitActions";


interface Props {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  onRefreshServices: () => void;
  prefillTarget?: string;
  onPrefillConsumed?: () => void;
  onHistoryOpen?: (filePath: string) => void;
  /** Refresh the History sidebar — a mapping edit while it is open must not leave it stale. */
  onAfterSave?: () => void;
  entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
  /** The workspace's position relative to its remote — needed for "committed but not pushed". */
  workspaceGitState?: import("@/types").WorkspaceGitState | null;
  /**
   * Commit and push. Backs **Publish** and **Sync** — which of the two is a labelling decision made by
   * `entityGitView()`, not a second code path.
   */
  onPublish?: (entityId: string) => Promise<GitWriteResult>;
  /** Discard local changes. Also backs **Recover** for a deleted mapping. */
  onRevert?: (entityId: string) => Promise<GitWriteResult>;
  /** Whether a publish for this mapping is in flight, from the shared git-action hook. */
  isPublishing?: (entityId: string) => boolean;
}

/** The create modal's form. */
interface FormState {
  subdomain: string;
  target: string;
  label: string;
}

/** The same three fields, as held for a row that is being edited in place. */
type MappingEdit = FormState;

const EMPTY_FORM: FormState = { subdomain: "", target: "", label: "" };

const subdomainOf = (m: LocalMapping) => m.domain.replace(/\.localhost$/, "");

const fullDomain = (sub: string) =>
  sub.trim() ? `${sub.trim().toLowerCase()}.localhost` : "";

/**
 * Place an engine refusal on the form field it names. The **engine** owns a mapping's rules
 * (`validateMapping`), so every client is held to the same ones; this only maps its `details.field` onto
 * this form's field names. An error that names no field is shown on the target, beside the save.
 */
function fieldErrorsOf(err: unknown): Partial<FormState> {
  const message = err instanceof Error ? err.message : String(err);
  const field = (err as { details?: { field?: unknown } } | null)?.details?.field;
  return field === "domain" ? { subdomain: message } : { target: message };
}

export default function MappingsPanel({
  config,
  onConfigChange,
  onRefreshServices,
  prefillTarget,
  onPrefillConsumed,
  onHistoryOpen,
  onAfterSave,
  entitySyncStatus,
  workspaceGitState,
  onPublish,
  onRevert,
  isPublishing,
}: Props) {
  const { confirm, ConfirmDialogElement } = useConfirmDialog();

  /** Every change is an engine command; afterwards the panel adopts the engine's config rather than editing its own. */
  const reloadConfig = async () => onConfigChange(await window.api.getConfig());

  // The modal is create-only; existing mappings are edited on their cards.
  const [modalOpen, setModalOpen] = useState(false);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [errors, setErrors] = useState<Partial<FormState>>({});

  /**
   * In-progress inline edits, keyed by mapping id.
   *
   * Edits accumulate here and the debounce below sends them to the engine in one pass, rather than one
   * `mapping.update` per keystroke.
   */
  const [edits, setEdits] = useState<Record<string, MappingEdit>>({});
  const [rowErrors, setRowErrors] = useState<Record<string, Partial<FormState>>>({});
  const [publishErrors, setPublishErrors] = useState<Record<string, string>>({});
  /** The mapping whose pending-deletion dialog is open. */
  const [deletedPopup, setDeletedPopup] = useState<LocalMapping | null>(null);

  /**
   * What the last git action on this mapping actually did.
   *
   * Added because "clicking publish does nothing" was *accurate*: a successful publish produced no
   * feedback beyond a subtle status-label change, and a publish that staged nothing — the engine
   * correctly returns `ok: true, committed: false` for an already-committed entity — produced **none
   * at all**. The two were indistinguishable, so a no-op read as a broken button.
   *
   * A `none` notice has no timeout: it is a statement about the entity's state, and it should stay
   * until the state changes. An `ok` notice is transient feedback and clears itself.
   */
  const [gitNotice, setGitNotice] = useState<Record<string, { kind: "ok" | "none"; text: string; sha?: string }>>({});
  const noticeTimers = React.useRef<Record<string, ReturnType<typeof setTimeout>>>({});

  const showNotice = React.useCallback((id: string, kind: "ok" | "none", text: string, sha?: string) => {
    setGitNotice((prev) => ({ ...prev, [id]: { kind, text, sha } }));
    if (noticeTimers.current[id]) clearTimeout(noticeTimers.current[id]);
    if (kind === "ok") {
      noticeTimers.current[id] = setTimeout(() => {
        setGitNotice((prev) => { const next = { ...prev }; delete next[id]; return next; });
        delete noticeTimers.current[id];
      }, 3000);
    }
  }, []);

  // A timer that outlives the panel would set state on an unmounted component.
  useEffect(() => () => { Object.values(noticeTimers.current).forEach(clearTimeout); }, []);

  /**
   * The uniform git view per mapping, plus the set of mappings whose file is gone.
   *
   * **`deletedIds` is load-bearing, not cosmetic.** A deleted-but-uncommitted mapping stays in
   * `config.mappings` (that is how `loadConfig()` keeps it visible — see its pending-deletion merge),
   * so without this guard the autosave below would happily `updateMapping()` a file that no longer
   * exists and **resurrect the deletion** the user just made.
   */
  /**
   * The engine's `mappings/<id>.json` (`plan/19` WI 5 item 3). `mappings` is a flat kind, so the path has
   * no folder level — and this used to recompute it here with a hand-mirrored helper that the engine's
   * `workspaceFs` also had.
   */
  const { relPathOf } = useEntityPaths();

  const { views, deletedIds } = useMemo(() => {
    const views = new Map<string, ReturnType<typeof entityGitView>>();
    const deletedIds = new Set<string>();
    for (const m of config.mappings) {
      const v = entityGitView(entitySyncStatus?.[relPathOf(m.id)], workspaceGitState);
      views.set(m.id, v);
      if (v.status === "deleted") deletedIds.add(m.id);
    }
    return { views, deletedIds };
  }, [config.mappings, entitySyncStatus, workspaceGitState, relPathOf]);

  useEffect(() => {
    if (prefillTarget) {
      setForm({ subdomain: "", target: prefillTarget, label: "" });
      setErrors({});
      setModalOpen(true);
      onPrefillConsumed?.();
    }
  }, [prefillTarget]);

  const openAdd = () => {
    setForm(EMPTY_FORM);
    setErrors({});
    setModalOpen(true);
  };

  // -- Create ---------------------------------------------------------------

  const createMapping = async () => {
    setErrors({});
    try {
      await window.api.addMapping({
        domain: fullDomain(form.subdomain),
        target: form.target.trim(),
        label: form.label,
        enabled: true,
      });
    } catch (err) {
      setErrors(fieldErrorsOf(err));
      return;
    }
    await reloadConfig();
    setModalOpen(false);
    onRefreshServices();
    onAfterSave?.();
  };

  // -- Inline edit + autosave ----------------------------------------------

  const rowValue = (m: LocalMapping): MappingEdit =>
    edits[m.id] ?? { subdomain: subdomainOf(m), target: m.target, label: m.label ?? "" };

  const setRowField = (id: string, patch: Partial<MappingEdit>) => {
    if (deletedIds.has(id)) return;
    const m = config.mappings.find((x) => x.id === id);
    if (!m) return;
    setEdits((prev) => ({
      ...prev,
      [id]: { ...(prev[id] ?? { subdomain: subdomainOf(m), target: m.target, label: m.label ?? "" }), ...patch },
    }));
    setRowErrors((prev) => {
      if (!prev[id]) return prev;
      const next = { ...prev };
      delete next[id];
      return next;
    });
  };

  /**
   * Write every pending row in one pass.
   *
   * A row that fails validation is **skipped and left in `edits`** rather than silently written —
   * a half-typed target must not reach the config — and it keeps its error until the user fixes it.
   * Only the rows actually written are dropped, so a rejected row survives the save.
   */
  /**
   * Send every pending row to the engine, which decides whether it is valid. A refused row keeps its edit and
   * shows the engine's message; only rows the engine stored leave `edits`. Returns the ids it refused.
   */
  const applyEdits = async (): Promise<Set<string>> => {
    const refused = new Set<string>();
    const ids = Object.keys(edits).filter((id) => !deletedIds.has(id));
    if (ids.length === 0) return refused;

    const nextErrors: Record<string, Partial<FormState>> = {};
    const written: string[] = [];
    for (const id of ids) {
      const m = config.mappings.find((x) => x.id === id);
      if (!m) continue;
      const e = edits[id];
      try {
        await window.api.updateMapping({ ...m, domain: fullDomain(e.subdomain), target: e.target.trim(), label: e.label });
        written.push(id);
      } catch (err) {
        nextErrors[id] = fieldErrorsOf(err);
        refused.add(id);
      }
    }

    setRowErrors(nextErrors);
    if (written.length === 0) return refused;

    await reloadConfig();
    setEdits((prev) => {
      const next = { ...prev };
      for (const id of written) delete next[id];
      return next;
    });
    onRefreshServices();
    onAfterSave?.();
    return refused;
  };

  const { savedAt: autoSavedAt, error: autoSaveError } = useDebouncedAutosave({
    snapshot: JSON.stringify(edits),
    enabled: true,
    onSave: applyEdits,
  });

  // -- Row actions ----------------------------------------------------------

  const remove = async (id: string) => {
    const view = views.get(id);
    const m = config.mappings.find((x) => x.id === id);
    const what = m?.label || m?.domain || "this mapping";
    // The confirmation says which of the two outcomes this actually is, from the same view that
    // decides the button labels — a tracked mapping stays recoverable, an untracked one does not.
    const message = view?.deleteIsRecoverable
      ? strings.entityGit.deleteConfirmRecoverable.replace("{what}", what)
      : strings.entityGit.deleteConfirmPermanent.replace("{what}", what);
    if (!await confirm(message)) return;
    await window.api.deleteMapping(id);
    // The engine deletes the mapping and the rules that targeted it; the UI only re-reads.
    await reloadConfig();
    setEdits((prev) => {
      if (!(id in prev)) return prev;
      const next = { ...prev };
      delete next[id];
      return next;
    });
    onRefreshServices();
    onAfterSave?.();
  };

  const toggleEnabled = async (m: LocalMapping, enabled: boolean) => {
    await window.api.setEntityEnabled(config.activeWorkspaceId, "mappings", m.id, enabled);
    await reloadConfig();
  };

  const publish = async (m: LocalMapping) => {
    if (!onPublish || isPublishing?.(m.id)) return;
    setPublishErrors((prev) => { const next = { ...prev }; delete next[m.id]; return next; });
    try {
      // Persist in-progress card edits before committing the entity file; a row the engine refused is not
      // published, and already shows why.
      if ((await applyEdits()).has(m.id)) return;
      const result = await onPublish(m.id);
      if (!result.ok) {
        setPublishErrors((prev) => ({ ...prev, [m.id]: result.error ?? "Publish failed" }));
      } else if (!result.commit) {
        // `commit: null` means the engine staged nothing. Whether that is *correct* depends on the
        // status this card is showing, and the two must not disagree in silence: if the card still
        // reads "Not published", git and the status map contradict each other, and "already committed"
        // would be a claim the card itself refutes — which is exactly the shape of the report (a
        // Publish button above the words "this entity is already committed").
        const stillUnpublished = views.get(m.id)?.primary === "publish";
        showNotice(
          m.id,
          "none",
          stillUnpublished ? strings.entityGit.gitStatusDisagreement : strings.entityGit.nothingToPublish,
        );
      } else {
        // The **commit id**, which is the proof the click did something. Labelled Published or Synced
        // to match the button that was pressed, from the same view that chose its label.
        const template =
          views.get(m.id)?.primary === "sync"
            ? strings.entityGit.syncedWithCommit
            : strings.entityGit.publishedWithCommit;
        showNotice(m.id, "ok", template.replace("{sha}", shortCommit(result.commit)), result.commit);
      }
    } catch (error) {
      setPublishErrors((prev) => ({ ...prev, [m.id]: error instanceof Error ? error.message : String(error) }));
    }
  };

  const revert = async (m: LocalMapping) => {
    if (!onRevert) return;
    setPublishErrors((prev) => { const next = { ...prev }; delete next[m.id]; return next; });
    // Drop any half-typed edit first: reverting to the committed version while a keystroke is
    // still pending would let the autosave write the edit back over the restored file.
    setEdits((prev) => { if (!(m.id in prev)) return prev; const next = { ...prev }; delete next[m.id]; return next; });
    const result = await onRevert(m.id);
    if (!result.ok) setPublishErrors((prev) => ({ ...prev, [m.id]: result.error ?? "Revert failed" }));
    else showNotice(m.id, "ok", strings.entityGit.reverted);
  };

  const emptyNode = <EmptyState
    fill
    className="border border-dashed border-border rounded-xl bg-card/40"
    icon={<ArrowLeftRight size={38} />}
    title={strings.mappings.noMappingsYet}
    description={strings.mappings.noMappingsYetHint}
    action={<Button variant="primary" onClick={openAdd} icon={<Plus size={15} />}>{strings.mappings.addMapping}</Button>}
  />;

  return (
    <>
      {ConfirmDialogElement}
      <div className="flex flex-col flex-1 overflow-hidden">
        <PanelHeader
          title={strings.mappings.title}
          subtitle={strings.mappings.subtitle}
          actions={
            <div className="flex items-center gap-3">
              {autoSaveError ? (
                <span title={autoSaveError} className="flex items-center gap-1.5 text-[11px] font-medium text-destructive">
                  <AlertCircle size={12} className="flex-shrink-0" />
                  <span>{strings.common.saveFailed}</span>
                </span>
              ) : autoSavedAt ? (
                <span className="flex items-center gap-1.5 text-[11px] text-muted-foreground/70">
                  <CheckCircle2 size={12} className="flex-shrink-0" />
                  <span>{strings.common.saved}</span>
                </span>
              ) : null}
              <Button variant="primary" onClick={openAdd} icon={<Plus size={15} />}>{strings.mappings.addMapping}</Button>
            </div>
          }
        />

        <div className="flex-1 overflow-y-auto p-6">
          {config.mappings.length === 0 ? emptyNode : (
            <div className="grid gap-4" style={{ gridTemplateColumns: "repeat(auto-fill, minmax(min(100%, 320px), 1fr))" }}>
              {config.mappings.map((m) => {
                const value = rowValue(m);
                const view = views.get(m.id);
                const isDeleted = view?.status === "deleted";
                return (
                  <article
                    key={m.id}
                    className="group rounded-xl border border-border bg-card shadow-sm transition-colors hover:border-border/80"
                    onClick={isDeleted ? () => setDeletedPopup(m) : undefined}
                    style={isDeleted ? { cursor: "pointer", opacity: 0.85 } : undefined}
                  >
                    <div className="flex items-start gap-3 border-b border-border/70 px-4 py-4">
                      <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-signal/10 text-signal"><Globe size={17} /></div>
                      <div className="min-w-0 flex-1">
                        {isDeleted ? (
                          <span className="block px-1 py-0.5 text-sm font-semibold text-foreground line-through truncate">
                            {value.label || value.subdomain || m.id}
                          </span>
                        ) : (
                          <Input className="w-full border-transparent bg-transparent px-1 py-0.5 text-sm font-semibold text-foreground shadow-none hover:border-border focus:border-border" value={value.label} placeholder={strings.mappings.titlePlaceholder} aria-label={strings.mappings.columnLabel} onChange={(e) => setRowField(m.id, { label: e.target.value })} />
                        )}
                      </div>
                      <div className="flex items-center gap-2 pt-1">
                        {view && (
                          <span title={view.stateLabel} className="flex items-center gap-1.5 text-[10px] text-muted-foreground">
                            <StatusDot color={entityGitDotColor(view.status)} />{view.stateLabel}
                          </span>
                        )}
                        <Switch checked={m.enabled} onChange={(v) => toggleEnabled(m, v)} disabled={isDeleted} />
                      </div>
                    </div>
                    {isDeleted ? (
                      <div className="px-4 py-4 text-xs text-muted-foreground">
                        {strings.entityGit.pendingDeletionBody}
                      </div>
                    ) : (
                      <div className="space-y-3 px-4 py-4">
                        <FormField label={strings.mappings.subdomainLabel} error={rowErrors[m.id]?.subdomain}>
                          <div className="flex items-center"><Input className="min-w-0 flex-1 rounded-r-none font-mono text-xs" value={value.subdomain} error={!!rowErrors[m.id]?.subdomain} aria-label={strings.mappings.columnDomain} onChange={(e) => setRowField(m.id, { subdomain: e.target.value })} /></div>
                          <div className="mt-1.5 flex items-center gap-1.5 text-xs text-muted-foreground"><span className="font-mono text-signal">{fullDomain(value.subdomain)}</span></div>
                        </FormField>
                        <FormField label={strings.mappings.targetCardLabel} error={rowErrors[m.id]?.target}>
                          <div className="relative"><Server size={14} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" /><Input className="w-full pl-9 font-mono text-xs" value={value.target} error={!!rowErrors[m.id]?.target} aria-label={strings.mappings.columnTarget} onChange={(e) => setRowField(m.id, { target: e.target.value })} /></div>
                        </FormField>
                      </div>
                    )}
                    <div className="flex items-center justify-between border-t border-border/70 px-3 py-2">
                      <div className="flex items-center gap-1" onClick={(e) => e.stopPropagation()}>
                        {isDeleted ? (
                          <>
                            {onRevert && (
                              <Button variant="secondary" size="sm" icon={<RotateCcw size={13} />} title={strings.entityGit.recoverTooltip} onClick={() => void revert(m)}>
                                {strings.entityGit.recover}
                              </Button>
                            )}
                            {onPublish && (
                              <Button variant="danger" size="sm" icon={<ArrowUp size={13} />} title={strings.entityGit.syncDeleteTooltip} onClick={() => void publish(m)}>
                                {strings.entityGit.syncDelete}
                              </Button>
                            )}
                          </>
                        ) : (
                          <>
                            {/* One primary button, labelled Publish or Sync by the entity's state. */}
                            {onPublish && view?.primary && (
                              <Button variant="secondary" size="sm" disabled={isPublishing?.(m.id)} title={view.primaryTitle} onClick={() => void publish(m)}>
                                {isPublishing?.(m.id) ? strings.entityGit.publishing : view.primaryLabel}
                              </Button>
                            )}
                            {onRevert && view?.canRevert && (
                              <Button variant="ghost" size="sm" icon={<RotateCcw size={13} />} title={strings.entityGit.revertTooltip} onClick={() => void revert(m)}>
                                {strings.entityGit.revert}
                              </Button>
                            )}
                            {onHistoryOpen && view?.canViewHistory && (
                              <IconButton icon={<History size={13} />} title={strings.entityGit.historyTooltip} onClick={() => onHistoryOpen(`mappings/${m.id}.json`)} className="hover:text-signal" />
                            )}
                          </>
                        )}
                      </div>
                      <Button variant="danger" size="sm" onClick={() => void remove(m.id)}>{strings.common.delete}</Button>
                    </div>
                    {publishErrors[m.id] && <p role="alert" className="border-t border-destructive/20 px-4 py-2 text-xs text-destructive">{publishErrors[m.id]}</p>}
                    {gitNotice[m.id] && (
                      <p
                        role="status"
                        // The full id, on hover: the notice shows the 7-character form because a
                        // 40-character hash in a narrow card wraps, and the short form is what
                        // `git log --oneline` prints — but truncating without offering the whole
                        // value would lose it.
                        title={gitNotice[m.id].sha}
                        className={`border-t px-4 py-2 text-xs font-mono ${
                          gitNotice[m.id].kind === "ok"
                            ? "border-signal/20 text-signal"
                            : "border-amber/20 text-amber"
                        }`}
                      >
                        {gitNotice[m.id].text}
                      </p>
                    )}
                  </article>
                );
              })}
            </div>
          )}
        </div>

        <Modal
          open={modalOpen}
          title={strings.mappings.addMappingTitle}
          onClose={() => setModalOpen(false)}
        >
          <FormField label={strings.mappings.subdomainLabel} error={errors.subdomain}>
            <div className="flex items-center gap-0">
              <Input
                className="flex-1 rounded-l rounded-r-none min-w-0"
                placeholder={strings.mappings.subdomainPlaceholder}
                value={form.subdomain}
                error={!!errors.subdomain}
                onChange={(e) => setForm({ ...form, subdomain: e.target.value })}
                autoFocus
              />
              <span className="bg-surface-2 border border-l-0 border-border rounded-r px-3 py-2 text-sm font-mono text-muted-foreground select-none whitespace-nowrap">
                {strings.mappings.localHostSuffix}
              </span>
            </div>
            {form.subdomain.trim() && !errors.subdomain && (
              <p className="text-xs text-muted-foreground mt-1">
                {strings.mappings.willCreate} <span className="font-mono text-signal">{fullDomain(form.subdomain)}</span>
              </p>
            )}
          </FormField>

          <FormField label={strings.mappings.targetLabel} error={errors.target}>
            <Input
              className="w-full font-mono"
              placeholder={strings.mappings.targetPlaceholder}
              value={form.target}
              error={!!errors.target}
              onChange={(e) => setForm({ ...form, target: e.target.value })}
            />
          </FormField>

          <FormField label={strings.mappings.labelLabel}>
            <Input
              className="w-full"
              placeholder={strings.mappings.labelPlaceholder}
              value={form.label}
              onChange={(e) => setForm({ ...form, label: e.target.value })}
            />
          </FormField>

          {/* Create, not Save: once the mapping exists the row above edits it. */}
          <ModalFooter
            onCancel={() => setModalOpen(false)}
            onConfirm={createMapping}
            confirmLabel={strings.common.create}
          />
        </Modal>

        {/* Interacting with a deleted mapping opens the same dialog the folder tree uses. */}
        {deletedPopup && (
          <DeletedItemDialog
            name={deletedPopup.label || deletedPopup.domain || deletedPopup.id}
            onRecover={() => { void revert(deletedPopup); setDeletedPopup(null); }}
            onCommitDelete={() => { void publish(deletedPopup); setDeletedPopup(null); }}
            onClose={() => setDeletedPopup(null)}
          />
        )}
      </div>
    </>
  );
}
