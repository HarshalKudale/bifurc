/**
 * The web host's outer container — the connection screen, the reconnect loop, and the chrome.
 *
 * ## What this is, and why it exists
 *
 * `web/` is a **container** for `packages/gui`, not a mount point. It shows a connection form first, and
 * only renders `<App />` once the handshake has succeeded. That ordering is not a preference: the browser
 * has to authenticate before its first real request (`connect.ts` explains why), and `<App />` reads
 * `window.api` during mount. Rendering first and connecting after is exactly the order that produced
 * *"the first request must be hello"* and a UI that died on arrival.
 *
 * ## Reconnection, and the one thing that makes it cheap
 *
 * `plan/08`'s criterion is *"disconnect/reconnect works; the UI never silently freezes"*. When the session
 * dies this container **reassigns `window.api`** and keeps the UI mounted — no remount, no state loss, and
 * no change to any of the UI's 240 call sites, because each one reads the global at the moment it calls.
 * The alternative, threading a client through React context, would have meant editing all 240 for a
 * capability the seam already provides.
 *
 * Two things the reconnect owes the engine, both from `plan/05` work item 3:
 *
 * - **`lastSeq`.** A reconnect sends the highest event `seq` already seen, so the engine can replay what
 *   was missed instead of leaving a hole. Without it a brief blip loses log entries and leaves the UI
 *   showing stale state forever — the plan calls that *"the single most likely source of 'works locally,
 *   broken remotely' bugs"*.
 * - **`resyncRequired`.** When the engine *cannot* replay — the buffer overran — it says so, and the
 *   client must re-fetch authoritative state rather than wait for events that will never arrive. `plan/05`:
 *   *"silently ignoring it produces a UI that is permanently wrong in a way nobody can reproduce."* So it
 *   is surfaced, not swallowed.
 *
 * ## Why the loop never gives up
 *
 * A tab left open overnight should recover on its own, so the backoff caps rather than terminates. That
 * makes "stop trying" a user action, which is why the banner carries a **Cancel** that returns to the form
 * — a state the user can leave is different from one they are stuck in.
 */
import { useCallback, useEffect, useRef, useState, type FormEvent } from "react";
import { App, type AppSession } from "@bifurc/gui";

import { connectAndInstall, uninstallWindowApi, type InstalledApi } from "./api";
import { ConnectError, type ConnectFailure } from "./connect";
import { fromUrl, loadSettings, saveSettings, toUrl, type HostSettings } from "./settings";

/**
 * Reconnect delays, then the last value repeats.
 *
 * Starts at 1 s because the common case is a host being restarted, and caps at 30 s so a long outage does
 * not become a request storm from every open tab.
 */
const RECONNECT_DELAYS_MS = [1000, 2000, 4000, 8000, 15000, 30000];

const delayFor = (attempt: number): number =>
  RECONNECT_DELAYS_MS[Math.min(attempt - 1, RECONNECT_DELAYS_MS.length - 1)]!;

type Phase =
  | { kind: "form"; error: ConnectError | null }
  | { kind: "connecting" }
  | { kind: "connected" }
  | { kind: "reconnecting"; attempt: number; reason: string; delayMs: number };

/**
 * A scannable heading per failure, so the user reads *what kind* of problem it is before the sentence.
 *
 * The four are genuinely different instructions: check the address, check what else is using the endpoint,
 * upgrade one side, or read the engine's own reason.
 *
 * `unauthorized`'s heading used to read *"The host rejected the token"*. There is no token, so the heading
 * names the state rather than a cause the user could fix by retyping something — `connect.ts`'s
 * `classify()` carries what reaching it now means.
 */
