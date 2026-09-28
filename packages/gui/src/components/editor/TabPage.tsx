/**
 * The shared wireframe for every editor tab page.
 *
 * ## The problem this file exists to remove
 *
 * The four request tabs were each written at a different time and each grew its own header. Measured
 * before this file existed:
 *
 * | tab | title bar | address row | split |
 * |---|---|---|---|
 * | REST request | **none at all** | `UrlBar` (shared) | persisted, with a toggle |
 * | GraphQL | `EditorTitleBar` | hand-rolled `POST` badge + input | **hard-coded `horizontal`** |
 * | gRPC | `EditorTitleBar` | 3 inputs in one row **+ a second bar** below | **hard-coded `horizontal`** |
 * | SOAP | `EditorTitleBar` | **two rows** | **hard-coded `horizontal`** |
 * | Collection runner | **none at all** | n/a | hard-coded `w-72` fixed column |
 *
 * So the same tab showed its status code in four different places — a TabStrip suffix (REST), a
 * bespoke header `div` (GraphQL), a status bar *above* the TabStrip (gRPC) and a TabStrip `prefix`
 * (SOAP) — and only one of the five pages could be flipped between side-by-side and stacked.
 *
 * ## The contract
 *
 * Every tab page is exactly this, top to bottom:
 *
 * ```
 * TabTitleBar     chip · name · actions            (px-4 py-2.5, border-b)
 * TabToolbar      row 1 = protocol/verb · target · primary action
 *                 row 2 = optional extras (SOAPAction, gRPC service/method…)
 * SplitPane       request pane | response pane     (orientation toggle in the response TabStrip)
 * BottomBar       folder · sync · revert · history | autosave · cancel · create
 * ```
 *
 * The runner follows the same four bands; it has no BottomBar because a run is not a saved entity.
 *
 * ## What this file deliberately does NOT own
 *
 * The panes themselves. `RequestPane` / `ResponsePane` / `GrpcLeftPane` / `GrpcResponsePane` keep
 * their own contents — the tabs inside them genuinely differ per protocol, and flattening those would
 * be a functional change rather than a layout one. This file only fixes where the *bands* sit.
 *
 * ## ⚠ `packages/gui` has no `dist`
 *
 * `npm run build:packages` omits it, so a green `build:packages` says nothing about this file. Run
 * `CODEBUDDY_SAFE_DELETE_ENABLED=0 npm run build:renderer`. And `packages/gui` is not in
 * `npm run typecheck` — the sole runner is `npm run typecheck --workspace @bifurc/gui`, which has a
 * standing set of errors, so the instrument is *which files*, not the count.
 */
import React, { useCallback, useEffect } from "react";
import { Group as PanelGroup, Panel, Separator as ResizeHandle } from "react-resizable-panels";
import { usePersistedState } from "@/hooks/usePersistedState";
import { strings } from "@/lib/strings";
import { statusColor } from "@/lib/utils";
import { Columns2, Rows2 } from "@/lib/icons";
import { cn } from "@/components/ui/cn";

// -- Split orientation ------------------------------------------------------

/** Orientation of the request/response split pane. */
export type SplitOrientation = "horizontal" | "vertical";

const SPLIT_KEY = "bifurc:editor-split-orientation";
const SPLIT_EVENT = "bifurc:split-orientation";

/**
 * The persisted split orientation, plus the toggle.
 *
 * ## Why this is a hook rather than local state in one page
 *
 * The preference is stored under **one global key**, so the intent has always been "the user's
 * layout", not "this tab's layout". `usePersistedState` alone cannot express that: it reads
 * `localStorage` once at mount and never listens, and every open tab stays mounted
 * (`RequestsPanel` hides inactive tabs with `display: none`, it does not unmount them). So before
 * this hook, toggling in tab A wrote the key and left tab B — already mounted, already read —
 * showing the old orientation until the app restarted. With one toggle on one tab that was
 * invisible; with a toggle on all five pages it is the first thing a user would hit.
 *
 * The broadcast is a `CustomEvent` on `window`, matching the existing `bifurc:open-tab` idiom in
 * `useEntityTabs.ts`. The listener sets state to a value it already holds in the originating tab,
 * where React bails out of the re-render, so the echo is free.
 */
export function useSplitOrientation(): [SplitOrientation, () => void] {
  const [orientation, setOrientation] = usePersistedState<SplitOrientation>(
    SPLIT_KEY,
    "horizontal",
    (v) => (v === "vertical" ? "vertical" : "horizontal"),
  );

  useEffect(() => {
    const handler = (e: Event) => {
      const next = (e as CustomEvent<SplitOrientation>).detail;
      setOrientation(next === "vertical" ? "vertical" : "horizontal");
    };
    window.addEventListener(SPLIT_EVENT, handler);
    return () => window.removeEventListener(SPLIT_EVENT, handler);
  }, [setOrientation]);

  // Deliberately reads `orientation` from the closure rather than using the functional updater: the
  // updater must stay pure, and dispatching from inside it would fire twice under StrictMode.
  const toggle = useCallback(() => {
    const next: SplitOrientation = orientation === "horizontal" ? "vertical" : "horizontal";
    setOrientation(next);
    window.dispatchEvent(new CustomEvent<SplitOrientation>(SPLIT_EVENT, { detail: next }));
  }, [orientation, setOrientation]);

  return [orientation, toggle];
}

