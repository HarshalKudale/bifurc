import React, { useState, useEffect } from "react";
import { AppConfig, LocalMapping } from "@/types";
import Modal from "@/components/common/Modal";
import PanelHeader from "@/components/layout/PanelHeader";
import { strings } from "@/lib/strings";
import { flatEntityRelPath } from "@/lib/utils";
import { ArrowLeftRight, History, AlertCircle, CheckCircle2, Plus, Globe, Server, RotateCcw } from "@/lib/icons";
import { Button, IconButton, Input, FormField, EmptyState, ModalFooter, StatusDot, Switch } from "@/components/ui";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import { useDebouncedAutosave } from "@/hooks/useDebouncedAutosave";


interface Props {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  onRefreshServices: () => void;
  prefillTarget?: string;
  onPrefillConsumed?: () => void;
  onHistoryOpen?: (filePath: string) => void;
  entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
  onPublish?: (entityId: string) => Promise<void> | void;
  onRevert?: (entityId: string) => void;
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
 * Both validators are pure and return a message rather than throwing, so the create modal and the
 * inline row editor can share them — the modal paints its result, the autosave path only asks
 * whether it is `null`.
 */
const subdomainError = (sub: string, mappings: LocalMapping[], ignoreId: string | null): string | null => {
  const s = sub.trim().toLowerCase();
  if (!s) return strings.mappings.subdomainRequired;
  const labels = s.split(".");
  const invalid = labels.some(
    (l) => !l || !/^[a-zA-Z0-9-]+$/.test(l) || l.startsWith("-") || l.endsWith("-")
  );
  if (invalid) return strings.mappings.subdomainInvalid;
  const domain = `${s}.localhost`;
  const duplicate = mappings.find((m) => m.domain.toLowerCase() === domain && m.id !== ignoreId);
  if (duplicate) return strings.mappings.subdomainDuplicate.replace("{domain}", domain);
  return null;
};

const targetError = (target: string): string | null =>
  !/^[a-zA-Z0-9.\-]+:\d{1,5}$/.test(target) || parseInt(target.split(":").pop()!, 10) > 65535
    ? strings.mappings.targetInvalid
    : null;

export default function MappingsPanel({
  config,
  onConfigChange,
  onRefreshServices,
  prefillTarget,
  onPrefillConsumed,
  onHistoryOpen,
  entitySyncStatus,
  onPublish,
  onRevert,
}: Props) {
  const { confirm, ConfirmDialogElement } = useConfirmDialog();

  // The modal is create-only; existing mappings are edited on their cards.
  const [modalOpen, setModalOpen] = useState(false);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [errors, setErrors] = useState<Partial<FormState>>({});

  /**
   * In-progress inline edits, keyed by mapping id.
   *
   * A keystroke must **not** reach `config` — that path persists the workspace to disk — so edits
   * accumulate here and the debounce below applies them in one pass.
   */
  const [edits, setEdits] = useState<Record<string, MappingEdit>>({});
  const [rowErrors, setRowErrors] = useState<Record<string, Partial<FormState>>>({});
  const [publishingIds, setPublishingIds] = useState<Record<string, boolean>>({});
  const [publishErrors, setPublishErrors] = useState<Record<string, string>>({});

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
    const errs: Partial<FormState> = {};
    const subErr = subdomainError(form.subdomain, config.mappings, null);
    const tgtErr = targetError(form.target);
    if (subErr) errs.subdomain = subErr;
    if (tgtErr) errs.target = tgtErr;
    setErrors(errs);
    if (Object.keys(errs).length > 0) return;

    const newM = await window.api.addMapping({
      domain: fullDomain(form.subdomain),
      target: form.target,
      label: form.label,
      enabled: true,
    });
    await onConfigChange({ ...config, mappings: [...config.mappings, newM] });
    setModalOpen(false);
    onRefreshServices();
  };

  // -- Inline edit + autosave ----------------------------------------------

