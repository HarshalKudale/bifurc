/**
 * ## The listener fan-out — the one place that knows what a config write means for a listener
 *
 * `Engine_Module_Plan.md` §4.1. Before this module existed, the answer to *"after a config write, which
 * listeners have to be told?"* was spread across **29 call sites in 21 files** — 17 importers, four commands,
 * a startup wiring line — and every one of them reached into `proxy/server.ts` for `reloadConfig()`. That
 * made `proxy/server` the most-depended-on module in the engine for a reason that had nothing to do with the
 * proxy: it was where the *fan-out* happened to be written down first.
 *
 * This module is that answer, in one place. `kernel → utils → workspace → {features} → transport | runtime`
 * (§5.1) puts it in `runtime/` because it is a *composition* concern — the thing that binds a set of listeners
 * is the thing that knows how to poke them — and because `runtime → proxy` is the **allowed** direction. The
 * arrow must not reverse: `proxy/server.ts` must never import from here, or the layering rule is inverted.
 *
 * ### Two operations, not one
 *
 * §4.1 sketches a single `notifyConfigChanged()`. Measuring the real call sites splits the fan-out cleanly in
 * two, and conflating them under one name would hide the distinction that matters:
 *
 * 1. **Reload** (`notifyConfigChanged`) — the listener's rules are *derived from config*. A write happened;
 *    re-read and re-derive. This is the 29-site majority, and it is what the 17 importers of a bundle all mean.
 * 2. **Rebind** (`rebindRuntimePorts`) — the listener's **port** changed. A port is not derived from config;
 *    it is *owned* by the listener (or, for the edge, by the workspace). Moving it means `start(newPort)`, and
 *    only one command — `store/configCommands.ts`'s `config.save` — ever does it. `restartEdge` and
 *    `startWebhookServer` have **no caller outside that command**; keeping them reachable only from here is
 *    what makes that a fact rather than a coincidence.
 *
 * They are separate functions because they are separate questions. A listener can need one without the other —
 * the edge rebinds on a port change but has nothing to *reload* (a port is not derived from config), which is
 * exactly why `reverseproxy/edge.ts` deliberately has no `reloadConfig()` at all.
 */

import { isRunning, reloadConfig, startServer } from "../proxy/server";
import { isEdgeRunning, restartEdge } from "../reverseproxy/edge";
import { isWebhookServerRunning, startWebhookServer } from "../webhooks/webhookServer";
import type { AppConfig } from "../workspace/config";

/**
 * Re-derive every listener whose rules come from config.
 *
 * **Only the forward proxy is called, and the emptiness of the rest is the design.**
 *
 *  - **proxy** — `reloadConfig()` re-reads the config and swaps this instance's in-memory dispatch state.
 *    This is the entire body of today's fan-out, and the reason the function exists.
 *  - **edge** — nothing. A port is owned by its workspace, not by the active config, so there is nothing to
 *    reload; the edge's own docblock makes this explicit (`reverseproxy/edge.ts` L52, L108, L166).
 *  - **webhook** — nothing. Its active maps are *registered* by the webhook commands, not derived from config.
 *
 * A new listener lands here as one line, and **no caller changes** — which is the property §4.1 is buying.
 *
 * Uses the proxy **facade** (`proxy/server.ts`'s `reloadConfig`), not `defaultServer()`, because the facade is
 * what a composition can bind: the desktop's host binds its own instance and this call has to reach *that* one,
 * not a lazily created default.
 */
export function notifyConfigChanged(): void {
  reloadConfig();
}

/**
 * Re-bind the runtime-ownable ports after a config write — `plan/17` §5 work item 7, moved here from the body
 * of `config.save`.
 *
 * `plan/17` §1 splits the ports by *whether a change needs a restart*: only the **companion** does, and
 * manually. The edge, the forward proxy and the webhook **re-bind at runtime**, and this is that rule, in one
 * place instead of inline in a command.
 *
 * **Three branches, one per port, each with its own rule about whether it may move:**
 *
 *  - **the edge** on `settings.port` — bound, not scanned. A mapping URL that stops working because the port
 *    only took effect on the next boot is the failure mode this removes. The `!isEdgeRunning()` arm is the
 *    recovery path for a port that was occupied at boot: a save is the natural moment to retry.
 *  - **the forward proxy** on `settings.forwardProxyPort` — read straight from the setting rather than from
 *    `getPort() ||`, because `plan/17` §2 makes D16's remembered resolved port dead (there is no scan, so
 *    there is no resolved port that could differ from the configured one). The `tlsChanged` arm belongs here:
 *    `proxy/server.ts`'s `start()` is what loads the CA, and `reloadConfig()` swaps the config but not the CA.
 *  - **the webhook receiver** on `settings.webhookPort` — host-level and **off by default**, so this moves a
 *    receiver that is running and **never starts one**. Starting it is the user's action; a settings save that
 *    bound a port nobody asked for would be this function inventing a feature toggle.
 *
 * **Every `start()` in this engine already does `if (server) stop()` first**, so `start(newPort)` *is* the
 * rebind and an explicit stop would be a second way to say the same thing that could drift. **A bind failure
 * is not thrown** — it lands in the listener's `getError()` and is reported through `status()`, the engine's
 * oldest bind contract, which is what makes the "revive a listener that is down" arms safe.
 *
 * **The first bind is deliberately not here.** `runtime/host.ts` binds each listener directly
 * (`bindProxyInstance` / `bindWebhookInstance` / `bindEdgeInstance`, then `start()`), before any config exists
 * to diff against. This function only ever *moves* a bound listener.
 */
export function rebindRuntimePorts(incoming: AppConfig, prev: AppConfig): void {
  const tlsChanged = incoming.tlsEnabled !== prev.tlsEnabled
    || incoming.tlsCaCertPath !== prev.tlsCaCertPath
    || incoming.tlsCaKeyPath !== prev.tlsCaKeyPath;

  if (!isEdgeRunning() || incoming.port !== prev.port) {
    restartEdge(incoming.port);
  }

  if (!isRunning() || incoming.forwardProxyPort !== prev.forwardProxyPort || tlsChanged) {
    startServer(incoming.forwardProxyPort);
  }

  if (incoming.webhookPort !== prev.webhookPort && isWebhookServerRunning()) {
    startWebhookServer(incoming.webhookPort);
  }
}
