/**
 * The web host's outer container — the connection screen, and the chrome around the shared UI.
 *
 * ## What this is, and why it exists
 *
 * `web/` is a **container** for `packages/gui`, not a mount point. It shows a connection form first, and
 * only renders `<App />` once the handshake has succeeded. That ordering is not a preference: the
 * browser has to authenticate before its first real request (`connect.ts` explains why), and `<App />`
 * reads `window.api` during mount. Rendering first and connecting after is exactly the order that
 * produced *"the first request must be hello"* and a UI that died on arrival.
 *
 * ## Why the connection settings are handed to `<App />`
 *
 * Because the UI is the same UI in both hosts, so anything that *differs* has to be told to it rather
 * than discovered by it. `<App host="web" connection={{ url }} />` is that channel — see `AppProps` in
 * `packages/gui/src/App.tsx`. It is deliberately a **prop and not a second global**: `window.api` is the
 * communication seam, and D18's whole premise is that it is the *only* thing that varies. A UI that read
 * a second injected global would have two seams to keep honest.
 *
 * ## The two things this file must not do
 *
 * It must not leave a **stale screen** when the host dies: `onFatal` tears the session down and returns
 * to the form, because a page with no process boundary has nothing else to notice that the engine is
 * gone — `plan/08`'s "the UI never silently freezes" criterion.
 *
 * And it must not show a **generic** error. `plan/08`'s criterion is that *"handshake failure, auth
 * failure and version mismatch all produce actionable UI, not generic errors"*, which is why the failure
 * kind survives as data from `connect.ts` all the way to the heading below.
 */
import { useCallback, useState, type FormEvent } from "react";
import { App } from "@bifurc/gui";

import { connectAndInstall, uninstallWindowApi, type InstalledApi } from "./api";
import { ConnectError, type ConnectFailure } from "./connect";
import { fromUrl, loadSettings, saveSettings, toUrl, type HostSettings } from "./settings";

type Phase =
  | { kind: "form"; error: ConnectError | null }
  | { kind: "connecting" }
  | { kind: "connected"; session: InstalledApi; settings: HostSettings };

/**
 * A scannable heading per failure, so the user reads *what kind* of problem it is before the sentence.
 *
 * The four are genuinely different instructions: check the address, find the token, upgrade one side, or
 * read the engine's own reason.
 */
const FAILURE_TITLE: Record<ConnectFailure, string> = {
  unreachable: "Could not reach a host",
  unauthorized: "The host rejected the token",
  version: "Protocol version mismatch",
  refused: "The host refused the connection",
};

/**
 * Remembered settings, with `?engine=` layered over them.
 *
 * The query wins because it was asked for explicitly — that is what makes one built bundle point at a
 * local, a Docker-hosted or a remote host without a rebuild, which is `plan/08`'s "connects to a local
 * engine and to a remote engine" criterion.
 */
function initialSettings(): HostSettings {
  const remembered = loadSettings();
  const query = new URLSearchParams(location.search).get("engine");
  const parsed = query === null ? undefined : fromUrl(query);
  return parsed === undefined ? remembered : { ...remembered, ...parsed };
}

function asConnectError(error: unknown): ConnectError {
  return error instanceof ConnectError
    ? error
    : new ConnectError("refused", error instanceof Error ? error.message : String(error), error);
}

