import React, { useState } from "react";
import { AppConfig } from "@/types";
import { strings } from "@/lib/strings";
import { Input, Select, FormField } from "@/components/ui";
import CodeEditor from "@/components/common/CodeEditor";

export interface RuleFormState {
  name: string;
  pattern: string;
  useRegex: boolean;
  targetType: "mapping" | "external";
  targetMappingId: string;
  targetExternal: string;
  requestScript: string;
  responseScript: string;
  folderId?: string | null;
}

interface Props {
  state: RuleFormState;
  errors: Partial<Record<keyof RuleFormState, string>>;
  onChange: <K extends keyof RuleFormState>(key: K, value: RuleFormState[K]) => void;
  config: AppConfig;
  minimal?: boolean; // If true, matches RuleTab style slightly more
}

export default function ProxyRuleForm({ state, errors, onChange, config, minimal = false }: Props) {
  const s = strings.proxyRules;
  const [scriptTab, setScriptTab] = useState<"request" | "response">("request");

  return (
    <>
      <FormField label={s.matchUrl} error={errors.pattern}>
        <div className="flex items-center gap-2">
          <Input
            className={`flex-1 font-mono ${!minimal ? "text-xs" : ""}`}
            placeholder={state.useRegex ? "^https?://api\\.example\\.com/.*" : "https://api.example.com/endpoint"}
            value={state.pattern}
            onChange={(e: React.ChangeEvent<HTMLInputElement>) => onChange("pattern", e.target.value)}
            error={!!errors.pattern}
          />
          <button
            type="button"
            onClick={() => onChange("useRegex", !state.useRegex)}
            className={`px-3 py-1.5 rounded border text-xs font-semibold transition-colors cursor-pointer flex-shrink-0 ${
              state.useRegex
                ? "border-signal bg-signal/15 text-signal"
                : "border-border bg-card text-muted-foreground hover:text-foreground"
            }`}
            title={state.useRegex ? s.switchToExact : s.switchToRegex}
          >
            {state.useRegex ? s.regexToggle : s.exactToggle}
          </button>
        </div>
        <p className={`${minimal ? "text-xs" : "text-[11px]"} text-muted-foreground mt-1`}>
          {state.useRegex ? s.regexHelp : s.exactHelp}
        </p>
      </FormField>

      <div className={!minimal ? "border border-border/70 rounded-lg p-3 bg-card/40" : ""}>
        <div className={`${!minimal ? "text-[10px]" : "text-xs"} text-muted-foreground font-semibold uppercase tracking-wider mb-2.5`}>
          {s.forwardTo}
        </div>
        <div className="flex items-center gap-4 mb-3">
          {(["mapping", "external"] as const).map((type) => (
            <label key={type} className={`flex items-center ${minimal ? "gap-1.5 text-sm" : "gap-2 text-xs"} cursor-pointer font-medium text-foreground`}>
              <input
                type="radio"
                name={minimal ? undefined : "targetType"}
                className="accent-signal"
                checked={state.targetType === type}
                onChange={() => onChange("targetType", type)}
              />
              {type === "mapping" ? s.targetMapping : s.targetExternal}
            </label>
          ))}
        </div>

        {state.targetType === "mapping" ? (
          <FormField label="" error={errors.targetMappingId}>
            <Select
              className={`w-full ${!minimal ? "text-xs font-mono" : ""}`}
              error={!!errors.targetMappingId}
              value={state.targetMappingId}
              onChange={(e: React.ChangeEvent<HTMLSelectElement>) => onChange("targetMappingId", e.target.value)}
            >
              <option value="">{s.selectMapping}</option>
              {(config.mappings ?? []).map((m) => (
                <option key={m.id} value={m.id}>
                  {m.domain} → {m.target}
                </option>
              ))}
            </Select>
            {(config.mappings ?? []).length === 0 && (
              <p className="text-xs text-muted-foreground mt-1">{s.noMappingsDefined}</p>
            )}
          </FormField>
        ) : (
          <FormField label="" error={errors.targetExternal}>
            <Input
              className={`w-full font-mono ${!minimal ? "text-xs" : ""}`}
              placeholder="api.example.com:8080 or 127.0.0.1:3000"
              value={state.targetExternal}
              onChange={(e: React.ChangeEvent<HTMLInputElement>) => onChange("targetExternal", e.target.value)}
              error={!!errors.targetExternal}
            />
            <p className={`${minimal ? "text-xs" : "text-[11px]"} text-muted-foreground mt-1`}>host:port (e.g. api.example.com:8080 or 127.0.0.1:3000)</p>
          </FormField>
        )}
      </div>

      <div className={`flex flex-col flex-1 ${!minimal ? "min-h-[220px] border border-border/70 rounded-lg overflow-hidden" : "min-h-0"}`}>
        <div className={`flex items-center gap-0 border-b border-border ${!minimal ? "bg-card/40" : "mb-0"}`}>
          {(["request", "response"] as const).map((tab) => (
            <button
              key={tab}
              type="button"
              onClick={() => setScriptTab(tab)}
              className={`px-3.5 py-2 text-xs font-medium border-b-2 transition-colors cursor-pointer -mb-px ${
                scriptTab === tab
                  ? !minimal ? "border-signal text-signal bg-surface" : "border-signal text-signal"
                  : "border-transparent text-muted-foreground hover:text-foreground"
              }`}
            >
              {tab === "request" ? s.requestScript : s.responseScript}
              {!minimal && tab === "request" && state.requestScript.trim() && (
                <span className="ml-1.5 w-1.5 h-1.5 rounded-full bg-signal inline-block" />
              )}
              {!minimal && tab === "response" && state.responseScript.trim() && (
                <span className="ml-1.5 w-1.5 h-1.5 rounded-full bg-amber inline-block" />
              )}
            </button>
          ))}
        </div>
        <div className={`flex-1 ${!minimal ? "min-h-[180px] bg-background" : "min-h-[200px] border-t border-border"} relative`} style={minimal ? { minHeight: 200 } : undefined}>
          {scriptTab === "request" ? (
            <CodeEditor
              key="req-script"
              language="javascript"
              value={state.requestScript}
              onChange={(v) => onChange("requestScript", v)}
              placeholder={s.requestScriptPlaceholder}
              className="w-full h-full"
              minHeight={minimal ? 200 : 180}
            />
          ) : (
            <CodeEditor
              key="res-script"
              language="javascript"
              value={state.responseScript}
              onChange={(v) => onChange("responseScript", v)}
              placeholder={s.responseScriptPlaceholder}
              className="w-full h-full"
              minHeight={minimal ? 200 : 180}
            />
          )}
        </div>
      </div>
    </>
  );
}
