import React, { useRef } from "react";
import FolderPicker from "@/components/sidebar/FolderPicker";
import { Folder } from "@/types";
import { methodColor } from "@/lib/utils";
import { strings } from "@/lib/strings";
import { RefreshCw, RotateCcw, Loader2, History, CheckCircle2, AlertCircle } from "@/lib/icons";

// -- UrlBar -----------------------------------------------------------------
// Method (or fixed verb) + URL input + action button in one row.
//
// **This is the shared address row for every protocol**, not just REST. GraphQL and SOAP pass a
// `leading` verb badge instead of a method dropdown; gRPC passes `leading` for the address and its
// service/method pair through `beforeAction`, so the whole target stays on one line. Before that,
// three of the four protocols had hand-rolled their own version of this row and had drifted —
// different paddings, different button sizes, one of them stacked three bars deep.

export interface UrlBarProps {
  /**
   * Rendered in place of the method dropdown — a fixed verb badge (`POST`, `gRPC`).
   *
   * Mutually exclusive with `method`/`methods` in practice: a protocol whose verb is not a choice
   * should not render a one-option `<select>`, which reads as a broken dropdown.
   */
  leading?: React.ReactNode;
  /** Omit when `leading` is given. */
  method?: string;
  onMethodChange?(m: string): void;
  methods?: string[];
  url: string;
  onUrlChange(u: string): void;
  urlPlaceholder: string;
  /**
   * Drop the bar's own padding and bottom border, and **stretch to fill the row**.
   *
   * Set when the bar sits inside a `TabToolbarRow`, which owns the band's frame — otherwise the page
   * gets the padding twice and two stacked borders, which is exactly how the SOAP tab ended up
   * looking like it had an extra divider under its address row.
   *
   * ⚠ `bare` also means "this is a flex item in a **row**", so it carries `flex-1 min-w-0`. Dropping
   * the padding without the width was a real bug: the bar collapsed to its content width and left
   * dead space to the right of the Send button, which is what the first revision of this refactor
   * shipped. Do not use `bare` as a child of a column flex container — `flex-1` would then grow
   * *height*.
   */
  bare?: boolean;
  /** Button label when idle */
  actionLabel: string;
  /** Button label when loading */
  actionLoadingLabel: string;
  actionLoading: boolean;
  actionDisabled: boolean;
  onAction(): void;
  /** Called on Enter key in the URL field */
  onEnter?(): void;
  /** Extra node(s) rendered inside the input box after the URL input (e.g. regex toggle) */
  inputSuffix?: React.ReactNode;
  /**
   * Extra controls rendered **after the address group and before the action button**.
   *
   * gRPC uses it for its service/method pair, so "what am I calling" reads as one line ending in
   * Send. It has to be *before* the button: anything placed after it lands on the far side of Send,
   * which leaves the primary action stranded in the middle of the row.
   */
  beforeAction?: React.ReactNode;
  /** Extra node(s) rendered after the action button */
  afterButton?: React.ReactNode;
}

/**
 * A fixed protocol verb, styled to occupy the same footprint as the method dropdown.
 *
 * `minWidth: 84` is copied from the `<select>` below on purpose: the four protocols' URL inputs
 * start at the same x only because the two variants are the same width. Change one, change both.
 */
export function UrlBarVerb({ verb }: { verb: string }) {
  return (
    <span
      className="bg-surface-2 border-r border-border text-xs font-bold font-mono px-3 py-2.5 flex-shrink-0 flex items-center"
      style={{ color: methodColor(verb), minWidth: 84 }}
    >
      {verb}
    </span>
  );
}

