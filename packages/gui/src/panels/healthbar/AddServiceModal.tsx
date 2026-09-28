import React, { useState, useEffect, useRef } from "react";
import { HealthBarService } from "@/types";
import { strings } from "@/lib/strings";
import { Input, FormField, ModalFooter, Switch } from "@/components/ui";
import Modal from "@/components/common/Modal";

interface FormState {
  name: string;
  url: string;
  autoRefreshEnabled: boolean;
}

const EMPTY_FORM: FormState = { name: "", url: "", autoRefreshEnabled: true };

interface AddServiceModalProps {
  open: boolean;
  editingService: HealthBarService | null;
  onClose: () => void;
  /** Create path — driven by the Create button. */
  onCreate: (name: string, url: string, autoRefreshEnabled: boolean) => void;
  /** Edit path — fired as the fields change; the panel debounces the write. */
  onEdit: (id: string, patch: Partial<HealthBarService>) => void;
}

export default function AddServiceModal({
  open,
  editingService,
  onClose,
  onCreate,
  onEdit,
}: AddServiceModalProps) {
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [errors, setErrors] = useState<Partial<Record<keyof FormState, string>>>({});

  /**
   * Seed the form once per **opening**, not on every change of `editingService`.
   *
   * The panel re-derives that object from `services` on each render, so an autosave landing
   * changes its identity — re-seeding then would clobber whatever the user typed during the
   * round-trip.
   */
  const seededForRef = useRef<string | null>(null);
  useEffect(() => {
    if (!open) { seededForRef.current = null; return; }
    const key = editingService?.id ?? "__new__";
    if (seededForRef.current === key) return;
    seededForRef.current = key;
    setForm(editingService
      ? { name: editingService.name, url: editingService.url, autoRefreshEnabled: editingService.autoRefreshEnabled }
      : EMPTY_FORM);
    setErrors({});
  }, [open, editingService]);

  const collectErrors = (f: FormState) => {
    const errs: Partial<Record<keyof FormState, string>> = {};
    if (!f.name.trim()) errs.name = strings.healthBar.nameRequired;
    if (!f.url.trim()) {
      errs.url = strings.healthBar.urlRequired;
    } else {
      const stripped = f.url.replace(/\{\{[^}]+\}\}/g, "placeholder");
      try { new URL(stripped); } catch { errs.url = strings.healthBar.urlInvalid; }
    }
    return errs;
  };

  /**
   * Edit mode writes as you type. A form that would not validate is simply **not propagated** — a
   * half-typed URL must not reach the workspace file — and no error is painted, because shouting
   * at a field the user is still filling in is worse than staying quiet for a moment.
   */
  const patch = (next: FormState) => {
    setForm(next);
    if (!editingService) return;
    if (Object.keys(collectErrors(next)).length > 0) return;
    onEdit(editingService.id, {
      name: next.name.trim(),
      url: next.url.trim(),
      autoRefreshEnabled: next.autoRefreshEnabled,
    });
  };

  /** Create mode: the explicit write, and the only place errors are shown. */
  const handleCreate = () => {
    const errs = collectErrors(form);
    setErrors(errs);
    if (Object.keys(errs).length > 0) return;
    onCreate(form.name.trim(), form.url.trim(), form.autoRefreshEnabled);
  };

  const isEditing = !!editingService;

  return (
    <Modal open={open} title={isEditing ? strings.healthBar.editService : strings.healthBar.addService} onClose={onClose}>
      <FormField label={strings.healthBar.serviceName} error={errors.name}>
        <Input
          className="w-full"
          placeholder="e.g. Auth Service"
          value={form.name}
          error={!!errors.name}
          autoFocus
          onChange={(e) => patch({ ...form, name: e.target.value })}
          onKeyDown={(e) => { if (e.key === "Enter" && !isEditing) handleCreate(); }}
        />
      </FormField>
      <FormField label={strings.healthBar.healthCheckUrl} error={errors.url}>
        <Input
          className="w-full font-mono"
          placeholder="http://localhost:3000/health or http://{{HOST}}/health"
          value={form.url}
          error={!!errors.url}
          onChange={(e) => patch({ ...form, url: e.target.value })}
          onKeyDown={(e) => { if (e.key === "Enter" && !isEditing) handleCreate(); }}
        />
        <p className="text-xs text-muted-foreground mt-1">
          {strings.healthBar.supportsEnvVars} <code className="text-signal">{"{{VAR_NAME}}"}</code>
        </p>
      </FormField>

      <div className="flex items-center gap-2 mt-3">
        <Switch checked={form.autoRefreshEnabled} onChange={(v) => patch({ ...form, autoRefreshEnabled: v })} />
        <span className="text-xs text-muted-foreground">{strings.healthBar.autoRefresh}</span>
      </div>

      {/* Create writes; Done only closes — an existing service has been saved by the time it is
          clicked, because every field above writes itself. */}
      <ModalFooter
        onCancel={onClose}
        onConfirm={isEditing ? onClose : handleCreate}
        confirmLabel={isEditing ? strings.common.done : strings.common.create}
      />
    </Modal>
  );
}
