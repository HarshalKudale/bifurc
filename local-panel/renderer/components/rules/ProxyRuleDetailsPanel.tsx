import React, { useState, useEffect, useCallback, useMemo } from "react";
import { AppConfig, ProxyRule } from "@/types";
import CodeEditor from "@/components/common/CodeEditor";
import { strings } from "@/lib/strings";
import { Input, Select, FormField, Switch, Button, IconButton } from "@/components/ui";
import { X, History, Trash2, Copy, GitCommit } from "@/lib/icons";

export interface RuleSavePayload {
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
  rule: Partial<ProxyRule> | null;
  isNew: boolean;
  config: AppConfig;
  onSave: (data: RuleSavePayload) => Promise<any>;
  onClose: () => void;
  onDelete?: () => void;
  onDuplicate?: () => void;
  onToggleEnabled?: () => void;
  enabled?: boolean;
  onSync?: () => Promise<void>;
  onRevert?: () => Promise<void>;
  onHistory?: () => void;
  syncStatus?: "clean" | "modified" | "new" | "deleted";
}

interface RuleFormState {
  name: string;
  pattern: string;
  useRegex: boolean;
  targetType: "mapping" | "external";
  targetMappingId: string;
  targetExternal: string;
  requestScript: string;
  responseScript: string;
}

function stateFromRule(rule: Partial<ProxyRule> | null): RuleFormState {
  return {
    name: rule?.name ?? "",
    pattern: rule?.pattern ?? "",
    useRegex: rule?.useRegex ?? true,
    targetType: rule?.targetType ?? "mapping",
    targetMappingId: rule?.targetMappingId ?? "",
    targetExternal: rule?.targetExternal ?? "",
    requestScript: rule?.requestScript ?? "",
    responseScript: rule?.responseScript ?? "",
  };
}

