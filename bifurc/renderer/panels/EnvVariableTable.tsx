import React, { useState } from "react";
import { Environment, EnvVariable } from "@/types";
import { Globe, X, History } from "@/lib/icons";
import { IconButton } from "@/components/ui";
import { strings } from "@/lib/strings";
import ActiveDot from "@/components/ui/ActiveDot";

const mkVid = (() => {
  let _vid = 0;
  return () => `v${++_vid}`;
})();

function VarRow({
  row,
  onUpdate,
  onDelete,
}: {
  row: EnvVariable;
  onUpdate: (patch: Partial<EnvVariable>) => void;
  onDelete: () => void;
}) {
  return (
    <div className="flex items-stretch border-b border-border/25 last:border-0 group hover:bg-card/30">
      <div className="flex-1 border-r border-border/25 min-w-0">
        <input
          className="w-full h-full bg-transparent font-mono text-xs px-3 py-2 outline-none focus:bg-card/60"
          style={{ color: "var(--c-signal)" }}
          placeholder="VARIABLE_NAME"
          value={row.key}
          onChange={(e) => onUpdate({ key: e.target.value })}
        />
      </div>
      <div className="flex-1 min-w-0">
        <input
          className="w-full h-full bg-transparent font-mono text-xs text-foreground px-3 py-2 outline-none focus:bg-card/60"
          placeholder="value"
          value={row.value}
          onChange={(e) => onUpdate({ value: e.target.value })}
        />
      </div>
      <button
        onClick={onDelete}
        className="w-9 flex-shrink-0 flex items-center justify-center text-muted-foreground hover:text-destructive opacity-0 group-hover:opacity-100 transition-all cursor-pointer"
      >
        <X size={13} />
      </button>
    </div>
  );
}

export function EnvVariableTable({
  env,
  isActive,
  isGlobal,
  onSave,
  onDelete,
  onActivate,
  onHistory,
}: {
  env: Environment;
  isActive: boolean;
  isGlobal: boolean;
  onSave: (updated: Environment) => Promise<void>;
  onDelete: () => Promise<void>;
  onActivate: () => Promise<void>;
  onHistory?: () => void;
}) {
  const [name, setName] = useState(env.name);
  const [vars, setVars] = useState<EnvVariable[]>(() => env.variables.map((v) => ({ ...v })));
  const [saving, setSaving] = useState(false);
  const [dirty, setDirty] = useState(false);

  const markDirty = () => setDirty(true);

  const updateVar = (id: string, patch: Partial<EnvVariable>) => {
    setVars((prev) => prev.map((v) => (v.id === id ? { ...v, ...patch } : v)));
    markDirty();
  };

  const deleteVar = (id: string) => {
    setVars((prev) => prev.filter((v) => v.id !== id));
    markDirty();
  };

  const addVar = () => {
    setVars((prev) => [...prev, { id: mkVid(), key: "", value: "" }]);
    markDirty();
  };

  const handleSave = async () => {
    setSaving(true);
    try {
      await onSave({ ...env, name: name.trim() || env.name, variables: vars.filter((v) => v.key.trim()) });
      setDirty(false);
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="flex flex-col flex-1 overflow-hidden">
      {/* Header */}
      <div className="flex items-center gap-3 px-5 py-3 border-b border-border flex-shrink-0">
        {isGlobal ? (
          <Globe size={15} className="text-signal flex-shrink-0" />
        ) : (
          <ActiveDot active={isActive} color="accent" size="sm" />
        )}
        {isGlobal ? (
          <span className="flex-1 text-sm font-semibold text-foreground">Global</span>
        ) : (
          <input
            className="flex-1 bg-transparent text-sm font-semibold text-foreground outline-none placeholder:text-muted-foreground min-w-0"
            value={name}
            onChange={(e) => { setName(e.target.value); markDirty(); }}
            placeholder={strings.environments.environmentName}
          />
        )}
        <div className="flex items-center gap-1.5 flex-shrink-0">
          {isGlobal && (
            <span className="px-2.5 py-1 rounded border border-signal/40 bg-signal/10 text-signal text-xs font-semibold">
              {strings.environments.alwaysActive}
            </span>
          )}
          {!isGlobal && !isActive && (
            <button
              onClick={onActivate}
              className="px-2.5 py-1 rounded border border-border bg-card hover:border-signal/50 hover:bg-signal/10 hover:text-signal text-muted-foreground text-xs font-medium transition-all cursor-pointer"
            >
              {strings.environments.setActive}
            </button>
          )}
          {!isGlobal && isActive && (
            <span className="px-2.5 py-1 rounded border border-signal/40 bg-signal/10 text-signal text-xs font-semibold">
              {strings.environments.active}
            </span>
          )}
          {dirty && (
            <button
              onClick={handleSave}
              disabled={saving}
              className="px-2.5 py-1 rounded border border-signal/40 bg-signal/10 hover:bg-signal/20 text-signal text-xs font-semibold transition-all cursor-pointer disabled:opacity-50"
            >
              {saving ? strings.server.saving : strings.common.save}
            </button>
          )}
          {onHistory && (
            <IconButton
              icon={<History size={12} />}
              title={strings.environments.viewHistory}
              onClick={onHistory}
              className="hover:text-signal"
            />
          )}
          {!isGlobal && (
            <button
              onClick={onDelete}
              className="px-2.5 py-1 rounded border border-border bg-card hover:border-destructive/40 hover:bg-destructive/10 hover:text-destructive text-muted-foreground text-xs font-medium transition-all cursor-pointer"
            >
              {strings.common.delete}
            </button>
          )}
        </div>
      </div>

      {/* Variables table */}
      <div className="flex flex-col flex-1 overflow-y-auto">
        <div className="flex items-center border-b border-border/60 bg-background/20 flex-shrink-0">
          <div className="flex-1 px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground border-r border-border/40">
            Variable
          </div>
          <div className="flex-1 px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
            Value
          </div>
          <div className="w-9 flex-shrink-0" />
        </div>

        {vars.length === 0 && (
          <p className="px-4 py-4 text-xs text-muted-foreground italic">{strings.environments.noVariables}</p>
        )}

        {vars.map((v) => (
          <VarRow key={v.id} row={v} onUpdate={(p) => updateVar(v.id, p)} onDelete={() => deleteVar(v.id)} />
        ))}

        <button
          onClick={addVar}
          className="flex items-center gap-2 px-4 py-2.5 text-xs text-muted-foreground hover:text-foreground hover:bg-card/30 transition-colors cursor-pointer w-full text-left border-t border-border/20"
        >
          <span className="text-signal font-semibold text-sm leading-none">+</span>{strings.environments.addVariable}
        </button>
      </div>
    </div>
  );
}
