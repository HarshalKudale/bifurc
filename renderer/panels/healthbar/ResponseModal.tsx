import React, { useEffect } from "react";
import { HealthBarService } from "@/types";
import { ServiceState } from "./types";
import { strings } from "@/lib/strings";
import { Badge, StatusDot, Button } from "@/components/ui";
import { X } from "@/lib/icons";

function formatTs(ts: number | null): string {
  if (!ts) return "";
  const d = new Date(ts);
  return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

function tryFormatJson(text: string | null): string {
  if (!text) return "";
  try { return JSON.stringify(JSON.parse(text), null, 2); } catch { return text; }
}

interface ResponseModalProps {
  open: boolean;
  service: HealthBarService | null;
  state: ServiceState | null;
  onClose: () => void;
}

export default function ResponseModal({
  open,
  service,
  state,
  onClose,
}: ResponseModalProps) {
  useEffect(() => {
    if (!open) return;
    const handler = (e: KeyboardEvent) => { if (e.key === "Escape") onClose(); };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [open, onClose]);

  if (!open || !service || !state) return null;

  const isSuccess = state.status === "success" && state.statusCode !== null && state.statusCode >= 200 && state.statusCode < 300;
  const isError = state.status === "error" || (state.statusCode !== null && state.statusCode >= 300);

  const hasHeaders = state.headers && Object.keys(state.headers).length > 0;
  const hasBody = state.body !== null;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/60 backdrop-blur-sm p-8"
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div className="bg-surface border border-border rounded-lg shadow-2xl flex flex-col w-full max-w-5xl h-[70%] overflow-hidden">
        <div className="flex items-center gap-3 px-6 py-4 border-b border-border flex-shrink-0">
          <StatusDot color={isSuccess ? "green" : isError ? "red" : "dim"} size="md" />
          <div className="flex-1 min-w-0">
            <p className="text-sm font-semibold text-foreground">{service.name}</p>
            <p className="text-xs text-muted-foreground font-mono truncate mt-0.5">{service.url}</p>
          </div>
          <div className="flex items-center gap-2 flex-shrink-0">
            {state.statusCode !== null && (
              <Badge variant={isSuccess ? "green" : "red"}>{state.statusCode}</Badge>
            )}
            {state.durationMs !== null && (
              <span className="text-xs text-muted-foreground">{state.durationMs}ms</span>
            )}
            {state.checkedAt !== null && (
              <span className="text-xs text-muted-foreground">· {formatTs(state.checkedAt)}</span>
            )}
            <button
              onClick={onClose}
              className="w-7 h-7 flex items-center justify-center rounded hover:bg-surface-2 text-muted-foreground hover:text-foreground transition-colors cursor-pointer ml-1"
            >
              <X size={14} />
            </button>
          </div>
        </div>

        {state.error && (
          <div className="px-6 py-3 bg-destructive/5 border-b border-destructive/20 text-xs text-destructive font-mono break-all flex-shrink-0">
            {state.error}
          </div>
        )}

        <div className="flex-1 flex overflow-hidden min-h-0">
          <div className="w-80 flex-shrink-0 border-r border-border flex flex-col overflow-hidden">
            <div className="px-4 py-2.5 border-b border-border/60 bg-background/30 flex-shrink-0">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                Response Headers
              </span>
            </div>
            <div className="flex-1 overflow-y-auto">
              {hasHeaders ? (
                Object.entries(state.headers!).map(([k, v]) => (
                  <div key={k} className="border-b border-border/20 last:border-0 px-4 py-2 hover:bg-card/30">
                    <p className="text-[11px] font-mono text-signal truncate">{k}</p>
                    <p className="text-[11px] font-mono text-muted-foreground break-all mt-0.5">{String(v)}</p>
                  </div>
                ))
              ) : (
                <p className="px-4 py-4 text-xs text-muted-foreground italic">{strings.common.noHeaders}</p>
              )}
            </div>
          </div>

          <div className="flex-1 flex flex-col overflow-hidden">
            <div className="px-4 py-2.5 border-b border-border/60 bg-background/30 flex-shrink-0">
              <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                Response Body
              </span>
            </div>
            <div className="flex-1 overflow-auto p-4">
              {hasBody ? (
                <pre className="text-xs text-muted-foreground font-mono whitespace-pre-wrap break-all leading-relaxed">
                  {tryFormatJson(state.body)}
                </pre>
              ) : (
                <p className="text-xs text-muted-foreground italic">No body</p>
              )}
            </div>
          </div>
        </div>

        <div className="flex justify-end px-6 py-3 border-t border-border flex-shrink-0">
          <Button variant="secondary" onClick={onClose}>{strings.common.close}</Button>
        </div>
      </div>
    </div>
  );
}