export function UrlBar({
  leading, method, onMethodChange, methods, url, onUrlChange, urlPlaceholder, bare = false,
  actionLabel, actionLoadingLabel, actionLoading, actionDisabled, onAction, onEnter,
  inputSuffix, beforeAction, afterButton,
}: UrlBarProps) {
  const urlRef = useRef<HTMLInputElement>(null);

  return (
    <div
      className={
        bare
          // `flex-1 min-w-0` is what makes the bar fill the toolbar row. Without it the bar is only
          // as wide as its contents and the row keeps the remainder as dead space.
          ? "flex flex-1 min-w-0 items-center gap-2"
          : "px-4 py-2.5 border-b border-border flex-shrink-0 flex items-center gap-2"
      }
    >
      <div
        className="flex items-stretch rounded border border-border focus-within:border-signal transition-colors overflow-hidden flex-1 min-w-0"
        style={{ background: "var(--c-card)" }}
      >
        {leading ?? (
          <select
            value={method}
            onChange={(e) => onMethodChange?.(e.target.value)}
            className="bg-surface-2 border-r border-border text-xs font-bold font-mono px-3 py-2.5 outline-none cursor-pointer appearance-none flex-shrink-0"
            style={{ color: methodColor(method ?? ""), minWidth: 84 }}
          >
            {(methods ?? []).map((m) => (
              <option key={m} value={m} style={{ color: methodColor(m), background: "var(--c-card)" }}>{m}</option>
            ))}
          </select>
        )}
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
      {beforeAction}
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
// Shared footer: the folder picker and this entity's git actions on the left, autosave status
// and the Create button on the right.
//
// **There is no Save button.** An existing entity is written by `useDebouncedAutosave` one
// second after the user stops editing. The only button here that writes is Create, and it
// exists because a draft has no server id yet — creation is deliberate, saving is not.
// Sync / Revert / History stay deliberate.

export interface BottomBarProps {
  folders?: Folder[];
  folderId: string | null;
  onFolderChange(id: string | null): void;
  /** Draft only — discards it. An existing entity has Revert instead. */
  onCancel?: () => void;
  /** Draft only — creates the entity. */
  onSave?: () => void;
  saveLabel?: string;
  saveDisabled?: boolean;
  saving?: boolean;
  savingLabel?: string;
  /** Autosave state of an existing entity — rendered in place of the old Save button. */
  autoSaving?: boolean;
  autoSavedAt?: number | null;
  autoSaveError?: string | null;
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
  autoSaving, autoSavedAt, autoSaveError,
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
        <AutosaveStatus saving={autoSaving} savedAt={autoSavedAt} error={autoSaveError} />
        {onCancel && (
          <button
            onClick={onCancel}
            className="px-3 py-1.5 rounded text-muted-foreground/70 hover:text-foreground hover:bg-surface-2/60 text-xs font-medium transition-all cursor-pointer"
          >
            {strings.common.cancel}
          </button>
        )}
        {onSave && saveLabel && (
          <button
            onClick={onSave}
            disabled={saveDisabled || saving}
            className="px-4 py-1.5 rounded bg-signal hover:bg-signal/80 disabled:opacity-40 disabled:cursor-not-allowed text-background text-xs font-semibold transition-all cursor-pointer flex items-center gap-1.5"
          >
            {saving
              ? <><span className="inline-block w-3 h-3 border-2 border-white/30 border-t-white rounded-full animate-spin" />{savingLabel ?? saveLabel}</>
              : saveLabel}
          </button>
        )}
      </div>
    </div>
  );
}

/**
 * Quiet proof that the editor is not the only copy of the user's work.
 *
 * The Save button used to be that proof, so removing it without a replacement would leave the
 * user guessing whether a keystroke survived. Deliberately text-only until something is worth
 * saying: an idle "All changes saved" on every tab would be noise, not reassurance.
 */
function AutosaveStatus({
  saving, savedAt, error,
}: {
  saving?: boolean;
  savedAt?: number | null;
  error?: string | null;
}) {
  if (error) {
    return (
      <span title={error} className="flex items-center gap-1.5 text-[11px] font-medium text-destructive">
        <AlertCircle size={12} className="flex-shrink-0" />
        <span>{strings.common.saveFailed}</span>
      </span>
    );
  }
  if (saving) {
    return (
      <span className="flex items-center gap-1.5 text-[11px] text-muted-foreground">
        <Loader2 size={12} className="animate-spin flex-shrink-0" />
        <span>{strings.common.saving}</span>
      </span>
    );
  }
  if (!savedAt) return null;
  return (
    <span className="flex items-center gap-1.5 text-[11px] text-muted-foreground/70">
      <CheckCircle2 size={12} className="flex-shrink-0" />
      <span>{strings.common.saved}</span>
    </span>
  );
}
