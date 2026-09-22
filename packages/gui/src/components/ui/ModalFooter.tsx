import React from "react";
import Button from "@/components/ui/Button";
import { strings } from "@/lib/strings";

interface ModalFooterProps {
  onCancel: () => void;
  onConfirm: () => void;
  confirmLabel?: string;
  confirmVariant?: "primary" | "danger";
  confirmDisabled?: boolean;
  cancelLabel?: string;
}

export default function ModalFooter({
  onCancel,
  onConfirm,
  confirmLabel = strings.common.save,
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