const FAILURE_TITLE: Record<ConnectFailure, string> = {
  unreachable: "Could not reach a host",
  unauthorized: "The host refused the session",
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
  /**
   * Bumped on every successful connection. A change is the UI's signal to re-fetch — which is how the
   * *reconnect* case is handled, and also how `resyncRequired` is honoured, since both mean the same
   * thing to the UI: "what you are holding may be stale".
   */
  const [generation, setGeneration] = useState(0);
  const [resyncRequired, setResyncRequired] = useState(false);
  const [engineVersion, setEngineVersion] = useState<string | undefined>(undefined);

  /**
   * The live session and the last-known settings, both as refs.
   *
   * Refs rather than state because `onFatal` fires from a socket handler — outside React's render, with no
   * guarantee about which render's closure it sees. Reading state there would read whatever was captured
   * when the handler was created, which for a long-lived connection is the *first* render's values.
   */
  const sessionRef = useRef<InstalledApi | null>(null);
  const settingsRef = useRef<HostSettings>(settings);
  /**
   * The highest event `seq` seen, carried **across** reconnects so the engine can replay the gap. Read
   * from the dying session before it is replaced, which is the only moment the value is still available.
   */
  const lastSeqRef = useRef(0);

  const handleFatal = useCallback((error: Error) => {
    // Capture the sequence first: after this the client is unreachable, and `lastSeq` is the one thing
    // the *next* connection cannot reconstruct.
    lastSeqRef.current = sessionRef.current?.lastSeq() ?? lastSeqRef.current;
    sessionRef.current = null;
    /**
     * **`window.api` is deliberately left installed.**
     *
     * Uninstalling it looks tidier and is strictly worse: the UI is still mounted, so its next call reads
     * `window.api.getConfig` on `undefined` and throws a `TypeError`. Measured — the reconnect check
     * produced exactly that, twice: `Cannot read properties of undefined (reading 'getConfig')`. Left in
     * place, the same call **rejects** with the client's own "this session ended", which the UI already
     * catches. A rejection is a state the UI can render; a `TypeError` is a crash.
     */
    setPhase({
      kind: "reconnecting",
      attempt: 1,
      reason: error.message,
      delayMs: delayFor(1),
    });
  }, []);

  /** One connection attempt. Shared by the form and the reconnect loop so they cannot drift. */
  const attemptConnect = useCallback(
    async (next: HostSettings, lastSeq: number | undefined): Promise<InstalledApi> => {
      const session = await connectAndInstall({
        url: toUrl(next),
        clientVersion: __APP_VERSION__,
        ...(lastSeq === undefined ? {} : { lastSeq }),
        onFatal: handleFatal,
      });
      sessionRef.current = session;
      settingsRef.current = next;
      lastSeqRef.current = session.lastSeq();
      setEngineVersion(session.hello.engineVersion);
      // `hello.resyncRequired` is the engine saying replay was impossible, so a re-fetch is mandatory
      // rather than merely prudent. It is no longer defaulted: the handshake is unconditional, so there is
      // always an answer to read — see `connect.ts`'s header.
      setResyncRequired(session.hello.resyncRequired);
      return session;
    },
    [handleFatal],
  );

  const handleConnect = useCallback(
    async (next: HostSettings) => {
      setPhase({ kind: "connecting" });
      try {
        // No `lastSeq` on a user-initiated connect: this is a fresh session, and the form is not a
        // reconnect. Sending one would ask the engine to replay into a UI that has just been reset.
        await attemptConnect(next, undefined);
        saveSettings(next);
        setSettings(next);
        setGeneration((g) => g + 1);
        setPhase({ kind: "connected" });
      } catch (error) {
        setPhase({ kind: "form", error: asConnectError(error) });
      }
    },
    [attemptConnect],
  );

  /**
   * Drive the reconnect when the phase says so.
   *
   * The attempt is scheduled by the phase, and each outcome *replaces* the phase — which is what makes the
   * loop self-driving without a recursive timer: success moves to `connected`, failure re-enters
   * `reconnecting` with the next delay, and the effect re-runs on that change.
   */
  useEffect(() => {
    if (phase.kind !== "reconnecting") return;

    let cancelled = false;
    const timer = setTimeout(() => {
      void (async () => {
        try {
          await attemptConnect(settingsRef.current, lastSeqRef.current);
          if (cancelled) return;
          setGeneration((g) => g + 1);
          setPhase({ kind: "connected" });
        } catch (error) {
          if (cancelled) return;
          const failure = asConnectError(error);

          /**
           * **Two failures are not transient, and retrying them is worse than giving up.**
           *
           * `unauthorized` used to be the *expected* outcome of a host restart: the engine token rotated
           * on every start, so a reconnect could never succeed against one, and the loop would have spun
           * forever behind a banner telling the user to go and read a new token. That reason went with the
           * token (2026-09-25). The decision stands on a narrower one — this client always sends `hello`
           * as its first request, so an `UNAUTHORIZED` means the endpoint is answering somebody else, and
           * `connect.ts`'s `classify()` names the two shapes that produces. That is not a host to wait
           * for; it is a condition the user has to see, and a backoff loop would hide it.
           *
           * A protocol major mismatch is the same shape: no amount of waiting changes it.
           *
           * Everything else — a refused socket, a host that has not finished booting — is genuinely worth
           * retrying, which is why the loop exists at all.
           */
          if (failure.failure === "unauthorized" || failure.failure === "version") {
            sessionRef.current = null;
            setPhase({ kind: "form", error: failure });
            return;
          }

          const attempt = phase.attempt + 1;
          setPhase({
            kind: "reconnecting",
            attempt,
            reason: failure.message,
            delayMs: delayFor(attempt),
          });
        }
      })();
    }, phase.delayMs);

    return () => {
      // A phase change (or an unmount) must not leave a timer that fires into a dead component.
      cancelled = true;
      clearTimeout(timer);
    };
  }, [phase, attemptConnect]);

  const handleDisconnect = useCallback(async () => {
    const session = sessionRef.current;
    sessionRef.current = null;
    // Order matters: stop new calls, unmount the UI that makes them, then close the transport.
    uninstallWindowApi();
    setPhase({ kind: "form", error: null });
    await session?.close();
  }, []);

  const handleCancelReconnect = useCallback(async () => {
    setPhase({ kind: "form", error: null });
    // Nothing to close — a reconnect that is still pending has no session — but a *previous* one may have
    // been captured by the fatal handler.
    await sessionRef.current?.close();
    sessionRef.current = null;
  }, []);

  if (phase.kind === "connected" || phase.kind === "reconnecting") {
    const url = toUrl(settingsRef.current);
    const lost = phase.kind === "reconnecting" ? phase : null;
    const session: AppSession = {
      url,
      generation,
      resyncRequired,
      ...(engineVersion === undefined ? {} : { engineVersion }),
    };
    return (
      <div className="flex flex-col h-screen bg-background text-foreground overflow-hidden">
        <HostBar
          url={url}
          engineVersion={engineVersion}
          tone={lost === null ? "ok" : "lost"}
          note={
            lost !== null
              ? `Lost the host — reconnecting (attempt ${lost.attempt}, next in ${Math.round(
                  lost.delayMs / 1000,
                )}s). ${lost.reason}`
              : resyncRequired
                ? "The engine could not replay missed events — refreshing."
                : undefined
          }
          action={
            lost === null
              ? { label: "Disconnect", onClick: () => void handleDisconnect() }
              : { label: "Cancel", onClick: () => void handleCancelReconnect() }
          }
        />
        {/*
          `min-h-0` so the flex child may shrink; without it the UI's own height pushes the bar off-screen.

          **`<App />` is rendered in both phases, and that is the whole point of merging these branches.**
          An earlier version rendered a replacement tree while reconnecting, which unmounted the UI and
          discarded every open tab and draft for an outage the user may not have noticed — while the comment
          right there claimed the opposite. Measured: `nav` was `0` during the outage *and* after recovery.
          `plan/08` asks for a **banner**, not a teardown.
        */}
        <div className="flex-1 min-h-0">
          <App host="web" session={session} />
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
          {/*
            This paragraph used to send the user to `engine.token` in the host's data directory. There is no
            token (2026-09-25), so it now states the two facts that decide whether a connection is allowed:
            where the listener is, and which page the engine will answer.
          */}
          <p className="text-xs text-muted-foreground">
            The host prints its RPC endpoint on startup. Every listener binds loopback, so this reaches a
            host on this device only. The engine answers a browser only when its page is served from
            loopback or a <code className="font-mono">*.localhost</code> name.
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

/** The strip above the UI: which host, which engine, and the one action that applies. */
function HostBar({
  url,
  engineVersion,
  tone,
  note,
  action,
}: {
  url: string;
  engineVersion?: string;
  tone: "ok" | "lost";
  note?: string;
  action: { label: string; onClick: () => void };
}) {
  return (
    <div className="flex items-center gap-3 border-b border-border bg-surface px-3 py-1.5 text-xs flex-shrink-0">
      <span
        className={`h-2 w-2 rounded-full flex-shrink-0 ${tone === "ok" ? "bg-signal" : "bg-destructive"}`}
        aria-hidden="true"
      />
      <span className="text-muted-foreground">{tone === "ok" ? "Connected to" : "Lost"}</span>
      <span className="font-mono text-foreground">{url}</span>
      {engineVersion !== undefined && (
        <span className="text-subtle-text">engine {engineVersion}</span>
      )}
      {note !== undefined && (
        <span className="text-destructive truncate" title={note}>
          {note}
        </span>
      )}
      <span className="flex-1" />
      <button
        type="button"
        onClick={action.onClick}
        className="rounded-sm border border-border px-2 py-0.5 text-muted-foreground hover:text-foreground"
      >
        {action.label}
      </button>
    </div>
  );
}
