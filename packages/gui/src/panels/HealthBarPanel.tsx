import React, { useState, useEffect, useCallback, useRef } from "react";
import { AppConfig, Environment, HealthBarService } from "@/types";
import PanelHeader from "@/components/layout/PanelHeader";
import { resolveVars } from "@/lib/resolveVars";
import { Button, EmptyState } from "@/components/ui";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import { useDebouncedAutosave } from "@/hooks/useDebouncedAutosave";
import { Plus, RefreshCw, Activity, Cloud, AlertCircle, CheckCircle2, History, RotateCcw } from "@/lib/icons";
import { strings } from "@/lib/strings";
import { entityGitView, entityGitDotColor } from "@/lib/entityGit";
import { HealthCheckResult, ServiceState } from "./healthbar/types";
import ServiceCard from "./healthbar/ServiceCard";
import ResponseModal from "./healthbar/ResponseModal";
import AddServiceModal from "./healthbar/AddServiceModal";

interface Props {
  config: AppConfig;
  entitySyncStatus: Record<string, "clean" | "modified" | "new" | "deleted">;
  /** The workspace's position relative to its remote — needed for "committed but not pushed". */
  workspaceGitState?: import("@/types").WorkspaceGitState | null;
  onPublish: () => Promise<unknown>;
  /** Revert this singleton file to its last synced version. */
  onRevert?: () => Promise<unknown>;
  /** Open the History sidebar for this singleton file. */
  onHistoryOpen?: (filePath: string) => void;
  onAfterSave?: () => void;
}

let _hbid = 0;
const mkHbId = () => `hb${Date.now().toString(36)}${(++_hbid).toString(36)}`;
const defaultState: ServiceState = { status: "idle", statusCode: null, body: null, headers: null, error: null, durationMs: null, checkedAt: null };

