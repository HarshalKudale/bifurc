import React, { useState, useEffect } from "react";
import { HealthBarService } from "@/types";
import { strings } from "@/lib/strings";
import { Input, FormField, ModalFooter } from "@/components/ui";
import Modal from "@/components/common/Modal";

interface FormState {
  name: string;
  url: string;
}

const EMPTY_FORM: FormState = { name: "", url: "" };

interface AddServiceModalProps {
  open: boolean;
  editingService: HealthBarService | null;
  onClose: () => void;
  onSave: (name: string, url: string) => void;
}

export default function AddServiceModal({
  open,
  editingService,
  onClose,
  onSave,
}: AddServiceModalProps) {
  const [form, setForm] = useState<FormState>(EMPTY_FORM);
  const [errors, setErrors] = useState<Partial<FormState>>({});

  useEffect(() => {
    if (open) {
      setForm(editingService ? { name: editingService.name, url: editingService.url } : EMPTY_FORM);
      setErrors({});
    }
  }, [open, editingService]);

  const validate = (): boolean => {
    const errs: Partial<FormState> = {};
    if (!form.name.trim()) errs.name = strings.healthBar.nameRequired;
    if (!form.url.trim()) {
      errs.url = strings.healthBar.urlRequired;
    } else {
      const stripped = form.url.replace(/\{\{[^}]+\}\}/g, "placeholder");
      try { new URL(stripped); } catch {
        errs.url = strings.healthBar.urlInvalid;
      }
    }
    setErrors(errs);
    return Object.keys(errs).length === 0;
  };

  const handleSave = () => {
    if (!validate()) return;
    onSave(form.name.trim(), form.url.trim());
  };

  return (
    <Modal open={open} title={editingService ? strings.healthBar.editService : strings.healthBar.addService} onClose={onClose}>
      <FormField label={strings.healthBar.serviceName} error={errors.name}>
        <Input
          className="w-full"
          placeholder="e.g. Auth Service"
          value={form.name}
          error={!!errors.name}
          autoFocus
          onChange={(e) => setForm((f) => ({ ...f, name: e.target.value }))}
          onKeyDown={(e) => { if (e.key === "Enter") handleSave(); }}
        />
      </FormField>
      <FormField label={strings.healthBar.healthCheckUrl} error={errors.url}>
        <Input
          className="w-full font-mono"
          placeholder="http://localhost:3000/health or http://{{HOST}}/health"
          value={form.url}
          error={!!errors.url}
          onChange={(e) => setForm((f) => ({ ...f, url: e.target.value }))}
          onKeyDown={(e) => { if (e.key === "Enter") handleSave(); }}
        />
        <p className="text-xs text-muted-foreground mt-1">
          {strings.healthBar.supportsEnvVars} <code className="text-signal">{"{{VAR_NAME}}"}</code>
        </p>
      </FormField>
      <ModalFooter
        onCancel={onClose}
        onConfirm={handleSave}
        confirmLabel={editingService ? strings.healthBar.update : strings.healthBar.addService}
      />
    </Modal>
  );
}
