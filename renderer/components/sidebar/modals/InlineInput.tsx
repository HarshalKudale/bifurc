import React, { useState, useEffect, useRef } from "react";
import { strings } from "@/lib/strings";

export function InlineInput({
  value,
  onCommit,
  onCancel,
}: {
  value: string;
  onCommit(v: string): void;
  onCancel(): void;
}) {
  const [val, setVal] = useState(value);
  const ref = useRef<HTMLInputElement>(null);
  
  useEffect(() => {
    ref.current?.focus();
    ref.current?.select();
  }, []);

  return (
    <input
      ref={ref}
      className="flex-1 bg-surface-2 border border-signal rounded px-1.5 py-0.5 text-xs text-foreground outline-none min-w-0"
      value={val}
      onChange={(e) => setVal(e.target.value)}
      onKeyDown={(e) => {
        if (e.key === "Enter") {
          e.preventDefault();
          if (val.trim()) onCommit(val.trim());
        }
        if (e.key === "Escape") {
          e.preventDefault();
          onCancel();
        }
      }}
      onBlur={() => {
        if (val.trim()) onCommit(val.trim());
        else onCancel();
      }}
      onClick={(e) => e.stopPropagation()}
    />
  );
}
