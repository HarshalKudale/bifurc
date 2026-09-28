import React, { useState } from "react";
import { Environment, EnvVariable } from "@/types";
import { Variable, X, Copy, CheckCircle2 } from "@/lib/icons";
import { strings } from "@/lib/strings";

/**
 * The **variables** face of the right sidebar: every `{{VAR}}` the active environment can resolve,
 * in the two groups the resolver actually uses.
 *
 * ## Why the groups are separate rather than one merged list
 *
 * `mergeEnvVars` overlays the active environment on `__global__`, so a key defined in both resolves
 * to the *environment's* value. A merged list would show one row and hide that relationship
 * entirely — which is the one fact you need when a substitution is not what you expected. So the
 * global group keeps its own rows and marks the shadowed ones (`overriddenBadge`).
 *
 * ## Why this is content, not a sidebar
 *
 * The width, the left border and the open/close transition belong to `RightSidebar`, which swaps
 * this in beside the history face. Nothing here knows how wide it is or whether it is animating.
 */

interface Props {
  /** The workspace's active environment — the one whose values win. Null when none is selected. */
  activeEnvironment: Environment | null;
  /** The `__global__` environment, whose variables apply underneath every environment. */
  globalEnvironment: Environment | null;
  onClose(): void;
}

function VariableRow({ variable, overridden }: { variable: EnvVariable; overridden: boolean }) {
  const [copied, setCopied] = useState(false);
  const token = `{{${variable.key}}}`;

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(token);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    } catch {
      // No clipboard, or the permission was denied. The token is on screen and selectable either
      // way, so there is nothing worth interrupting the user with.
    }
  };

  return (
    <div className="group flex items-start gap-1.5 px-3 py-1.5 hover:bg-card/50">
      <div className="flex-1 min-w-0">
        <div className="flex items-center gap-1.5 min-w-0">
          <code className="text-[11px] font-mono text-signal truncate" title={token}>{token}</code>
          {overridden && (
            <span
              className="text-[9px] uppercase tracking-wide text-amber flex-shrink-0"
              title={strings.rightSidebar.overriddenByEnv}
            >
              {strings.rightSidebar.overriddenBadge}
            </span>
          )}
        </div>
        <div className="text-[10px] font-mono text-muted-foreground break-all">
          {variable.value || <span className="italic opacity-60">{strings.rightSidebar.emptyValue}</span>}
        </div>
      </div>
      <button
        type="button"
        onClick={handleCopy}
        title={strings.rightSidebar.copyReference}
        aria-label={strings.rightSidebar.copyReference}
        className="w-5 h-5 flex items-center justify-center rounded text-muted-foreground hover:text-foreground hover:bg-surface-2 opacity-0 group-hover:opacity-100 focus-visible:opacity-100 transition-opacity flex-shrink-0 cursor-pointer"
      >
        {copied ? <CheckCircle2 size={11} className="text-signal" /> : <Copy size={11} />}
      </button>
    </div>
  );
}

function VariableGroup({
  title, subtitle, variables, emptyText, overriddenKeys,
}: {
  title: string;
  subtitle?: string;
  variables: EnvVariable[];
  emptyText: string;
  /** Keys the active environment also defines — the global rows this shadows. */
  overriddenKeys?: Set<string>;
}) {
  return (
    <div className="border-b border-border/50">
      <div className="flex items-center gap-2 px-3 py-1.5 bg-background/30">
        <span className="text-[10px] font-semibold uppercase tracking-wider text-muted-foreground flex-shrink-0">
          {title}
        </span>
        {subtitle && (
          <span className="text-[10px] text-muted-foreground/70 truncate" title={subtitle}>{subtitle}</span>
        )}
        <span className="ml-auto text-[9px] text-muted-foreground/60 flex-shrink-0">{variables.length}</span>
      </div>
      {variables.length === 0 ? (
        <p className="px-3 py-2 text-[10px] text-muted-foreground italic">{emptyText}</p>
      ) : (
        variables.map((v) => (
          <VariableRow key={v.id} variable={v} overridden={overriddenKeys?.has(v.key) ?? false} />
        ))
      )}
    </div>
  );
}

export default function VariablesSidebar({ activeEnvironment, globalEnvironment, onClose }: Props) {
  const envVars = activeEnvironment?.variables ?? [];
  const globalVars = globalEnvironment?.variables ?? [];
  const envKeys = new Set(envVars.map((v) => v.key));
  const total = envVars.length + globalVars.length;

  return (
    <>
      {/* Header */}
      <div className="flex items-center gap-2 px-3 py-2.5 border-b border-border flex-shrink-0">
        <Variable size={13} className="text-signal flex-shrink-0" />
        <div className="flex-1 min-w-0">
          <div className="text-xs font-semibold text-foreground truncate">{strings.rightSidebar.variablesTitle}</div>
          <div className="text-[10px] text-muted-foreground truncate">
            {activeEnvironment ? activeEnvironment.name : strings.rightSidebar.noActiveEnv}
          </div>
        </div>
        <button
          type="button"
          onClick={onClose}
          title={strings.rightSidebar.closeVariables}
          aria-label={strings.rightSidebar.closeVariables}
          className="w-6 h-6 flex items-center justify-center rounded hover:bg-card text-muted-foreground hover:text-foreground transition-colors flex-shrink-0 cursor-pointer"
        >
          <X size={12} />
        </button>
      </div>

      {/* Total */}
      {total > 0 && (
        <div className="px-3 py-1.5 border-b border-border flex-shrink-0 bg-background/30">
          <span className="text-[10px] text-muted-foreground">
            {strings.rightSidebar.nVariables.replace("{n}", String(total)).replace("{s}", total !== 1 ? "s" : "")}
          </span>
        </div>
      )}

      {/* Groups: the active environment first, then the globals it sits on top of */}
      <div className="flex-1 overflow-y-auto">
        <VariableGroup
          title={strings.rightSidebar.currentEnvGroup}
          subtitle={activeEnvironment?.name}
          variables={envVars}
          emptyText={activeEnvironment ? strings.rightSidebar.noVarsInEnv : strings.rightSidebar.noActiveEnvHint}
        />
        <VariableGroup
          title={strings.rightSidebar.globalGroup}
          variables={globalVars}
          emptyText={strings.rightSidebar.noGlobalVars}
          overriddenKeys={envKeys}
        />
      </div>
    </>
  );
}
