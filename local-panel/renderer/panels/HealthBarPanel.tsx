import React, { useState, useEffect, useCallback, useRef } from "react";
import { AppConfig, Environment, HealthBarService } from "@/types";
import PanelHeader from "@/components/layout/PanelHeader";
import { resolveVars } from "@/lib/resolveVars";
import { Button, EmptyState } from "@/components/ui";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import { Plus, RefreshCw, Activity, Cloud } from "@/lib/icons";
import { strings } from "@/lib/strings";
import { HealthCheckResult, ServiceState } from "./healthbar/types";
import ServiceCard from "./healthbar/ServiceCard";
import ResponseModal from "./healthbar/ResponseModal";
import AddServiceModal from "./healthbar/AddServiceModal";

interface Props {
  config: AppConfig;
  entitySyncStatus: Record<string, "clean" | "modified" | "new" | "deleted">;
  onPublish: () => Promise<void>;
  onAfterSave?: () => void;
}

let _hbid = 0;
const mkHbId = () => `hb${Date.now().toString(36)}${(++_hbid).toString(36)}`;
const defaultState: ServiceState = { status: "idle", statusCode: null, body: null, headers: null, error: null, durationMs: null, checkedAt: null };

export default function HealthBarPanel({ config, entitySyncStatus, onPublish, onAfterSave }: Props) {
  const wsId = config.activeWorkspaceId;
  const { confirm, ConfirmDialogElement } = useConfirmDialog();
  const activeEnv: Environment | null = (config.environments ?? []).find((e) => e.id === config.activeEnvironmentId) ?? null;

  const [services, setServices] = useState<HealthBarService[]>([]);
  const [checkStates, setCheckStates] = useState<Record<string, ServiceState>>({});
  const [loading, setLoading] = useState(true);
  const [addModalOpen, setAddModalOpen] = useState(false);
  const [editingService, setEditingService] = useState<HealthBarService | null>(null);
  const [responseModal, setResponseModal] = useState<{ service: HealthBarService; state: ServiceState } | null>(null);
  const [publishing, setPublishing] = useState(false);
  const inflightRef = useRef<Set<string>>(new Set());

  const syncStatus = entitySyncStatus["healthbar/services.json"];
  const publishDisabled = syncStatus === "clean" || publishing;

  const loadServices = useCallback(async () => {
    setLoading(true);
    try {
      const svcs = await window.api.healthbarGetServices(wsId);
      setServices(svcs);
      return svcs;
    } finally { setLoading(false); }
  }, [wsId]);

  const persistServices = useCallback(async (svcs: HealthBarService[]) => {
    await window.api.healthbarSaveServices(wsId, svcs);
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

  const handleOpenAdd = () => { setEditingService(null); setAddModalOpen(true); };

  const handleSaveService = async (name: string, url: string) => {
    let updated: HealthBarService[];
    if (editingService) {
      updated = services.map(s => s.id === editingService.id ? { ...s, name, url } : s);
    } else {
      updated = [...services, { id: mkHbId(), name, url, autoRefreshEnabled: true, createdAt: Date.now() }];
    }
    setServices(updated); setAddModalOpen(false); setEditingService(null);
    await persistServices(updated);
    checkService(editingService ? updated.find(s => s.id === editingService.id)! : updated[updated.length - 1]);
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
      <span title={syncStatus === "clean" ? strings.healthBar.synced : undefined}>
        <Button variant="secondary" icon={<Cloud size={13} />} disabled={publishDisabled} onClick={handlePublish}>
          {publishing ? strings.healthBar.publishing : strings.healthBar.publish}
        </Button>
      </span>
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
                  onRefresh={handleRefresh} onToggleAutoRefresh={handleToggleAutoRefresh} onDelete={handleDelete} onClick={handleCardClick} />
              ))}
            </div>
          )}
        </div>
        <AddServiceModal open={addModalOpen} editingService={editingService} onClose={() => { setAddModalOpen(false); setEditingService(null); }} onSave={handleSaveService} />
        <ResponseModal open={!!responseModal} service={responseModal?.service ?? null} state={responseModal?.state ?? null} onClose={() => setResponseModal(null)} />
      </div>
    </>
  );
}
