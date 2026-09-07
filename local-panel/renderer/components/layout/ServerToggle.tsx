import React, { useState } from "react";
import { strings } from "@/lib/strings";
import { Play, Square } from "@/lib/icons";

interface Props {
  running: boolean;
  error: string | null;
  onStart: () => Promise<void>;
  onStop: () => Promise<void>;
}

export default function ServerToggle({ running, error, onStart, onStop }: Props) {
  const [busy, setBusy] = useState(false);

  const handle = async () => {
    setBusy(true);
    try { running ? await onStop() : await onStart(); }
    finally { setBusy(false); }
  };

  const hasError = !!error;

  return (
    <button
      type="button"
      onClick={handle}
      disabled={busy}
      aria-label={running ? strings.titleBar.stopServer : strings.titleBar.startServer}
      style={{ WebkitAppRegion: "no-drag" } as React.CSSProperties}
      title={running ? strings.titleBar.stopServer : strings.titleBar.startServer}
      className={`flex items-center justify-center w-8 h-8 rounded-md border transition-all duration-150 cursor-pointer disabled:opacity-40 disabled:cursor-not-allowed flex-shrink-0 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-signal/35 focus-visible:ring-offset-2 focus-visible:ring-offset-surface ${
        running
          ? "border-signal/40 bg-signal/10 hover:bg-destructive/15 hover:border-destructive/40 text-signal hover:text-destructive"
          : hasError
          ? "border-destructive/40 bg-destructive/10 hover:bg-signal/15 hover:border-signal/40 text-destructive hover:text-signal"
          : "border-border bg-card hover:bg-signal/15 hover:border-signal/40 text-muted-foreground hover:text-signal"
      }`}
    >
      {busy ? (
        <span className="inline-block w-3 h-3 border-2 border-current/30 border-t-current rounded-full animate-spin" />
      ) : running ? (
        <Square size={10} fill="currentColor" />
      ) : (
        <Play size={10} fill="currentColor" />
      )}
    </button>
  );
}
