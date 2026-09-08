import React, { useEffect, useRef } from "react";

export interface ContextMenuItem {
  label?: string;
  icon?: React.ReactNode;
  danger?: boolean;
  sep?: boolean;
  disabled?: boolean;
  action?(): void;
}

interface ContextMenuProps {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose(): void;
  minWidth?: number;
}

export default function ContextMenu({ x, y, items, onClose, minWidth = 210 }: ContextMenuProps) {
  const totalH = items.reduce((s, i) => s + (i.sep ? 9 : 30), 0) + 8;
  const ax = Math.min(x, window.innerWidth - minWidth - 8);
  const ay = Math.min(y, window.innerHeight - totalH - 8);
  const menuRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    const h = (e: MouseEvent) => { e.stopPropagation(); onClose(); };
    window.addEventListener("click", h);
    return () => window.removeEventListener("click", h);
  }, [onClose]);

  // Focus first enabled item on mount
  useEffect(() => {
    const first = menuRef.current?.querySelector<HTMLButtonElement>("button:not(:disabled)");
    first?.focus();
  }, []);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") { e.preventDefault(); onClose(); return; }
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const buttons = Array.from(menuRef.current?.querySelectorAll<HTMLButtonElement>("button:not(:disabled)") ?? []);
    if (!buttons.length) return;
    const focused = document.activeElement as HTMLButtonElement;
    const idx = buttons.indexOf(focused);
    if (e.key === "ArrowDown") buttons[(idx + 1) % buttons.length].focus();
    else buttons[(idx - 1 + buttons.length) % buttons.length].focus();
  };

  return (
    <div
      ref={menuRef}
      className="fixed z-50 bg-card border border-border rounded-md shadow-2xl py-1 select-none animate-scale-in"
      style={{ left: ax, top: ay, minWidth }}
      onClick={(e) => e.stopPropagation()}
      onContextMenu={(e) => e.preventDefault()}
      onKeyDown={handleKeyDown}
    >
      {items.map((item, i) =>
        item.sep ? (
          <div key={i} className="mx-2 my-1 border-t border-border/60" />
        ) : (
          <button
            key={i}
            onClick={item.disabled ? undefined : item.action}
            disabled={item.disabled}
            className={`w-full text-left px-3 py-1.5 text-xs font-medium transition-colors flex items-center gap-2 ${
              item.disabled
                ? "opacity-40 cursor-not-allowed text-muted-foreground"
                : item.danger
                ? "text-destructive hover:bg-surface-2 cursor-pointer"
                : "text-foreground hover:bg-surface-2 cursor-pointer"
            }`}
          >
            {item.icon !== undefined && (
              <span className="flex-shrink-0 flex items-center justify-center" style={{ width: 14 }}>
                {item.icon}
              </span>
            )}
            {item.label}
          </button>
        )
      )}
    </div>
  );
}
