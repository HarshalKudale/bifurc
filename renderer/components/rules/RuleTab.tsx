import React, { useState, useCallback, useMemo, useImperativeHandle, forwardRef } from "react";
import { AppConfig, ProxyRule, Folder } from "@/types";
import EditorTitleBar from "@/components/editor/EditorTitleBar";
import { BottomBar } from "@/components/editor/RequestTab";
import { useDraftPersist, loadDraft } from "@/hooks/useDraftPersist";
import { strings } from "@/lib/strings";
import ProxyRuleForm from "./ProxyRuleForm";

// -- Types ------------------------------------------------------------------

export interface RuleTabHandle {
  refresh(rule: ProxyRule): void;
  save(): Promise<any> | void;
}

interface RuleTabState {
  name: string;
  pattern: string;
  useRegex: boolean;
  targetType: "mapping" | "external";
  targetMappingId: string;
  targetExternal: string;
  requestScript: string;
  responseScript: string;
  folderId: string | null;
}

export interface RuleSavePayload {
  name: string;
  pattern: string;
  useRegex: boolean;
  targetType: "mapping" | "external";
  targetMappingId: string;
  targetExternal: string;
  requestScript: string;
  responseScript: string;
  folderId: string | null;
}

interface Props {
  tabId: string;
  draftTabId: string | null;
  initial: Partial<ProxyRule> | null;
  folders: Folder[];
  config: AppConfig;
  onSave(data: RuleSavePayload): Promise<any>;
  onClose(): void;
  enabled?: boolean;
  onToggleEnabled?: () => void;
  onSync?: (savedId?: string) => Promise<void>;
  onRevert?: () => Promise<void>;
  syncStatus?: "clean" | "modified" | "new" | "deleted";
  onHistory?: () => void;
}

// -- RuleDraft type for localStorage ---------------------------------------

interface RuleDraft {
  name?: string;
  pattern?: string;
  useRegex?: boolean;
  targetType?: "mapping" | "external";
  targetMappingId?: string;
  targetExternal?: string;
  requestScript?: string;
  responseScript?: string;
  folderId?: string | null;
}

function stateFromRule(rule: Partial<ProxyRule> | null): RuleTabState {
  return {
    name: rule?.name ?? "",
    pattern: rule?.pattern ?? "",
    useRegex: rule?.useRegex ?? true,
    targetType: rule?.targetType ?? "mapping",
    targetMappingId: rule?.targetMappingId ?? "",
    targetExternal: rule?.targetExternal ?? "",
    requestScript: rule?.requestScript ?? "",
    responseScript: rule?.responseScript ?? "",
    folderId: rule?.folderId ?? null,
  };
}

function stateFromDraft(d: RuleDraft): RuleTabState {
  return {
    name: d.name ?? "",
    pattern: d.pattern ?? "",
    useRegex: d.useRegex ?? true,
    targetType: d.targetType ?? "mapping",
    targetMappingId: d.targetMappingId ?? "",
    targetExternal: d.targetExternal ?? "",
    requestScript: d.requestScript ?? "",
    responseScript: d.responseScript ?? "",
    folderId: d.folderId ?? null,
  };
}

function isDraftEmpty(s: RuleTabState): boolean {
  return !s.name && !s.pattern && !s.requestScript && !s.responseScript;
}

// -- RuleTab component ------------------------------------------------------