/**
 * Flip between side-by-side and stacked.
 *
 * The icon is the layout you would get **by pressing it**, not the current one — that is the
 * existing behaviour from `ResponsePane` and it is preserved here rather than re-decided.
 */
export function SplitOrientationToggle({
  orientation,
  onToggle,
}: {
  orientation: SplitOrientation;
  onToggle: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      title={strings.editor.splitLayoutToggle}
      aria-label={strings.editor.splitLayoutToggle}
      data-testid="split-orientation-toggle"
      className="p-1.5 rounded border transition-colors cursor-pointer flex-shrink-0 border-border text-muted-foreground hover:text-foreground hover:bg-surface-2"
    >
      {orientation === "horizontal" ? <Rows2 size={14} /> : <Columns2 size={14} />}
    </button>
  );
}

/**
 * The request/response split, in one place.
 *
 * `key={orientation}` is load-bearing: `react-resizable-panels` reads `defaultSize` only on mount, so
 * without the key a flip would swap the axis while keeping the old percentage split — which for a
 * 40/60 side-by-side read as a 40/60 stacked split, i.e. a request pane a third of the height.
 * Remounting restores the intended 50/50.
 *
 * Sizes are 50/50 for every protocol. They used to differ (REST 40/60, the other three 50/50) with
 * no recorded reason; the split is draggable and persisted, so the default is only a starting point.
 */
export function SplitPane({
  orientation,
  left,
  right,
  leftSize,
  rightSize,
  minSize = 20,
  className,
}: {
  orientation: SplitOrientation;
  left: React.ReactNode;
  right: React.ReactNode;
  /**
   * Default panel sizes, as percentages. Both default to 50.
   *
   * The runner is the only caller that overrides them, at 30/70: its left pane is a navigation list
   * of request names, not an editor, and an even split would spend half the width on a column that
   * never needs it. The four request tabs stay at 50/50.
   */
  leftSize?: number;
  rightSize?: number;
  minSize?: number;
  className?: string;
}) {
  const isHorizontal = orientation === "horizontal";
  return (
    <PanelGroup
      key={orientation}
      orientation={orientation}
      className={cn("flex flex-1 min-h-0 overflow-hidden", className)}
    >
      <Panel defaultSize={leftSize ?? 50} minSize={minSize} className="flex flex-col overflow-hidden">
        {left}
      </Panel>
      <ResizeHandle
        className={
          isHorizontal
            ? "w-1 bg-border hover:bg-signal/40 active:bg-signal/60 transition-colors cursor-col-resize flex-shrink-0"
            : "h-1 w-full bg-border hover:bg-signal/40 active:bg-signal/60 transition-colors cursor-row-resize flex-shrink-0"
        }
      />
      <Panel defaultSize={rightSize ?? 50} minSize={minSize} className="flex flex-col overflow-hidden">
        {right}
      </Panel>
    </PanelGroup>
  );
}

// -- Band 1: the title bar --------------------------------------------------

/** The protocol chip's colours. Same values as `ProtocolSelectorTab`'s cards, so the chooser, the
 * title bar and (for REST) the method badge all agree about what each protocol looks like. */
export interface ProtocolTone {
  color: string;
  bg: string;
}

export const PROTOCOL_TONE: Record<"rest" | "graphql" | "grpc" | "soap" | "runner", ProtocolTone> = {
  rest: { color: "var(--c-signal)", bg: "oklch(var(--signal) / 0.15)" },
  graphql: { color: "#d946ef", bg: "rgba(217, 70, 239, 0.15)" },
  grpc: { color: "#06b6d4", bg: "rgba(6, 182, 212, 0.15)" },
  soap: { color: "#14b8a6", bg: "rgba(20, 184, 166, 0.15)" },
  runner: { color: "var(--c-amber)", bg: "oklch(var(--amber) / 0.15)" },
};

export interface TabTitleBarProps {
  /** The protocol chip's text — `REST`, `GQL`, `gRPC`, `SOAP`, `RUNNER`. */
  chip: string;
  /** The chip's colours. Omit for a plain muted label (the `EditorTitleBar` look). */
  tone?: ProtocolTone;
  name: string;
  namePlaceholder?: string;
  onNameChange?(v: string): void;
  /** Render the name as text. The runner has no editable name — its identity is the folder. */
  readOnly?: boolean;
  autoFocus?: boolean;
  /** Right-aligned controls. */
  actions?: React.ReactNode;
}

