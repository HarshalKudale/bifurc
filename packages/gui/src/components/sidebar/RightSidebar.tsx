import React from "react";
import { Environment } from "@/types";
import HistorySidebar from "@/components/sidebar/HistorySidebar";
import VariablesSidebar from "@/components/sidebar/VariablesSidebar";

/**
 * The one right-hand sidebar, and the **only** thing that decides how wide it is.
 *
 * ## Why a shell rather than a second sidebar
 *
 * The panel opened for one reason — the git history of a single entity — and that is now one of
 * several things it can show. Two independently-positioned sidebars would both claim the same 280px
 * and the same left border, and the user would have to reason about which one was on top. So the
 * frame (width, border, transition) lives here, exactly once, and the faces are swappable content:
 *
 * - `"history"` — `HistorySidebar`, the audit trail of the entity at `filePath`.
 * - `"variables"` — `VariablesSidebar`, everything the active environment can resolve.
 *
 * ## The mode is the open state
 *
 * `null` is closed. That is what makes the toggle semantics fall out for free: clicking the control
 * for the face that is *already* showing clears the mode (close), and clicking the other one
 * replaces it (swap). No separate `open` boolean to keep consistent with the mode.
 *
 * ## Why the closed shell still renders
 *
 * A closed sidebar is a zero-width element rather than an unmounted one, because `.right-sidebar`
 * animates its width — unmounting would make it vanish instead of slide. `overflow: hidden` (in
 * `styles.css`) is what keeps the fixed-width child from painting while collapsed.
 */
export type RightSidebarMode = "history" | "variables";

interface Props {
  /** Which face to show, or `null` for closed. */
  mode: RightSidebarMode | null;
  /** The entity whose history is shown. Only read in `"history"` mode. */
  filePath: string;
  workspaceId: string;
  onClose(): void;
  /** Increment to trigger a history reload (e.g. after a save). */
  reloadKey?: number;
  activeEnvironment: Environment | null;
  globalEnvironment: Environment | null;
}

export default function RightSidebar({
  mode, filePath, workspaceId, onClose, reloadKey, activeEnvironment, globalEnvironment,
}: Props) {
  const open = mode !== null;

  return (
    <div className="right-sidebar" style={{ width: open ? 280 : 0, opacity: open ? 1 : 0 }}>
      <div className="flex flex-col h-full bg-surface border-l border-border" style={{ width: 280, minWidth: 280 }}>
        {mode === "history" && (
          <HistorySidebar
            filePath={filePath}
            workspaceId={workspaceId}
            onClose={onClose}
            reloadKey={reloadKey}
          />
        )}
        {mode === "variables" && (
          <VariablesSidebar
            activeEnvironment={activeEnvironment}
            globalEnvironment={globalEnvironment}
            onClose={onClose}
          />
        )}
      </div>
    </div>
  );
}