export default forwardRef<RuleTabHandle, Props>(function RuleTab(
  { tabId, draftTabId, initial, folders, config, onSave, onClose, enabled, onToggleEnabled, onSync, onRevert, syncStatus, onHistory },
  ref,
) {
  const isDraft = draftTabId !== null;

  const [state, setState] = useState<RuleTabState>(() => {
    if (isDraft) {
      const saved = loadDraft<RuleDraft>(draftTabId);
      return saved ? stateFromDraft(saved) : stateFromRule(initial);
    }
    return stateFromRule(initial);
  });

  const [saving, setSaving] = useState(false);
  const [errors, setErrors] = useState<Partial<Record<keyof RuleTabState, string>>>({});

  const set = useCallback(<K extends keyof RuleTabState>(key: K, val: RuleTabState[K]) => {
    setState((prev) => ({ ...prev, [key]: val }));
    setErrors((prev) => { const next = { ...prev }; delete next[key]; return next; });
  }, []);

  // Draft auto-save
  const { markSaved } = useDraftPersist(
    draftTabId,
    () => ({
      name: state.name, pattern: state.pattern, useRegex: state.useRegex,
      targetType: state.targetType, targetMappingId: state.targetMappingId,
      targetExternal: state.targetExternal,
      requestScript: state.requestScript, responseScript: state.responseScript,
      folderId: state.folderId,
    } as RuleDraft),
    () => isDraftEmpty(state),
  );

  const isDirty = useMemo(() => {
    if (isDraft) return !isDraftEmpty(state);
    const init = stateFromRule(initial);
    return (
      state.name !== init.name ||
      state.pattern !== init.pattern ||
      state.useRegex !== init.useRegex ||
      state.targetType !== init.targetType ||
      state.targetMappingId !== init.targetMappingId ||
      state.targetExternal !== init.targetExternal ||
      state.requestScript !== init.requestScript ||
      state.responseScript !== init.responseScript ||
      state.folderId !== init.folderId
    );
  }, [state, initial, isDraft]);

  const validate = (): boolean => {
    const errs: Partial<Record<keyof RuleTabState, string>> = {};
    if (!state.pattern.trim()) errs.pattern = strings.proxyRules.patternRequired;
    if (state.useRegex) {
      try { new RegExp(state.pattern); } catch { errs.pattern = strings.proxyRules.invalidRegexPattern; }
    }
    if (state.targetType === "mapping" && !state.targetMappingId) {
      errs.targetMappingId = strings.proxyRules.selectTargetMapping;
    }
    if (state.targetType === "external" && !state.targetExternal.trim()) {
      errs.targetExternal = strings.proxyRules.enterHostPort;
    }
    setErrors(errs);
    return Object.keys(errs).length === 0;
  };

  const handleSave = useCallback(async () => {
    if (!validate()) return;
    setSaving(true);
    try {
      const res = await onSave({
        name: state.name,
        pattern: state.pattern.trim(),
        useRegex: state.useRegex,
        targetType: state.targetType,
        targetMappingId: state.targetMappingId,
        targetExternal: state.targetExternal.trim(),
        requestScript: state.requestScript,
        responseScript: state.responseScript,
        folderId: state.folderId,
      });
      markSaved();
      return res;
    } finally {
      setSaving(false);
    }
  }, [state, onSave, markSaved]);

  // Imperative refresh handle (used by useEntityTabs when reloading a saved entity)
  useImperativeHandle(ref, () => ({
    refresh(rule: ProxyRule) {
      setState(stateFromRule(rule));
    },
    save() {
      return handleSave();
    },
  }), [handleSave]);

  const [syncing, setSyncing] = useState(false);
  const [reverting, setReverting] = useState(false);

  const handleSyncClick = useCallback(async () => {
    if (!onSync || syncing) return;
    setSyncing(true);
    try {
      let targetId = tabId;
      if (isDirty || isDraft) {
        const saved: any = await handleSave();
        if (saved && typeof saved === "object" && saved.id) {
          targetId = saved.id;
        }
      }
      await onSync(targetId);
    } finally {
      setSyncing(false);
    }
  }, [onSync, syncing, isDirty, isDraft, handleSave, tabId]);

  const hasChanges = !isDraft && Boolean(isDirty || (syncStatus && syncStatus !== "clean"));

  const handleRevertClick = useCallback(async () => {
    if (!onRevert || reverting) return;
    setReverting(true);
    try {
      await onRevert();
    } finally {
      setReverting(false);
    }
  }, [onRevert, reverting]);

  const canSave = Boolean(state.pattern.trim());
  const syncDisabled = !hasChanges || (!canSave && isDirty) || syncing;
  const revertDisabled = !hasChanges || reverting;
  const syncTitle = !hasChanges ? strings.common.noChangesToSync : strings.common.syncTooltip;
  const revertTitle = !hasChanges ? strings.common.noChangesToRevert : strings.common.revertTooltip;

  const s = strings.proxyRules;

  return (
    <div className="flex flex-col flex-1 overflow-hidden h-full">
      <EditorTitleBar
        label={s.ruleLabel}
        namePlaceholder={s.ruleNamePlaceholder}
        name={state.name}
        onNameChange={(v) => set("name", v)}
        onClose={onClose}
        autoFocus={isDraft}
        enabled={enabled}
        onToggleEnabled={onToggleEnabled}
      />

      <div className="flex-1 overflow-y-auto p-4 flex flex-col gap-4 min-h-0">
        <ProxyRuleForm state={state} errors={errors} onChange={set} config={config} minimal={true} />
      </div>

      <BottomBar
        folders={folders}
        folderId={state.folderId}
        onFolderChange={(id) => set("folderId", id)}
        onCancel={onClose}
        onSave={handleSave}
        saveLabel={isDraft ? s.saveRule : s.updateRule}
        saving={saving}
        savingLabel={strings.server.saving}
        onSync={onSync ? handleSyncClick : undefined}
        onRevert={onRevert ? handleRevertClick : undefined}
        onHistory={onHistory}
        historyDisabled={!onHistory || isDraft}
        syncDisabled={syncDisabled}
        revertDisabled={revertDisabled}
        syncing={syncing}
        reverting={reverting}
        syncTitle={syncTitle}
        revertTitle={revertTitle}
      />
    </div>
  );
});
