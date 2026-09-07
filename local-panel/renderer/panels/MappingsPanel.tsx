import React, { useState, useEffect } from "react";
import { AppConfig, LocalMapping } from "@/types";
import Modal from "@/components/common/Modal";
import PanelHeader from "@/components/layout/PanelHeader";
import { strings } from "@/lib/strings";
import { flatEntityRelPath } from "@/lib/utils";
import { ArrowLeftRight, History, Pencil } from "@/lib/icons";
import { Button, IconButton, Input, FormField, EmptyState, DataTable, ModalFooter, StatusDot, Switch } from "@/components/ui";
import { useConfirmDialog } from "@/hooks/useConfirmDialog";
import type { TableColumn } from "@/components/ui";


interface Props {
  config: AppConfig;
  onConfigChange: (cfg: AppConfig) => Promise<void>;
  onRefreshServices: () => void;
  prefillTarget?: string;
  onPrefillConsumed?: () => void;
  onHistoryOpen?: (filePath: string) => void;
  entitySyncStatus?: Record<string, "clean" | "modified" | "new" | "deleted">;
  onPublish?: (entityId: string) => void;
  onRevert?: (entityId: string) => void;
}

interface FormState {
  subdomain: string;
  target: string;
  label: string;
}

const EMPTY_FORM: FormState = { subdomain: "", target: "", label: "" };

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

  const [modalOpen, setModalOpen] = useState(false);
  const [editingId, setEditingId] = useState<string | null>(null);
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [errors, setErrors] = useState<Partial<FormState>>({});


  useEffect(() => {
    if (prefillTarget) {
      setEditingId(null);
      setForm({ subdomain: "", target: prefillTarget, label: "" });
      setErrors({});
      setModalOpen(true);
      onPrefillConsumed?.();
    }
  }, [prefillTarget]);

  const openAdd = () => {
    setEditingId(null);
    setForm(EMPTY_FORM);
    setErrors({});
    setModalOpen(true);
  };

  const openEdit = (m: LocalMapping) => {
    setEditingId(m.id);
    const subdomain = m.domain.replace(/\.localhost$/, "");
    setForm({ subdomain, target: m.target, label: m.label || "" });
    setErrors({});
    setModalOpen(true);
  };

  const fullDomain = (sub: string) =>
    sub.trim() ? `${sub.trim().toLowerCase()}.localhost` : "";

  const validate = (): boolean => {
    const errs: Partial<FormState> = {};
    const sub = form.subdomain.trim().toLowerCase();

    if (!sub) {
      errs.subdomain = strings.mappings.subdomainRequired;
    } else {
      const labels = sub.split(".");
      const invalid = labels.some(
        (l) => !l || !/^[a-zA-Z0-9-]+$/.test(l) || l.startsWith("-") || l.endsWith("-")
      );
      if (invalid) {
        errs.subdomain = strings.mappings.subdomainInvalid;
      } else {
        const domain = `${sub}.localhost`;
        const duplicate = config.mappings.find(
          (m) => m.domain.toLowerCase() === domain && m.id !== editingId
        );
        if (duplicate) errs.subdomain = strings.mappings.subdomainDuplicate.replace("{domain}", domain);
      }
    }

    if (!/^[a-zA-Z0-9.\-]+:\d{1,5}$/.test(form.target) || parseInt(form.target.split(":").pop()!, 10) > 65535) {
      errs.target = strings.mappings.targetInvalid;
    }
    setErrors(errs);
    return Object.keys(errs).length === 0;
  };

  const save = async () => {
    if (!validate()) return;
    const domain = fullDomain(form.subdomain);
    if (editingId) {
      const updated = config.mappings.map((m) =>
        m.id === editingId ? { ...m, domain, target: form.target, label: form.label } : m
      );
      await window.api.updateMapping(updated.find((m) => m.id === editingId)!);
      await onConfigChange({ ...config, mappings: updated });
    } else {
      const newM = await window.api.addMapping({ domain, target: form.target, label: form.label, enabled: true });
      await onConfigChange({ ...config, mappings: [...config.mappings, newM] });
    }
    setModalOpen(false);
    onRefreshServices();
  };

  const remove = async (id: string) => {
    const ok = await confirm("Delete this mapping? This cannot be undone.");
    if (!ok) return;
    await window.api.deleteMapping(id);
    const mappings = config.mappings.filter((m) => m.id !== id);
    const proxyRules = config.proxyRules.filter((r) => r.targetMappingId !== id);
    await onConfigChange({ ...config, mappings, proxyRules });
    onRefreshServices();
  };

  const toggleEnabled = async (m: LocalMapping, enabled: boolean) => {
    if (enabled) {
      const enabledCount = config.mappings.filter((x) => x.enabled).length;
    }
    await window.api.setEntityEnabled(config.activeWorkspaceId, "mappings", m.id, enabled);
    await onConfigChange({ ...config, mappings: config.mappings.map((x) => (x.id === m.id ? { ...x, enabled } : x)) });
  };




  const syncDotColor = (syncSt: string | undefined) => {
    if (syncSt === "new" || syncSt === "deleted") return "red" as const;
    if (syncSt === "modified") return "yellow" as const;
    return "green" as const;
  };

  const columns: TableColumn<LocalMapping>[] = [
    {
      key: "edit",
      width: "w-10",
      render: (m) => {
        const syncSt = entitySyncStatus?.[flatEntityRelPath("mappings", m.id)];
        return (
          <div className="flex items-center gap-2">
            {syncSt && <StatusDot color={syncDotColor(syncSt)} />}
            <IconButton icon={<Pencil size={14} />} title={strings.common.edit} onClick={() => openEdit(m)} />
          </div>
        );
      },
    },
    {
      key: "domain",
      header: strings.mappings.columnDomain,
      render: (m) => <span className="font-mono text-xs text-foreground">{m.domain}</span>,
    },
    {
      key: "target",
      header: strings.mappings.columnTarget,
      render: (m) => <span className="font-mono text-xs text-muted-foreground">{m.target}</span>,
    },
    {
      key: "label",
      header: strings.mappings.columnLabel,
      render: (m) => <span className="text-xs text-muted-foreground">{m.label || "—"}</span>,
    },
    {
      key: "on",
      header: strings.mappings.columnOn,
      align: "center",
      width: "w-14",
      render: (m) => (
        <div className="flex items-center justify-center">
          <Switch checked={m.enabled} onChange={(v) => toggleEnabled(m, v)} />
        </div>
      ),
    },
    {
      key: "actions",
      width: "w-24",
      render: (m) => {
        const syncSt = entitySyncStatus?.[flatEntityRelPath("mappings", m.id)];
        return (
          <div className="flex items-center justify-end gap-1">
            {onPublish && syncSt && syncSt !== "clean" && (
              <Button variant="ghost" size="sm" onClick={() => onPublish(m.id)}>{strings.mappings.publish}</Button>
            )}
            {onRevert && syncSt && syncSt !== "clean" && (
              <button
                onClick={() => onRevert(m.id)}
                className="px-2.5 py-1 rounded text-amber hover:bg-amber/10 text-xs font-medium transition-all cursor-pointer"
              >
                {strings.mappings.revert}
              </button>
            )}
            {onHistoryOpen && (
              <IconButton
                icon={<History size={11} />}
                title={strings.mappings.viewHistory}
                onClick={() => onHistoryOpen(`mappings/${m.id}.json`)}
                className="hover:text-signal"
              />
            )}
            <Button variant="danger" size="sm" onClick={() => remove(m.id)}>{strings.common.delete}</Button>
          </div>
        );
      },
    },
  ];

  const emptyNode = config.mappings.length === 0
    ? <EmptyState icon={<ArrowLeftRight size={36} />} title={strings.mappings.noMappingsYet} description={strings.mappings.noMappingsYetHint} />
    : <EmptyState icon={<ArrowLeftRight size={36} />} title={strings.mappings.noMatchingMappings} description={strings.mappings.noMatchingMappingsHint} />;

  return (
    <>
      {ConfirmDialogElement}
      <div className="flex flex-col flex-1 overflow-hidden">
        <PanelHeader
          title={strings.mappings.title}
          subtitle={strings.mappings.subtitle}
          actions={
            <Button variant="primary" onClick={openAdd}>{strings.mappings.addMapping}</Button>
          }
        />

        <div className="flex-1 overflow-y-auto p-6">
          <DataTable
            columns={columns}
            data={config.mappings}
            rowKey={(m) => m.id}
            emptyState={emptyNode}
          />
        </div>

        <Modal
          open={modalOpen}
          title={editingId ? strings.mappings.editMapping : strings.mappings.addMappingTitle}
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

          <ModalFooter onCancel={() => setModalOpen(false)} onConfirm={save} />
        </Modal>
      </div>
    </>
  );
}
