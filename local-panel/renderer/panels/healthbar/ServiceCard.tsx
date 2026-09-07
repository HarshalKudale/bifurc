import React, { memo, useCallback } from "react";
import { HealthBarService } from "@/types";
import { CheckStatus, ServiceState } from "./types";
import { strings } from "@/lib/strings";
import { Badge, StatusDot, Switch, IconButton } from "@/components/ui";
import { RefreshCw, Trash2 } from "@/lib/icons";

function statusColor(s: CheckStatus, code: number | null): "green" | "red" | "yellow" | "dim" {
  if (s === "idle") return "dim";
  if (s === "checking") return "yellow";
  if (s === "error") return "red";
  if (code !== null && code >= 200 && code < 300) return "green";
  return "red";
}

function cardBorderClass(s: CheckStatus, code: number | null): string {
  if (s === "idle") return "border-border";
  if (s === "checking") return "border-amber/40";
  if (s === "error") return "border-destructive/40";
  if (code !== null && code >= 200 && code < 300) return "border-signal/40";
  return "border-destructive/40";
}

function cardBgClass(s: CheckStatus, code: number | null): string {
  if (s === "idle") return "";
  if (s === "checking") return "bg-amber/5";
  if (s === "error") return "bg-destructive/5";
  if (code !== null && code >= 200 && code < 300) return "bg-signal/5";
  return "bg-destructive/5";
}

function statusLabel(s: CheckStatus, code: number | null, error: string | null): string {
  if (s === "idle") return strings.healthBar.notChecked;
  if (s === "checking") return strings.healthBar.checking;
  if (s === "error") return error ?? strings.healthBar.error;
  if (code !== null) return `${code}`;
  return strings.healthBar.unknown;
}

function badgeVariant(s: CheckStatus, code: number | null): "green" | "red" | "yellow" | "neutral" {
  if (s === "idle") return "neutral";
  if (s === "checking") return "yellow";
  if (s === "error") return "red";
  if (code !== null && code >= 200 && code < 300) return "green";
  return "red";
}

function formatTs(ts: number | null): string {
  if (!ts) return "";
  const d = new Date(ts);
  return d.toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit", second: "2-digit" });
}

interface ServiceCardProps {
  service: HealthBarService;
  state: ServiceState;
  resolvedUrl: string;
  onRefresh: (id: string) => void;
  onToggleAutoRefresh: (id: string, enabled: boolean) => void;
  onDelete: (id: string) => void;
  onClick: (id: string) => void;
}

const ServiceCard = memo(function ServiceCard({
  service,
  state,
  resolvedUrl,
  onRefresh,
  onToggleAutoRefresh,
  onDelete,
  onClick,
}: ServiceCardProps) {
  const dot = statusColor(state.status, state.statusCode);
  const border = cardBorderClass(state.status, state.statusCode);
  const bg = cardBgClass(state.status, state.statusCode);
  const label = statusLabel(state.status, state.statusCode, state.error);
  const bv = badgeVariant(state.status, state.statusCode);

  const handleToggle = useCallback((enabled: boolean) => {
    onToggleAutoRefresh(service.id, enabled);
  }, [service.id, onToggleAutoRefresh]);

  const handleRefreshClick = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
    onRefresh(service.id);
  }, [service.id, onRefresh]);

  const handleDeleteClick = useCallback((e: React.MouseEvent) => {
    e.stopPropagation();
    onDelete(service.id);
  }, [service.id, onDelete]);

  const handleCardClick = useCallback(() => {
    onClick(service.id);
  }, [service.id, onClick]);

  return (
    <div
      className={`rounded-lg border ${border} ${bg} overflow-hidden transition-all`}
    >
      <button
        className="w-full text-left p-4 cursor-pointer hover:bg-card/30 transition-colors"
        onClick={handleCardClick}
        title={strings.healthBar.viewLastResponse}
      >
        <div className="flex items-start gap-3">
          <StatusDot
            color={dot}
            pulse={state.status === "checking"}
            size="md"
            className="mt-0.5 flex-shrink-0"
          />
          <div className="flex-1 min-w-0">
            <p className="text-sm font-semibold text-foreground truncate">{service.name}</p>
            <p className="text-xs text-muted-foreground font-mono truncate mt-0.5" title={resolvedUrl}>
              {resolvedUrl || service.url}
            </p>
          </div>
          <Badge variant={bv} className="flex-shrink-0 mt-0.5">
            {label}
          </Badge>
        </div>

        {(state.durationMs !== null || state.checkedAt !== null) && (
          <div className="flex items-center gap-3 mt-2.5 pl-7">
            {state.durationMs !== null && (
              <span className="text-xs text-muted-foreground">{state.durationMs}ms</span>
            )}
            {state.checkedAt !== null && (
              <span className="text-xs text-muted-foreground">
                {strings.healthBar.lastChecked} {formatTs(state.checkedAt)}
              </span>
            )}
          </div>
        )}
      </button>

      <div className="flex items-center gap-2 px-4 py-2 border-t border-border/40 bg-background/20">
        <span className="text-xs text-muted-foreground flex-shrink-0">{strings.healthBar.autoRefresh}</span>
        <Switch checked={service.autoRefreshEnabled} onChange={handleToggle} />
        <div className="flex-1" />
        <IconButton
          icon={<RefreshCw size={13} className={state.status === "checking" ? "animate-spin" : ""} />}
          title={strings.healthBar.refreshService}
          onClick={handleRefreshClick}
          disabled={state.status === "checking"}
        />
        <IconButton
          icon={<Trash2 size={13} />}
          title={strings.healthBar.removeService}
          onClick={handleDeleteClick}
          className="hover:border-destructive/40 hover:text-destructive"
        />
      </div>
    </div>
  );
});

export default ServiceCard;
