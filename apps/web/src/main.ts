/**
 * The page: read an engine URL, connect, and report what happened.
 *
 * Deliberately thin, and the split is the point. Every decision worth testing is in `connect.ts`; what
 * remains here is the part that can only be exercised by loading it in a browser, which is work item
 * 6's Playwright job and the owner's to run. Keeping the DOM out of `connect.ts` is what makes the
 * composition testable at all.
 */
import { connectToEngine } from "./connect";

const status = document.querySelector<HTMLParagraphElement>("#status");
const detail = document.querySelector<HTMLPreElement>("#detail");

/**
 * `?engine=ws://host:port`, defaulting to the loopback port `plan/14` D16 assigns.
 *
 * Read from the query rather than baked in at build time, so one bundle can point at a local engine, a
 * Docker-hosted one, or a remote one without a rebuild — which is the "connects to a local engine and
 * to a remote engine" acceptance criterion, and the reason `base: "./"` is set in the Vite config.
 */
function engineUrlFromLocation(): string {
  const fromQuery = new URLSearchParams(location.search).get("engine");
  return fromQuery ?? "ws://127.0.0.1:8080";
}

function show(line: string, extra?: unknown): void {
  if (status !== null) status.textContent = line;
  if (detail !== null) {
    detail.textContent = extra === undefined ? "" : JSON.stringify(extra, null, 2);
  }
}

async function main(): Promise<void> {
  const url = engineUrlFromLocation();
  show(`Connecting to ${url}…`);

  const connection = connectToEngine({
    url,
    // Reported rather than thrown: this fires from a socket handler, so there is no `try` for it to
    // land in. A page that showed nothing here would look identical to one that had never connected.
    onFatal: (error) => show(`Connection lost — ${error.message}`),
  });

  try {
    show(`Connected to ${url}`, await connection.probe());
  } catch (err) {
    show(
      `Could not reach an engine at ${url}`,
      err instanceof Error ? err.message : String(err),
    );
  }
}

void main();