export function Container() {
  const [phase, setPhase] = useState<Phase>({ kind: "form", error: null });
  const [settings, setSettings] = useState<HostSettings>(initialSettings);

  const handleConnect = useCallback(async (next: HostSettings) => {
    setPhase({ kind: "connecting" });
    try {
      const session = await connectAndInstall({
        url: toUrl(next),
        // Empty means "this endpoint has no auth boundary" rather than "an empty token" — see
        // `connect.ts`. A blank field is the honest way to express that.
        token: next.token === "" ? undefined : next.token,
        clientVersion: __APP_VERSION__,
        onFatal: (error) => {
          // Tear the global down before unmounting: leaving a `window.api` whose transport is closed
          // means the next render calls a client that rejects everything.
          uninstallWindowApi();
          setPhase({
            kind: "form",
            error: new ConnectError("unreachable", `Lost the host: ${error.message}`, error),
          });
        },
      });
      saveSettings(next);
      setSettings(next);
      setPhase({ kind: "connected", session, settings: next });
    } catch (error) {
      setPhase({ kind: "form", error: asConnectError(error) });
    }
  }, []);

  const handleDisconnect = useCallback(async () => {
    if (phase.kind !== "connected") return;
    // Order matters: stop new calls, unmount the UI that makes them, then close the transport.
    uninstallWindowApi();
    setPhase({ kind: "form", error: null });
    await phase.session.close();
  }, [phase]);

  if (phase.kind === "connected") {
    const url = toUrl(phase.settings);
    return (
      <div className="flex flex-col h-screen bg-background text-foreground overflow-hidden">
        <div className="flex items-center gap-3 border-b border-border bg-surface px-3 py-1.5 text-xs flex-shrink-0">
          <span className="h-2 w-2 rounded-full bg-signal flex-shrink-0" aria-hidden="true" />
          <span className="text-muted-foreground">Connected to</span>
          <span className="font-mono text-foreground">{url}</span>
          <span className="flex-1" />
          <button
            type="button"
            onClick={() => void handleDisconnect()}
            className="rounded-sm border border-border px-2 py-0.5 text-muted-foreground hover:text-foreground"
          >
            Disconnect
          </button>
        </div>
        {/*
          `min-h-0` so the flex child may actually shrink — without it the UI's own `h-full` resolves
          against a content-sized box and the panel pushes the bar off-screen.
        */}
        <div className="flex-1 min-h-0">
          <App host="web" connection={{ url }} />
        </div>
      </div>
    );
  }

  const connecting = phase.kind === "connecting";
  const error = phase.kind === "form" ? phase.error : null;

  return (
    <div className="flex h-screen items-center justify-center bg-background text-foreground p-6">
      <form
        className="w-full max-w-md rounded-lg border border-border bg-card p-5 flex flex-col gap-4"
        onSubmit={(event: FormEvent) => {
          event.preventDefault();
          void handleConnect(settings);
        }}
      >
        <div className="flex flex-col gap-1">
          <h1 className="text-sm font-medium">Connect to a workspace host</h1>
          <p className="text-xs text-muted-foreground">
            The host prints its RPC endpoint on startup. The token is in{" "}
            <code className="font-mono">engine.token</code> inside the host&rsquo;s data directory.
          </p>
        </div>

        <div className="flex gap-2">
          <label className="flex flex-1 flex-col gap-1">
            <span className="text-xs text-muted-foreground">Host</span>
            <input
              className="rounded-sm border border-border bg-background px-2 py-1 font-mono text-xs"
              value={settings.host}
              disabled={connecting}
              autoComplete="off"
              spellCheck={false}
              onChange={(event) => setSettings({ ...settings, host: event.target.value })}
            />
          </label>
          <label className="flex w-24 flex-col gap-1">
            <span className="text-xs text-muted-foreground">Port</span>
            <input
              className="rounded-sm border border-border bg-background px-2 py-1 font-mono text-xs"
              value={settings.port}
              disabled={connecting}
              inputMode="numeric"
              autoComplete="off"
              onChange={(event) => setSettings({ ...settings, port: event.target.value })}
            />
          </label>
        </div>

        <label className="flex flex-col gap-1">
          <span className="text-xs text-muted-foreground">
            Token <span className="text-subtle-text">— leave blank if the host has no auth</span>
          </span>
          <input
            className="rounded-sm border border-border bg-background px-2 py-1 font-mono text-xs"
            value={settings.token}
            disabled={connecting}
            autoComplete="off"
            spellCheck={false}
            onChange={(event) => setSettings({ ...settings, token: event.target.value })}
          />
        </label>

        {error !== null && (
          <div
            role="alert"
            className="rounded-sm border border-destructive px-3 py-2 text-xs flex flex-col gap-1"
          >
            <span className="font-medium text-destructive">{FAILURE_TITLE[error.failure]}</span>
            <span className="text-muted-foreground">{error.message}</span>
          </div>
        )}

        <button
          type="submit"
          disabled={connecting}
          className="rounded-sm bg-signal px-3 py-1.5 text-xs font-medium text-signal-foreground disabled:opacity-60"
        >
          {connecting ? "Connecting…" : "Connect"}
        </button>
      </form>
    </div>
  );
}
