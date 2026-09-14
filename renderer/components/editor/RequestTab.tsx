import React, { useRef } from "react";
import EnvVarHint from "@/components/editor/EnvVarHint";
import RandomizerHint from "@/components/editor/RandomizerHint";
import FolderPicker from "@/components/sidebar/FolderPicker";
import { Environment, Folder } from "@/types";
import { methodColor } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { RefreshCw, RotateCcw, Loader2, History } from "@/lib/icons";

// -- UrlBar -----------------------------------------------------------------
// Method dropdown + URL input + EnvVarHint + action button in one row.

export interface UrlBarProps {
  method: string;
  onMethodChange(m: string): void;
  url: string;
  onUrlChange(u: string): void;
  methods: string[];
  urlPlaceholder: string;
  /** Button label when idle */
  actionLabel: string;
  /** Button label when loading */
  actionLoadingLabel: string;
  actionLoading: boolean;
  actionDisabled: boolean;
  onAction(): void;
  /** Called on Enter key in the URL field */
  onEnter?(): void;
  activeEnv?: Environment | null;
  /** Show the {{random.*}} hint button (default true - set false for mock URLs which use pattern matching) */
  showRandomizer?: boolean;
  /** Extra node(s) rendered inside the input box after the URL input (e.g. regex toggle) */
  inputSuffix?: React.ReactNode;
  /** Extra node(s) rendered after the action button */
  afterButton?: React.ReactNode;
}

export function UrlBar({
  method, onMethodChange, url, onUrlChange, methods, urlPlaceholder,
  actionLabel, actionLoadingLabel, actionLoading, actionDisabled, onAction, onEnter,
  activeEnv, showRandomizer = true, inputSuffix, afterButton,
}: UrlBarProps) {
  const urlRef = useRef<HTMLInputElement>(null);

  return (
    <div className="px-4 py-2.5 border-b border-border flex-shrink-0 flex items-center gap-2">
      <div
        className="flex items-stretch rounded border border-border focus-within:border-signal transition-colors overflow-hidden flex-1"
        style={{ background: "var(--c-card)" }}
      >
        <select
          value={method}
          onChange={(e) => onMethodChange(e.target.value)}
          className="bg-surface-2 border-r border-border text-xs font-bold font-mono px-3 py-2.5 outline-none cursor-pointer appearance-none flex-shrink-0"
          style={{ color: methodColor(method), minWidth: 84 }}
        >
          {methods.map((m) => (
            <option key={m} value={m} style={{ color: methodColor(m), background: "var(--c-card)" }}>{m}</option>
          ))}
        </select>
        <input
          ref={urlRef}
          className="flex-1 bg-transparent px-3 py-2.5 text-sm font-mono text-foreground outline-none placeholder:text-muted-foreground min-w-0"
          placeholder={urlPlaceholder}
          value={url}
          onChange={(e) => onUrlChange(e.target.value)}
          onKeyDown={(e) => { if (e.key === "Enter") onEnter?.(); }}
        />
        {inputSuffix}
      </div>
      <EnvVarHint env={activeEnv ?? null} onInsert={(token) => {
        const el = urlRef.current;
        if (el) {
          const s = el.selectionStart ?? url.length;
          const e2 = el.selectionEnd ?? url.length;
          const next = url.slice(0, s) + token + url.slice(e2);
          onUrlChange(next);
          setTimeout(() => { el.setSelectionRange(s + token.length, s + token.length); el.focus(); }, 0);
        } else {
          onUrlChange(url + token);
        }
      }} />
      {showRandomizer && <RandomizerHint onInsert={(token) => {
        const el = urlRef.current;
        if (el) {
          const s = el.selectionStart ?? url.length;
          const e2 = el.selectionEnd ?? url.length;
          const next = url.slice(0, s) + token + url.slice(e2);
          onUrlChange(next);
          setTimeout(() => { el.setSelectionRange(s + token.length, s + token.length); el.focus(); }, 0);
        } else {
          onUrlChange(url + token);
        }
      }} />}
      <button
        onClick={onAction}
        disabled={actionLoading || actionDisabled}
        className="px-4 py-2.5 rounded bg-signal hover:bg-signal/80 disabled:opacity-40 disabled:cursor-not-allowed text-background text-xs font-semibold transition-all cursor-pointer flex-shrink-0 flex items-center gap-1.5"
      >
        {actionLoading
          ? <><span className="inline-block w-3 h-3 border-2 border-white/30 border-t-white rounded-full animate-spin" />{actionLoadingLabel}</>
          : actionLabel}
      </button>
      {afterButton}
    </div>
  );
}

// -- TabStrip ---------------------------------------------------------------
// Labelled tab row used inside both panels.

export interface TabStripProps<T extends string> {
  tabs: { id: T; label: string }[];
  active: T;
  onChange(t: T): void;
  /** Node rendered flush-left before the tabs (e.g. section label) */
  prefix?: React.ReactNode;
  /** Node rendered flush-right after the tabs */
  suffix?: React.ReactNode;
}

export function TabStrip<T extends string>({ tabs, active, onChange, prefix, suffix }: TabStripProps<T>) {
  return (
    <div className="flex items-center flex-shrink-0 border-b border-border bg-background/40">
      {prefix}
      {tabs.map((t) => (
        <button
          key={t.id}
          onClick={() => onChange(t.id)}
          className={`px-4 py-2.5 text-xs font-medium cursor-pointer transition-colors whitespace-nowrap ${
            active === t.id ? "text-signal border-b-2 border-signal -mb-px" : "text-muted-foreground hover:text-foreground"
          }`}
        >
          {t.label}
        </button>
      ))}
      {suffix && <div className="ml-auto flex items-center">{suffix}</div>}
    </div>
  );
}

