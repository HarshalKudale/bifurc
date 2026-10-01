import { PROTOCOL_VERSION } from "@bifurc/protocol";
import { bus as defaultBus, type EngineEventBus } from "../kernel/eventBus";
import { commandRegistry, type CommandRegistry } from "../kernel/registry";
import { createSocketServer } from "../transport/socket";
import { EventLog } from "../transport/eventLog";
import { DEFAULT_HOST_DOMAIN } from "../utils/net";
import { engineSocketPath, findRunningEngine, removeRuntimeInfo, writeRuntimeInfo } from "./discovery";

export interface AttachedSocket {
  address: string;
  close(): Promise<void>;
}

export interface AttachSocketOptions {
  dataDir: string;
  /** Reported to clients in `hello` and recorded in runtime.json. */
  engineVersion?: string;
  registry?: CommandRegistry;
  bus?: EngineEventBus;
  onNotice?: (message: string) => void;
}

/**
 * Serve the engine this process already runs over its data directory's socket, and publish it in runtime.json.
 *
 * For an engine embedded in a host application (the desktop shell) that talks to it in-process: the host keeps
 * its own transport, and the CLI or any other client reaches the very same engine through this socket instead of
 * starting a second one. Returns `null` when another live engine already owns the data directory.
 */
export async function attachEngineSocket(opts: AttachSocketOptions): Promise<AttachedSocket | null> {
  const notice = opts.onNotice ?? ((m: string) => console.log(`[engine] ${m}`));
  const running = findRunningEngine(opts.dataDir);
  if (running !== null && running.pid !== process.pid) {
    notice(`an engine (pid ${running.pid}) already serves ${opts.dataDir}; not opening a socket`);
    return null;
  }
  const bus = opts.bus ?? defaultBus;
  const registry = opts.registry ?? commandRegistry;
  const engineVersion = opts.engineVersion ?? "0.0.0";
  const log = new EventLog({ bus });
  const path = engineSocketPath(opts.dataDir);
  const server = await createSocketServer({
    path,
    registry,
    bus,
    log,
    auth: {
      engineVersion,
      hostDomain: DEFAULT_HOST_DOMAIN,
      onAuthFailure: (error, hello) =>
        notice(`rpc session refused (${error.code}) from "${hello.clientName ?? "unnamed"}": ${error.message}`),
    },
  });
  writeRuntimeInfo({
    pid: process.pid,
    transport: "socket",
    address: path,
    dataDir: opts.dataDir,
    protocolVersion: PROTOCOL_VERSION,
    engineVersion,
    startedAt: Date.now(),
  });
  notice(`rpc endpoint: ${path}`);
  return {
    address: path,
    async close() {
      removeRuntimeInfo(opts.dataDir);
      try {
        await server.close();
      } catch {
        /* a socket file another process already removed must not block shutdown */
      }
      log.close();
    },
  };
}