  const rowValue = (m: LocalMapping): MappingEdit =>
    edits[m.id] ?? { subdomain: subdomainOf(m), target: m.target, label: m.label ?? "" };

  const setRowField = (id: string, patch: Partial<MappingEdit>) => {
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
  const applyEdits = async () => {
    const ids = Object.keys(edits);
    if (ids.length === 0) return;

    const nextErrors: Record<string, Partial<FormState>> = {};
    const updates: LocalMapping[] = [];
    for (const id of ids) {
      const m = config.mappings.find((x) => x.id === id);
      if (!m) continue;
      const e = edits[id];
      const subErr = subdomainError(e.subdomain, config.mappings, id);
      const tgtErr = targetError(e.target);
      if (subErr || tgtErr) {
        nextErrors[id] = { ...(subErr ? { subdomain: subErr } : {}), ...(tgtErr ? { target: tgtErr } : {}) };
        continue;
      }
      updates.push({ ...m, domain: fullDomain(e.subdomain), target: e.target.trim(), label: e.label });
    }

    setRowErrors(nextErrors);
    if (updates.length === 0) return;

    for (const u of updates) await window.api.updateMapping(u);
    const written = new Map(updates.map((u) => [u.id, u]));
    await onConfigChange({ ...config, mappings: config.mappings.map((m) => written.get(m.id) ?? m) });
    setEdits((prev) => {
      const next = { ...prev };
      for (const u of updates) delete next[u.id];
      return next;
    });
    onRefreshServices();
  };

  const { savedAt: autoSavedAt, error: autoSaveError } = useDebouncedAutosave({
    snapshot: JSON.stringify(edits),
    enabled: true,
    onSave: applyEdits,
  });

  // -- Row actions ----------------------------------------------------------

  const remove = async (id: string) => {
    const ok = await confirm("Delete this mapping? This cannot be undone.");
    if (!ok) return;
    await window.api.deleteMapping(id);
    const mappings = config.mappings.filter((m) => m.id !== id);
    const proxyRules = config.proxyRules.filter((r) => r.targetMappingId !== id);
    await onConfigChange({ ...config, mappings, proxyRules });
    setEdits((prev) => {
      if (!(id in prev)) return prev;
      const next = { ...prev };
      delete next[id];
      return next;
    });
    onRefreshServices();
  };

  const toggleEnabled = async (m: LocalMapping, enabled: boolean) => {
    await window.api.setEntityEnabled(config.activeWorkspaceId, "mappings", m.id, enabled);
    await onConfigChange({ ...config, mappings: config.mappings.map((x) => (x.id === m.id ? { ...x, enabled } : x)) });
  };

  const publish = async (m: LocalMapping) => {
    if (!onPublish || publishingIds[m.id]) return;
    const pending = edits[m.id];
    if (pending) {
      const errors = {
        ...(subdomainError(pending.subdomain, config.mappings, m.id) ? { subdomain: subdomainError(pending.subdomain, config.mappings, m.id)! } : {}),
        ...(targetError(pending.target) ? { target: targetError(pending.target)! } : {}),
      };
      if (Object.keys(errors).length) {
        setRowErrors((prev) => ({ ...prev, [m.id]: errors }));
        return;
      }
    }
    setPublishingIds((prev) => ({ ...prev, [m.id]: true }));
    setPublishErrors((prev) => { const next = { ...prev }; delete next[m.id]; return next; });
    try {
      // Persist in-progress card edits before committing the entity file.
      await applyEdits();
      await onPublish(m.id);
    } catch (error) {
      setPublishErrors((prev) => ({ ...prev, [m.id]: error instanceof Error ? error.message : String(error) }));
    } finally {
      setPublishingIds((prev) => ({ ...prev, [m.id]: false }));
    }
  };

  const syncDotColor = (syncSt: string | undefined) => {
    if (syncSt === "new" || syncSt === "deleted") return "red" as const;
    if (syncSt === "modified") return "yellow" as const;
    return "green" as const;
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
                const syncSt = entitySyncStatus?.[flatEntityRelPath("mappings", m.id)];
                const hasSavedVersion = syncSt === "clean" || syncSt === "modified";
                const changed = syncSt === "new" || syncSt === "modified";
                const statusText = syncSt === "new" ? strings.mappings.syncNew : syncSt === "modified" ? strings.mappings.syncModified : syncSt === "clean" ? strings.mappings.syncClean : "";
                return (
                  <article key={m.id} className="group rounded-xl border border-border bg-card shadow-sm transition-colors hover:border-border/80">
                    <div className="flex items-start gap-3 border-b border-border/70 px-4 py-4">
                      <div className="mt-0.5 flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-signal/10 text-signal"><Globe size={17} /></div>
                      <div className="min-w-0 flex-1">
                        <Input className="w-full border-transparent bg-transparent px-1 py-0.5 text-sm font-semibold text-foreground shadow-none hover:border-border focus:border-border" value={value.label} placeholder={strings.mappings.titlePlaceholder} aria-label={strings.mappings.columnLabel} onChange={(e) => setRowField(m.id, { label: e.target.value })} />
                      </div>
                      <div className="flex items-center gap-2 pt-1">
                        {syncSt && <span title={statusText} className="flex items-center gap-1.5 text-[10px] text-muted-foreground"><StatusDot color={syncDotColor(syncSt)} />{statusText}</span>}
                        <Switch checked={m.enabled} onChange={(v) => toggleEnabled(m, v)} />
                      </div>
                    </div>
                    <div className="space-y-3 px-4 py-4">
                      <FormField label={strings.mappings.subdomainLabel} error={rowErrors[m.id]?.subdomain}>
                        <div className="flex items-center"><Input className="min-w-0 flex-1 rounded-r-none font-mono text-xs" value={value.subdomain} error={!!rowErrors[m.id]?.subdomain} aria-label={strings.mappings.columnDomain} onChange={(e) => setRowField(m.id, { subdomain: e.target.value })} /></div>
                        <div className="mt-1.5 flex items-center gap-1.5 text-xs text-muted-foreground"><span className="font-mono text-signal">{fullDomain(value.subdomain)}</span></div>
                      </FormField>
                      <FormField label={strings.mappings.targetCardLabel} error={rowErrors[m.id]?.target}>
                        <div className="relative"><Server size={14} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-muted-foreground" /><Input className="w-full pl-9 font-mono text-xs" value={value.target} error={!!rowErrors[m.id]?.target} aria-label={strings.mappings.columnTarget} onChange={(e) => setRowField(m.id, { target: e.target.value })} /></div>
                      </FormField>
                    </div>
                    <div className="flex items-center justify-between border-t border-border/70 px-3 py-2">
                      <div className="flex items-center gap-1">
                        {onPublish && changed && <Button variant="secondary" size="sm" disabled={publishingIds[m.id]} onClick={() => void publish(m)}>{publishingIds[m.id] ? strings.footer.publishing : strings.mappings.publish}</Button>}
                        {onRevert && syncSt === "modified" && <Button variant="ghost" size="sm" icon={<RotateCcw size={13} />} onClick={() => onRevert(m.id)}>{strings.mappings.revert}</Button>}
                        {onHistoryOpen && hasSavedVersion && <IconButton icon={<History size={13} />} title={strings.mappings.viewHistory} onClick={() => onHistoryOpen(`mappings/${m.id}.json`)} className="hover:text-signal" />}
                      </div>
                      <Button variant="danger" size="sm" onClick={() => remove(m.id)}>{strings.common.delete}</Button>
                    </div>
                    {publishErrors[m.id] && <p role="alert" className="border-t border-destructive/20 px-4 py-2 text-xs text-destructive">{publishErrors[m.id]}</p>}
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
      </div>
    </>
  );
}