// -- BottomBar --------------------------------------------------------------
// Shared footer: folder picker on the left, cancel + save on the right.

export interface BottomBarProps {
  folders?: Folder[];
  folderId: string | null;
  onFolderChange(id: string | null): void;
  onCancel(): void;
  onSave(): void;
  saveLabel: string;
  saveDisabled?: boolean;
  saving?: boolean;
  savingLabel?: string;
  /** Commit and push current state of entity */
  onSync?: () => void | Promise<void>;
  /** Revert local changes to last synced version */
  onRevert?: () => void | Promise<void>;
  /** View git commit history for this entity */
  onHistory?: () => void;
  syncDisabled?: boolean;
  revertDisabled?: boolean;
  historyDisabled?: boolean;
  syncing?: boolean;
  reverting?: boolean;
  syncTitle?: string;
  revertTitle?: string;
  historyTitle?: string;
  /** Extra nodes after the folder picker */
  extraLeft?: React.ReactNode;
  /** Action to create a mock from current request/response */
  onCreateMock?: () => void;
  createMockDisabled?: boolean;
}

export function BottomBar({
  folders = [], folderId, onFolderChange, onCancel, onSave,
  saveLabel, saveDisabled, saving, savingLabel,
  onSync, onRevert, onHistory, syncDisabled, revertDisabled, historyDisabled, syncing, reverting,
  syncTitle, revertTitle, historyTitle, extraLeft,
  onCreateMock, createMockDisabled = true,
}: BottomBarProps) {
  return (
    <div className="flex items-center justify-between px-4 py-2.5 border-t border-border flex-shrink-0 bg-background/30">
      <div className="flex items-center gap-2">
        {folders.length > 0 && (
          <FolderPicker folders={folders} value={folderId} onChange={onFolderChange} />
        )}
        {onSync && (
          <button
            type="button"
            onClick={onSync}
            disabled={syncDisabled || syncing}
            title={syncTitle ?? strings.common.syncTooltip}
            className="px-2.5 py-1.5 rounded border border-border bg-card hover:bg-surface-2 disabled:opacity-40 disabled:cursor-not-allowed text-foreground text-xs font-medium transition-all cursor-pointer flex items-center gap-1.5"
          >
            {syncing ? (
              <Loader2 size={12} className="animate-spin text-signal flex-shrink-0" />
            ) : (
              <RefreshCw size={12} className="text-signal flex-shrink-0" />
            )}
            <span>{syncing ? strings.common.syncing : strings.common.sync}</span>
          </button>
        )}
        {onRevert && (
          <button
            type="button"
            onClick={onRevert}
            disabled={revertDisabled || reverting}
            title={revertTitle ?? strings.common.revertTooltip}
            className="px-2.5 py-1.5 rounded border border-border bg-card hover:bg-surface-2 disabled:opacity-40 disabled:cursor-not-allowed text-foreground text-xs font-medium transition-all cursor-pointer flex items-center gap-1.5"
          >
            {reverting ? (
              <Loader2 size={12} className="animate-spin text-amber flex-shrink-0" />
            ) : (
              <RotateCcw size={12} className="text-amber flex-shrink-0" />
            )}
            <span>{reverting ? strings.common.reverting : strings.common.revert}</span>
          </button>
        )}
        {onHistory && (
          <button
            type="button"
            onClick={onHistory}
            disabled={historyDisabled}
            title={historyTitle ?? strings.folderTree.history}
            className="px-2.5 py-1.5 rounded border border-border bg-card hover:bg-surface-2 disabled:opacity-40 disabled:cursor-not-allowed text-foreground text-xs font-medium transition-all cursor-pointer flex items-center gap-1.5"
          >
            <History size={12} className="text-muted-foreground flex-shrink-0" />
            <span>{strings.folderTree.history}</span>
          </button>
        )}
        {extraLeft}
      </div>
      <div className="flex items-center gap-2">
        {onCreateMock && (
          <button
            type="button"
            onClick={onCreateMock}
            disabled={createMockDisabled}
            title={strings.requests.createMockTitle}
            className="px-3 py-1.5 rounded border border-amber/40 bg-amber/10 hover:bg-amber/20 disabled:opacity-40 disabled:cursor-not-allowed text-amber text-xs font-medium transition-all cursor-pointer flex items-center gap-1.5"
          >
            <span>{strings.requests.createMock}</span>
          </button>
        )}
        <button
          onClick={onCancel}
          className="px-3 py-1.5 rounded border border-border bg-card hover:bg-surface-2 text-muted-foreground text-xs font-medium transition-all cursor-pointer"
        >
          {strings.common.cancel}
        </button>
        <button
          onClick={onSave}
          disabled={saveDisabled || saving}
          className="px-4 py-1.5 rounded bg-signal hover:bg-signal/80 disabled:opacity-40 disabled:cursor-not-allowed text-background text-xs font-semibold transition-all cursor-pointer flex items-center gap-1.5"
        >
          {saving
            ? <><span className="inline-block w-3 h-3 border-2 border-white/30 border-t-white rounded-full animate-spin" />{savingLabel ?? saveLabel}</>
            : saveLabel}
        </button>
      </div>
    </div>
  );
}
