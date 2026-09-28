import React from "react";
import Button from "@/components/ui/Button";
import { strings } from "@/lib/strings";

interface ModalFooterProps {
  onCancel: () => void;
  onConfirm: () => void;
  /**
   * **Required on purpose.** This used to default to `strings.common.save`, so a modal that forgot
   * to pass one silently rendered a "Save" button — and Save is no longer a concept in this app: an
   * entity that already exists autosaves, and the only explicit write is a **Create**. Making it
   * required means a new modal cannot reintroduce one by omission. Both call sites pass
   * `strings.common.create` / `strings.common.done`.
   */
  confirmLabel: string;
  confirmVariant?: "primary" | "danger";
  confirmDisabled?: boolean;
  cancelLabel?: string;
}

export default function ModalFooter({
  onCancel,
  onConfirm,
  confirmLabel,
  confirmVariant = "primary",
  confirmDisabled,
  cancelLabel = strings.common.cancel,
}: ModalFooterProps) {
  return (
    <div className="flex justify-end gap-2 mt-6 pt-4 border-t border-border">
      <Button variant="secondary" onClick={onCancel}>{cancelLabel}</Button>
      <Button variant={confirmVariant} onClick={onConfirm} disabled={confirmDisabled}>{confirmLabel}</Button>
    </div>
  );
}