export default function ProxyRuleDetailsPanel({
  rule,
  isNew,
  config,
  onSave,
  onClose,
  onDelete,
  onDuplicate,
  onToggleEnabled,
  enabled,
  onSync,
  onRevert,
  onHistory,
  syncStatus,
}: Props) {
  const s = strings.proxyRules;

  const [form, setForm] = useState<RuleFormState>(() => stateFromRule(rule));
  const [saving, setSaving] = useState(false);
  const [syncing, setSyncing] = useState(false);
  const [reverting, setReverting] = useState(false);
  const [errors, setErrors] = useState<Partial<Record<keyof RuleFormState, string>>>({});
  const [scriptTab, setScriptTab] = useState<"request" | "response">("request");

  useEffect(() => {
    setForm(stateFromRule(rule));
    setErrors({});
  }, [rule, isNew]);

  const setField = useCallback(<K extends keyof RuleFormState>(key: K, value: RuleFormState[K]) => {
    setForm((prev) => ({ ...prev, [key]: value }));
    setErrors((prev) => {
      const next = { ...prev };
      delete next[key];
      return next;
    });
  }, []);

  const isDirty = useMemo(() => {
    if (isNew) {
      return Boolean(form.name || form.pattern || form.requestScript || form.responseScript);
    }
    const initial = stateFromRule(rule);
    return (
      form.name !== initial.name ||
      form.pattern !== initial.pattern ||
      form.useRegex !== initial.useRegex ||
      form.targetType !== initial.targetType ||
      form.targetMappingId !== initial.targetMappingId ||
      form.targetExternal !== initial.targetExternal ||
      form.requestScript !== initial.requestScript ||
      form.responseScript !== initial.responseScript
    );
  }, [form, rule, isNew]);

  const validate = (): boolean => {
    const errs: Partial<Record<keyof RuleFormState, string>> = {};
    if (!form.pattern.trim()) {
      errs.pattern = s.patternRequired;
    } else if (form.useRegex) {
      try {
        new RegExp(form.pattern);
      } catch {
        errs.pattern = s.invalidRegexPattern;
      }
    }

    if (form.targetType === "mapping" && !form.targetMappingId) {
      errs.targetMappingId = s.selectTargetMapping;
    }
    if (form.targetType === "external" && !form.targetExternal.trim()) {
      errs.targetExternal = s.enterHostPort;
    }

    setErrors(errs);
    return Object.keys(errs).length === 0;
  };

  const handleSaveClick = async () => {
    if (!validate()) return;
    setSaving(true);
    try {
      await onSave({
        name: form.name.trim(),
        pattern: form.pattern.trim(),
        useRegex: form.useRegex,
        targetType: form.targetType,
        targetMappingId: form.targetMappingId,
        targetExternal: form.targetExternal.trim(),
        requestScript: form.requestScript,
        responseScript: form.responseScript,
        folderId: rule?.folderId ?? null,
      });
    } finally {
      setSaving(false);
    }
  };

  const handleSyncClick = async () => {
    if (!onSync || syncing) return;
    setSyncing(true);
    try {
      if (isDirty) {
        if (!validate()) return;
        await handleSaveClick();
      }
      await onSync();
    } finally {
      setSyncing(false);
    }
  };

  const handleRevertClick = async () => {
    if (!onRevert || reverting) return;
    setReverting(true);
    try {
      await onRevert();
    } finally {
      setReverting(false);
    }
  };

  const hasChanges = !isNew && Boolean(isDirty || (syncStatus && syncStatus !== "clean"));

  return (
    <div className="flex flex-col flex-1 overflow-hidden h-full bg-surface">
      {/* Header bar */}
      <div className="flex items-center gap-3 px-4 py-3 border-b border-border flex-shrink-0 bg-surface">
        <div className="flex items-center gap-2">
          <span className="text-[10px] font-semibold text-muted-foreground uppercase tracking-widest whitespace-nowrap">
            {isNew ? s.newRuleTitle : s.ruleLabel}
          </span>
          {!isNew && onToggleEnabled && enabled !== undefined && (
            <Switch checked={enabled} onChange={onToggleEnabled} title={enabled ? "Disable rule" : "Enable rule"} />
          )}
        </div>

        <input
          className="flex-1 bg-card border border-border focus:border-signal rounded px-3 py-1 text-sm text-foreground outline-none placeholder:text-muted-foreground transition-colors min-w-0"
          placeholder={s.ruleNamePlaceholder}
          value={form.name}
          onChange={(e) => setField("name", e.target.value)}
          autoFocus={isNew}
        />

        <div className="flex items-center gap-1 flex-shrink-0">
          {!isNew && onDuplicate && (
            <IconButton
              icon={<Copy size={13} />}
              title={strings.folderTree.duplicate}
              onClick={onDuplicate}
              className="text-muted-foreground hover:text-foreground"
            />
          )}
          {!isNew && onDelete && (
            <IconButton
              icon={<Trash2 size={13} />}
              title={strings.common.delete}
              onClick={onDelete}
              className="text-muted-foreground hover:text-destructive"
            />
          )}
          <IconButton
            icon={<X size={14} />}
            title={strings.common.close}
            onClick={onClose}
            className="text-muted-foreground hover:text-foreground ml-1"
          />
        </div>
      </div>

      {/* Main Form Body */}
      <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-4 min-h-0">
        {/* Match Pattern */}
        <FormField label={s.matchUrl} error={errors.pattern}>
          <div className="flex items-center gap-2">
            <Input
              className="flex-1 font-mono text-xs"
              placeholder={form.useRegex ? "^https?://api\\.example\\.com/.*" : "https://api.example.com/endpoint"}
              value={form.pattern}
              onChange={(e) => setField("pattern", e.target.value)}
              error={!!errors.pattern}
            />
            <button
              type="button"
              onClick={() => setField("useRegex", !form.useRegex)}
              className={`px-2.5 py-1.5 rounded border text-xs font-semibold transition-colors cursor-pointer flex-shrink-0 ${
                form.useRegex
                  ? "border-signal bg-signal/15 text-signal"
                  : "border-border bg-card text-muted-foreground hover:text-foreground"
              }`}
              title={form.useRegex ? s.switchToExact : s.switchToRegex}
            >
              {form.useRegex ? s.regexToggle : s.exactToggle}
            </button>
          </div>
          <p className="text-[11px] text-muted-foreground mt-1">
            {form.useRegex ? s.regexHelp : s.exactHelp}
          </p>
        </FormField>

        {/* Forward Target */}
        <div className="border border-border/70 rounded-lg p-3 bg-card/40">
          <div className="text-[10px] text-muted-foreground font-semibold uppercase tracking-wider mb-2.5">
            {s.forwardTo}
          </div>
          <div className="flex items-center gap-4 mb-3">
            <label className="flex items-center gap-2 cursor-pointer text-xs font-medium text-foreground">
              <input
                type="radio"
                name="targetType"
                className="accent-signal"
                checked={form.targetType === "mapping"}
                onChange={() => setField("targetType", "mapping")}
              />
              {s.targetMapping}
            </label>
            <label className="flex items-center gap-2 cursor-pointer text-xs font-medium text-foreground">
              <input
                type="radio"
                name="targetType"
                className="accent-signal"
                checked={form.targetType === "external"}
                onChange={() => setField("targetType", "external")}
              />
              {s.targetExternal}
            </label>
          </div>

          {form.targetType === "mapping" ? (
            <FormField label="" error={errors.targetMappingId}>
              <Select
                className="w-full text-xs font-mono"
                error={!!errors.targetMappingId}
                value={form.targetMappingId}
                onChange={(e) => setField("targetMappingId", e.target.value)}
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
                className="w-full font-mono text-xs"
                placeholder="api.example.com:8080 or 127.0.0.1:3000"
                value={form.targetExternal}
                onChange={(e) => setField("targetExternal", e.target.value)}
                error={!!errors.targetExternal}
              />
              <p className="text-[11px] text-muted-foreground mt-1">host:port (e.g. api.example.com:8080 or 127.0.0.1:3000)</p>
            </FormField>
          )}
        </div>

        {/* Scripts section */}
        <div className="flex flex-col flex-1 min-h-[220px] border border-border/70 rounded-lg overflow-hidden">
          <div className="flex items-center gap-0 border-b border-border bg-card/40">
            {(["request", "response"] as const).map((tab) => (
              <button
                key={tab}
                type="button"
                onClick={() => setScriptTab(tab)}
                className={`px-3.5 py-2 text-xs font-medium border-b-2 transition-colors cursor-pointer -mb-px ${
                  scriptTab === tab
                    ? "border-signal text-signal bg-surface"
                    : "border-transparent text-muted-foreground hover:text-foreground"
                }`}
              >
                {tab === "request" ? s.requestScript : s.responseScript}
                {tab === "request" && form.requestScript.trim() && (
                  <span className="ml-1.5 w-1.5 h-1.5 rounded-full bg-signal inline-block" />
                )}
                {tab === "response" && form.responseScript.trim() && (
                  <span className="ml-1.5 w-1.5 h-1.5 rounded-full bg-amber inline-block" />
                )}
              </button>
            ))}
          </div>
          <div className="flex-1 min-h-[180px] relative bg-background">
            {scriptTab === "request" ? (
              <CodeEditor
                key="req-script"
                language="javascript"
                value={form.requestScript}
                onChange={(v) => setField("requestScript", v)}
                placeholder={s.requestScriptPlaceholder}
                className="w-full h-full"
                minHeight={180}
              />
            ) : (
              <CodeEditor
                key="res-script"
                language="javascript"
                value={form.responseScript}
                onChange={(v) => setField("responseScript", v)}
                placeholder={s.responseScriptPlaceholder}
                className="w-full h-full"
                minHeight={180}
              />
            )}
          </div>
        </div>
      </div>

      {/* Footer toolbar */}
      <div className="flex items-center justify-between gap-2 px-4 py-3 border-t border-border bg-surface flex-shrink-0">
        <div className="flex items-center gap-1.5">
          {!isNew && onSync && (
            <Button
              variant="ghost"
              size="sm"
              onClick={handleSyncClick}
              disabled={syncing}
              title={hasChanges ? strings.common.syncTooltip : strings.common.noChangesToSync}
            >
              <GitCommit size={13} className="mr-1" />
              {syncing ? strings.footer.publishing : strings.mappings.publish}
            </Button>
          )}
          {!isNew && onRevert && (
            <button
              onClick={handleRevertClick}
              disabled={reverting || !hasChanges}
              className="px-2.5 py-1 rounded text-amber hover:bg-amber/10 text-xs font-medium transition-all disabled:opacity-40 cursor-pointer disabled:cursor-not-allowed"
            >
              {reverting ? "Reverting…" : strings.mappings.revert}
            </button>
          )}
          {!isNew && onHistory && (
            <IconButton
              icon={<History size={13} />}
              title={strings.mappings.viewHistory}
              onClick={onHistory}
              className="text-muted-foreground hover:text-signal"
            />
          )}
        </div>

        <div className="flex items-center gap-2">
          <Button variant="ghost" size="sm" onClick={onClose}>
            {strings.common.cancel}
          </Button>
          <Button variant="primary" size="sm" onClick={handleSaveClick} disabled={saving}>
            {saving ? strings.server.saving : isNew ? s.saveRule : s.updateRule}
          </Button>
        </div>
      </div>
    </div>
  );
}
