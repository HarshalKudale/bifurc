import React from "react";
import { Environment } from "@/types";
import { strings } from "@/lib/strings";
import { Globe, ChevronDown } from "@/lib/icons";
import { Button } from "@/components/ui";

interface Props {
  environments: Environment[];
  activeId: string | null;
  open: boolean;
  dropdownRef: React.Ref<HTMLDivElement>;
  onToggle: () => void;
  onClose: () => void;
  onSelect: (id: string | null) => void;
  onManage: () => void;
}

export default function EnvSelector({ environments, activeId, open, dropdownRef, onToggle, onClose, onSelect, onManage }: Props) {
  const active = environments.find((e) => e.id === activeId) ?? null;

  return (
    <div
      ref={dropdownRef}
      className="relative flex-shrink-0"
      style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
    >
      <button
        type="button"
        onClick={onToggle}
        aria-haspopup="menu"
        aria-expanded={open}
        title={strings.titleBar.switchEnvironment}
        className={`flex min-h-8 items-center gap-1.5 px-2.5 py-1 rounded-md border text-xs font-medium transition-colors cursor-pointer select-none focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-signal/35 ${
          open
            ? "border-signal/40 bg-signal/10 text-signal"
            : "border-border/80 bg-card/60 text-muted-foreground hover:bg-surface-2 hover:text-foreground"
        }`}
      >
        <Globe size={11} className="flex-shrink-0" />
        <span className="max-w-[130px] truncate text-foreground font-medium">
          {active ? active.name : strings.titleBar.noEnvironment}
        </span>
        <ChevronDown size={11} className="opacity-60 flex-shrink-0" />
      </button>

      {open && (
        <>
          <div className="fixed inset-0 z-40" onClick={onClose} />
          <div className="absolute top-full mt-1 right-0 z-50 bg-card border border-border rounded-md shadow-2xl py-1 min-w-[220px] animate-scale-in">
            <div className="px-3 py-2 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground border-b border-border/60">
              {strings.titleBar.environment}
            </div>
            <button
              type="button"
              onClick={() => onSelect(null)}
              className={`w-full text-left px-3 py-2 text-sm cursor-pointer hover:bg-surface-2 flex items-center gap-2 ${activeId === null ? "text-signal font-semibold" : "text-foreground"}`}
            >
              <span className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${activeId === null ? "bg-signal" : "bg-muted-foreground/30"}`} />
              {strings.titleBar.noEnv}
            </button>
            {environments.map((env) => (
              <button
                type="button"
                key={env.id}
                onClick={() => onSelect(env.id)}
                className={`w-full text-left px-3 py-2 text-sm cursor-pointer hover:bg-surface-2 flex items-center gap-2 ${activeId === env.id ? "text-signal font-semibold" : "text-foreground"}`}
              >
                <span
                  className={`w-1.5 h-1.5 rounded-full flex-shrink-0 ${activeId === env.id ? "bg-signal" : "bg-muted-foreground/30"}`}
                  style={{ boxShadow: activeId === env.id ? "0 0 4px var(--c-signal)" : "none" }}
                />
                <span className="truncate">{env.name}</span>
                <span className="ml-auto text-[10px] text-muted-foreground font-mono">{env.variables.length}v</span>
              </button>
            ))}
            <div className="border-t border-border/60 mt-1 pt-1">
              <Button
                variant="ghost"
                size="sm"
                onClick={onManage}
                className="w-full justify-start rounded-none px-3 text-sm text-muted-foreground hover:text-signal"
              >
                {strings.titleBar.manageEnvironments}
              </Button>
            </div>
          </div>
        </>
      )}
    </div>
  );
}
