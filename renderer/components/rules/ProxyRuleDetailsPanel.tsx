import React, { useState, useEffect, useCallback, useMemo } from "react";
import { AppConfig, ProxyRule } from "@/types";
import { strings } from "@/lib/strings";
import { Switch, Button, IconButton } from "@/components/ui";
import { X, History, Trash2, Copy, GitCommit } from "@/lib/icons";
import ProxyRuleForm, { RuleFormState } from "./ProxyRuleForm";

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
        <ProxyRuleForm state={form} errors={errors} onChange={setField} config={config} minimal={false} />
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