export default function HealthBarPanel({ config, entitySyncStatus, workspaceGitState, onPublish, onRevert, onHistoryOpen, onAfterSave }: Props) {
  const wsId = config.activeWorkspaceId;
  const { confirm, ConfirmDialogElement } = useConfirmDialog();
  const activeEnv: Environment | null = (config.environments ?? []).find((e) => e.id === config.activeEnvironmentId) ?? null;

  const [services, setServices] = useState<HealthBarService[]>([]);
  const [checkStates, setCheckStates] = useState<Record<string, ServiceState>>({});
  const [loading, setLoading] = useState(true);
  const [addModalOpen, setAddModalOpen] = useState(false);
  const [editingServiceId, setEditingServiceId] = useState<string | null>(null);
  const [responseModal, setResponseModal] = useState<{ service: HealthBarService; state: ServiceState } | null>(null);
  const [publishing, setPublishing] = useState(false);
  const inflightRef = useRef<Set<string>>(new Set());
  /** JSON of what is already on disk, so an autosave that changes nothing writes nothing. */
  const persistedRef = useRef<string>("");

  /**
   * The singleton file's git view.
   *
   * `healthbar/services.json` is **one** entity, so the whole panel is that entity — which is why
   * Publish / Sync / Revert / History are panel-level controls here rather than per-row ones. Revert
   * and History did not exist on this panel at all: the health bar could be published but never
   * rolled back, and its commits were invisible.
   */
  const view = entityGitView(entitySyncStatus["healthbar/services.json"], workspaceGitState);
  const publishDisabled = !view.primaryEnabled || publishing;

  const loadServices = useCallback(async () => {
    setLoading(true);
    try {
      const svcs = await window.api.healthbarGetServices(wsId);
      persistedRef.current = JSON.stringify(svcs);
      setServices(svcs);
      return svcs;
    } finally { setLoading(false); }
  }, [wsId]);

  const persistServices = useCallback(async (svcs: HealthBarService[]) => {
    const json = JSON.stringify(svcs);
    // Delete, the auto-refresh toggle and a create all persist immediately. Without this guard the
    // debounce that their own state change schedules would rewrite the identical file a second
    // later — a no-op on disk, but a pointless write and a spurious "Saved" flicker.
    if (json === persistedRef.current) return;
    await window.api.healthbarSaveServices(wsId, svcs);
    persistedRef.current = json;
    onAfterSave?.();
  }, [wsId, onAfterSave]);

  const checkService = useCallback(async (svc: HealthBarService) => {
    if (inflightRef.current.has(svc.id)) return;
    inflightRef.current.add(svc.id);
    const resolvedUrl = resolveVars(svc.url, activeEnv);
    setCheckStates(prev => ({
      ...prev,
      [svc.id]: { ...(prev[svc.id] ?? defaultState), status: "checking" }
    }));
    try {
      const res: HealthCheckResult = await window.api.healthbarCheckUrl(resolvedUrl);
      const isOk = res.ok && res.statusCode !== null && res.statusCode >= 200 && res.statusCode < 300;
      setCheckStates(prev => ({
        ...prev,
        [svc.id]: {
          status: isOk ? "success" : "error", statusCode: res.statusCode,
          body: res.body, headers: res.headers, error: res.error,
          durationMs: res.durationMs, checkedAt: Date.now()
        }
      }));
    } catch (err: any) {
      setCheckStates(prev => ({
        ...prev,
        [svc.id]: {
          ...defaultState, status: "error", error: err?.message ?? "Check failed", checkedAt: Date.now()
        }
      }));
    } finally { inflightRef.current.delete(svc.id); }
  }, [activeEnv]);

  const checkAll = useCallback((svcs: HealthBarService[]) => {
    svcs.forEach(checkService);
  }, [checkService]);

  useEffect(() => {
    loadServices().then(svcs => {
      const auto = svcs.filter(s => s.autoRefreshEnabled);
      if (auto.length) checkAll(auto);
    });
  }, [wsId]);

  const handleOpenAdd = () => { setEditingServiceId(null); setAddModalOpen(true); };
  const handleOpenEdit = useCallback((id: string) => { setEditingServiceId(id); setAddModalOpen(true); }, []);

  /** Name / URL typed straight on a card. The debounce below is what writes it. */
  const handleEditField = useCallback((id: string, patch: Partial<HealthBarService>) => {
    setServices(prev => prev.map(s => (s.id === id ? { ...s, ...patch } : s)));
  }, []);

  /**
   * Autosave. Gated on `!loading` so the services arriving from disk arm the hook silently instead
   * of looking like an edit and being written straight back.
   */
  const { savedAt: autoSavedAt, error: autoSaveError } = useDebouncedAutosave({
    snapshot: JSON.stringify(services),
    enabled: !loading,
    onSave: () => persistServices(services),
  });

  const handleCreateService = async (name: string, url: string, autoRefreshEnabled: boolean) => {
    const created: HealthBarService = { id: mkHbId(), name, url, autoRefreshEnabled, createdAt: Date.now() };
    const updated = [...services, created];
    setServices(updated);
    setAddModalOpen(false);
    setEditingServiceId(null);
    await persistServices(updated);
    checkService(created);
  };

  const handleDelete = useCallback(async (id: string) => {
    if (!await confirm("Delete this service? This cannot be undone.")) return;
    const updated = services.filter(s => s.id !== id);
    setServices(updated);
    setCheckStates(prev => { const next = { ...prev }; delete next[id]; return next; });
    await persistServices(updated);
  }, [confirm, services, persistServices]);

  const handleToggleAutoRefresh = useCallback(async (id: string, enabled: boolean) => {
    const updated = services.map(s => s.id === id ? { ...s, autoRefreshEnabled: enabled } : s);
    setServices(updated); await persistServices(updated);
  }, [services, persistServices]);

  const handlePublish = async () => {
    setPublishing(true);
    try { await onPublish(); } finally { setPublishing(false); }
  };

  const handleCardClick = useCallback((id: string) => {
    const svc = services.find(s => s.id === id);
    const state = checkStates[id];
    if (svc && state && state.status !== "idle") setResponseModal({ service: svc, state });
  }, [services, checkStates]);

  const handleRefresh = useCallback((id: string) => {
    const svc = services.find(s => s.id === id);
    if (svc) checkService(svc);
  }, [services, checkService]);

  const anyChecking = Object.values(checkStates).some(s => s.status === "checking");

  const headerActions = (
    <>
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
      {view.primary && (
        <span title={view.primaryTitle}>
          <Button variant="secondary" icon={<Cloud size={13} />} disabled={publishDisabled} onClick={handlePublish}>
            {publishing ? strings.entityGit.publishing : view.primaryLabel}
          </Button>
        </span>
      )}
      {onRevert && view.canRevert && (
        <Button variant="ghost" icon={<RotateCcw size={13} />} title={strings.entityGit.revertTooltip} onClick={() => void onRevert()}>
          {strings.entityGit.revert}
        </Button>
      )}
      {onHistoryOpen && view.canViewHistory && (
        <Button variant="ghost" icon={<History size={13} />} title={strings.entityGit.historyTooltip} onClick={() => onHistoryOpen("healthbar/services.json")}>
          {strings.entityGit.history}
        </Button>
      )}
      <Button variant="secondary" icon={<RefreshCw size={13} className={anyChecking ? "animate-spin" : ""} />}
        disabled={anyChecking || services.length === 0} onClick={() => checkAll(services)}>
        {strings.healthBar.refreshAll}
      </Button>
      <Button variant="primary" icon={<Plus size={13} />} onClick={handleOpenAdd}>
        {strings.healthBar.addService}
      </Button>
    </>
  );

  return (
    <>
      {ConfirmDialogElement}
      <div className="flex flex-col flex-1 overflow-hidden">
        <PanelHeader
          title={strings.healthBar.title}
          subtitle={services.length ? `${services.length} ${services.length !== 1 ? strings.healthBar.services : strings.healthBar.service}` : undefined}
          actions={headerActions}
        />
        <div className="flex-1 overflow-y-auto p-6">
          {loading && <div className="flex items-center justify-center h-32 text-muted-foreground text-sm">{strings.healthBar.loadingServices}</div>}
          {!loading && !services.length && (
            <EmptyState fill icon={<Activity size={40} />} title={strings.healthBar.noServices} description={strings.healthBar.noServicesDesc}
              action={<Button variant="primary" icon={<Plus size={13} />} onClick={handleOpenAdd}>{strings.healthBar.addService}</Button>} />
          )}
          {!loading && services.length > 0 && (
            <div className="grid grid-cols-1 gap-3 md:grid-cols-2 xl:grid-cols-3">
              {services.map(svc => (
                <ServiceCard key={svc.id} service={svc} state={checkStates[svc.id] ?? defaultState} resolvedUrl={resolveVars(svc.url, activeEnv)}
                  onRefresh={handleRefresh} onToggleAutoRefresh={handleToggleAutoRefresh} onDelete={handleDelete} onClick={handleCardClick}
                  onEdit={handleOpenEdit} onEditField={handleEditField} />
              ))}
            </div>
          )}
        </div>
        <AddServiceModal
          open={addModalOpen}
          editingService={editingServiceId ? services.find(s => s.id === editingServiceId) ?? null : null}
          onClose={() => { setAddModalOpen(false); setEditingServiceId(null); }}
          onCreate={handleCreateService}
          onEdit={handleEditField}
        />
        <ResponseModal open={!!responseModal} service={responseModal?.service ?? null} state={responseModal?.state ?? null} onClose={() => setResponseModal(null)} />
      </div>
    </>
  );
}
