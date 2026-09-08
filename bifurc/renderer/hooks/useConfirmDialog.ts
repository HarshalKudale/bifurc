import React, { useState, useCallback } from "react";
import ConfirmDialog from "@/components/ui/ConfirmDialog";

interface ConfirmOptions {
  title?: string;
  confirmLabel?: string;
  confirmVariant?: "danger" | "primary";
}

interface PendingConfirm {
  message: string;
  options: ConfirmOptions;
  resolve: (value: boolean) => void;
}

export function useConfirmDialog() {
  const [pending, setPending] = useState<PendingConfirm | null>(null);

  const confirm = useCallback((message: string, options: ConfirmOptions = {}): Promise<boolean> => {
    return new Promise((resolve) => {
      setPending({ message, options, resolve });
    });
  }, []);

  const handleConfirm = useCallback(() => {
    pending?.resolve(true);
    setPending(null);
  }, [pending]);

  const handleCancel = useCallback(() => {
    pending?.resolve(false);
    setPending(null);
  }, [pending]);

  const ConfirmDialogElement = pending
    ? React.createElement(ConfirmDialog, {
        open: true,
        message: pending.message,
        title: pending.options.title,
        confirmLabel: pending.options.confirmLabel,
        confirmVariant: pending.options.confirmVariant,
        onConfirm: handleConfirm,
        onCancel: handleCancel,
      })
    : null;

  return { confirm, ConfirmDialogElement };
}
