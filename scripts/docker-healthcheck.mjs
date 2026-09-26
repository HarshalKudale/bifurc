/**
 * The container's healthcheck — `plan/17` work item 5.
 *
 * ## What it probes, and why that one
 *
 * The **companion** — the RPC-over-WebSocket endpoint the `CMD` starts. It is the listener this image exists
 * to expose, and the one whose absence means "the host is not up". Probing the **edge** or the **forward
 * proxy** would be wrong for the opposite reason: both are *reported* rather than thrown when they fail to
 * bind (`plan/17` §1), so a container whose edge lost a port race would be marked unhealthy while the host
 * was serving RPC perfectly well.
 *
 * ## Why this is a script and not the one-line `node -e` it replaced
 *
 * The old probe hardcoded `4747`, which was correct only because the image's `CMD` also hardcoded it — a
 * coincidence, and one that broke the moment an operator overrode `--ws-port`. It now reads the port the
 * **entrypoint actually resolved and bound**, from the file the entrypoint writes, so the probe and the
 * listener agree by construction. The `Dockerfile` and `DOCKER.md` both used to carry a *"a `--ws-port`
 * override must be paired with a `--health-cmd` override"* caveat; this is what retires it.
 *
 * ## It is a TCP connect, not `GET /healthz`
 *
 * There is still no health endpoint (`plan/10` work item 6 asks for one), and a probe that pretended
 * otherwise would report a healthy container as unhealthy.
 */
import { readFile } from "node:fs/promises";
import { connect } from "node:net";
import { PORT_FILE, resolveCompanionPort } from "./docker-entrypoint.mjs";

/** Long enough for a loopback connect, short enough that three retries fit in the interval. */
const CONNECT_TIMEOUT_MS = 2000;

async function portToProbe() {
  /**
   * The entrypoint's answer first, because it is the only one that is a **fact**: it is the port the host
   * was told to bind. The environment and the default are what the entrypoint *would* have used, which is
   * the best available guess when the container was started with a different entrypoint (`--entrypoint …`),
   * and they are the same values — so the fallback agrees with the file in every case the file exists.
   */
  const published = await readFile(PORT_FILE, "utf8").catch(() => undefined);
  if (published !== undefined) {
    const port = Number(published.trim());
    if (Number.isInteger(port) && port > 0) return port;
  }

  // Throws on a malformed `BIFURC_COMPANION_PORT`, which is deliberate: the entrypoint would have refused
  // to start for the same reason, so a container in this state is misconfigured rather than merely down.
  return resolveCompanionPort([], process.env);
}

function probe(port) {
  return new Promise((resolve) => {
    const socket = connect({ port, host: "127.0.0.1" });
    const done = (ok, detail) => {
      socket.destroy();
      resolve({ ok, detail });
    };
    socket.setTimeout(CONNECT_TIMEOUT_MS);
    socket.once("connect", () => done(true, `companion ${port} accepted a connection`));
    socket.once("timeout", () => done(false, `companion ${port} did not answer within ${CONNECT_TIMEOUT_MS}ms`));
    socket.once("error", (err) => done(false, `companion ${port}: ${err.code ?? err.message}`));
  });
}

try {
  const port = await portToProbe();
  const result = await probe(port);
  console.log(result.detail);
  process.exit(result.ok ? 0 : 1);
} catch (err) {
  console.error(`healthcheck: ${err instanceof Error ? err.message : String(err)}`);
  process.exit(1);
}
