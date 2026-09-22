import React, { useState } from "react";

export interface TokenHintItem {
  /** Token key, inserted as `{{key}}`. */
  key: string;
  /** Secondary text shown to the right of the token (value or description). */
  label: React.ReactNode;
}

interface TokenHintProps {
  /** Tooltip shown on the trigger button. */
  title: string;
  /** Short text shown between the `{{` `}}` braces on the trigger button. */
  triggerLabel: string;
  /** Header text shown at the top of the popover. */
  headerLabel: string;
  /** Tailwind color class applied to the braces/token accents (e.g. "text-signal" or "text-violet"). */
  accentClass: string;
  items: TokenHintItem[];
  onInsert: (token: string) => void;
  minWidthClass?: string;
  maxHeightClass?: string;
  /** Extra classes for the label span (e.g. a max-width truncation constraint). */
  labelClassName?: string;
}

/**
 * Generic "insert a {{token}}" hint button: a small pill that opens a popover
 * listing tokens, inserting `{{key}}` into the caller's focused input on click.
 * Shared base for EnvVarHint (environment variables) and RandomizerHint (random.* tokens).
 */
export default function TokenHint({
  title, triggerLabel, headerLabel, accentClass, items, onInsert,
  minWidthClass = "min-w-[200px]", maxHeightClass = "max-h-64", labelClassName = "max-w-[100px]",
}: TokenHintProps) {
  const [open, setOpen] = useState(false);

  return (
    <div className="relative flex-shrink-0" onMouseDown={(e) => e.preventDefault()}>
      <button
        type="button"
        title={title}
        onClick={() => setOpen((v) => !v)}
        className={`flex items-center gap-1 px-2 py-1 rounded border border-border bg-card hover:bg-surface-2 text-muted-foreground hover:${accentClass} text-[10px] font-mono transition-colors cursor-pointer`}
      >
        <span className={`${accentClass}/70`}>{"{{"}</span>
        <span className="text-muted-foreground">{triggerLabel}</span>
        <span className={`${accentClass}/70`}>{"}}"}</span>
      </button>

      {open && (
        <>
          <div className="fixed inset-0 z-40" onClick={() => setOpen(false)} />
          <div className={`absolute top-full mt-1 right-0 z-50 bg-card border border-border rounded-md shadow-2xl py-1 ${minWidthClass} ${maxHeightClass} overflow-y-auto`}>
            <div className="px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground border-b border-border/60">
              {headerLabel}
            </div>
            {items.map((item) => (
              <button
                key={item.key}
                onClick={() => { onInsert(`{{${item.key}}}`); setOpen(false); }}
                className="w-full flex items-center justify-between gap-3 px-3 py-1.5 text-xs cursor-pointer hover:bg-surface-2 text-left group"
              >
                <span className={`font-mono ${accentClass} font-semibold shrink-0`}>{"{{" + item.key + "}}"}</span>
                <span className={`text-muted-foreground text-[10px] truncate group-hover:text-foreground text-right ${labelClassName}`}>
                  {item.label}
                </span>
              </button>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