/**
 * Band 1 — identical on every tab page.
 *
 * ⚠ The input's classes are copied verbatim from `EditorTitleBar` on purpose. The Mocks panel still
 * uses `EditorTitleBar`, and a request tab and a mock tab are one keystroke apart in the same
 * sidebar — if the two title bars were a pixel different it would read as a bug rather than a
 * design. If you restyle one, restyle both.
 *
 * ⚠ The REST request tab had **no** title bar before this; the name was only editable by
 * double-clicking the tab. Rendering the field does not add state — `restTabReducer` has carried
 * `name` since the beginning, `stateToSavePayload` already writes it, and the tab's own rename
 * handler dispatches `SET_FIELD { field: "name" }` to the very same place. This is the field the
 * tab strip was editing all along, made visible.
 */
export function TabTitleBar({
  chip,
  tone,
  name,
  namePlaceholder,
  onNameChange,
  readOnly = false,
  autoFocus,
  actions,
}: TabTitleBarProps) {
  return (
    <div className="flex items-center gap-3 px-4 py-2.5 border-b border-border flex-shrink-0">
      <span
        className={cn(
          "flex-shrink-0 font-mono whitespace-nowrap",
          tone
            ? "text-[10px] font-bold px-2 py-0.5 rounded"
            : "text-[10px] font-semibold uppercase tracking-widest text-muted-foreground",
        )}
        style={tone ? { color: tone.color, background: tone.bg } : undefined}
      >
        {chip}
      </span>

      {readOnly ? (
        <span className="flex-1 min-w-0 truncate text-sm text-foreground px-3 py-1.5">{name}</span>
      ) : (
        <input
          className="flex-1 min-w-0 bg-card border border-border focus:border-signal rounded px-3 py-1.5 text-sm text-foreground outline-none placeholder:text-muted-foreground transition-colors"
          placeholder={namePlaceholder}
          value={name}
          onChange={(e) => onNameChange?.(e.target.value)}
          autoFocus={autoFocus}
        />
      )}

      {actions}
    </div>
  );
}

// -- Band 2: the toolbar ----------------------------------------------------

/**
 * Band 2's frame — the address/connection band.
 *
 * Rendered even when a protocol has nothing to put in it, so the band below the title bar is always
 * the same height and the split pane always starts at the same y. The rows inside are
 * `TabToolbarRow`, which is where the per-protocol difference is allowed to live.
 */
export function TabToolbar({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div
      className={cn(
        "px-4 py-2.5 border-b border-border flex-shrink-0 flex flex-col gap-2",
        className,
      )}
    >
      {children}
    </div>
  );
}

/** One row inside a `TabToolbar`. */
export function TabToolbarRow({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return <div className={cn("flex items-center gap-2", className)}>{children}</div>;
}

// -- Band 3 helpers: the response strip -------------------------------------

/**
 * Status + duration, for a response TabStrip's `suffix`.
 *
 * One rendering of "what came back", used by all four protocols. It replaced four different
 * treatments — a suffix (REST), a bespoke header row (GraphQL), a bar *above* the TabStrip (gRPC)
 * and a `prefix` (SOAP) — which between them put the status code in four different places and at
 * three different font sizes.
 *
 * Deliberately renders `null` rather than a placeholder when there is no status yet: an empty
 * response should not reserve space for a number that does not exist.
 */
export function ResponseStatus({
  status,
  durationMs,
}: {
  status?: number | null;
  durationMs?: number | null;
}) {
  if (status === null || status === undefined) return null;
  return (
    <>
      <span className="text-sm font-bold font-mono" style={{ color: statusColor(status) }}>
        {status}
      </span>
      {durationMs != null && (
        <span className="text-xs font-mono text-muted-foreground">{durationMs}ms</span>
      )}
    </>
  );
}

/** The right-hand cluster of a response TabStrip: the layout toggle, then the status. */
export function ResponseStripSuffix({
  orientation,
  onToggleOrientation,
  status,
  durationMs,
  leading,
}: {
  orientation?: SplitOrientation;
  onToggleOrientation?: () => void;
  status?: number | null;
  durationMs?: number | null;
  /** Extra controls between the toggle and the status. */
  leading?: React.ReactNode;
}) {
  return (
    <div className="flex items-center gap-2 pr-3">
      {orientation && onToggleOrientation && (
        <SplitOrientationToggle orientation={orientation} onToggle={onToggleOrientation} />
      )}
      {leading}
      <ResponseStatus status={status} durationMs={durationMs} />
    </div>
  );
}

// -- The page itself --------------------------------------------------------

/**
 * The outer shell of a tab page.
 *
 * `bg-surface` is on every page now. It used to be REST-only, which meant switching between a REST
 * tab and a GraphQL tab changed the page's own background — the most visible of the "these do not
 * belong together" tells.
 */
export function TabPage({
  children,
  className,
}: {
  children: React.ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex flex-col h-full overflow-hidden bg-surface", className)}>{children}</div>
  );
}
