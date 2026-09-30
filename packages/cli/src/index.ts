#!/usr/bin/env node
import { platformDefaultDataDir } from "@bifurc/engine/workspace/paths";
import { findRunningEngine } from "@bifurc/engine/runtime/discovery";
import { flag, has, parseArgs, type ParsedArgs } from "./args";
import * as cmd from "./commands";
import { CliError, connect, startEngine, stopEngine, type EngineConnection } from "./engine";

const VERSION = "0.1.0";

export const USAGE = `Usage: bifurc <command> [options]

Talks to the Bifurc engine that serves a data directory, over its socket (a named pipe on Windows).
The engine is found through <data-dir>/runtime.json.

Engine
  engine start [--transport socket|ws]   Start a standalone engine in the background
  engine stop                            Stop it gracefully
  engine status                          Where it runs, without opening a session
  status                                 Engine, proxy and active workspace

Workspaces
  workspaces list
  workspaces use <id>

Mappings (<sub>.localhost → target)
  mappings list
  mappings add <sub|domain> <target> [--label <text>]
  mappings remove <id>
  mappings enable|disable <id>

Services
  services list                          Local processes listening on a port

Requests
  requests list
  requests show <id>
  requests add --url <url> [--method GET] [--name <n>] [-H "k: v"]... [--body <text> | --body-file <f>]
  requests send <id>                     Send a saved request by id (engine resolves and runs its scripts)
  requests send --url <url> [--method] [-H "k: v"]... [--body …]   Send an unsaved request

Anything else
  call <command> [<json-payload>]        Any engine command, e.g. call server.status
  commands                               List every engine command

Options
  --data-dir <path>    Default: the desktop app's data directory (or BIFURC_DATA_DIR)
  -w, --workspace <id> Default: the active workspace
  --json               Machine-readable output
  -h, --help`;

function dataDirOf(args: ParsedArgs): string {
  return flag(args, "data-dir") ?? (process.env.BIFURC_DATA_DIR || platformDefaultDataDir());
}

async function engineCommand(args: ParsedArgs, out: cmd.Output, dataDir: string): Promise<void> {
  const sub = args.positionals[1] ?? "status";
  if (sub === "status") {
    const info = findRunningEngine(dataDir);
    out.print(info ?? { running: false, dataDir }, () =>
      info ? `running  pid ${info.pid}, ${info.transport} ${info.address}` : `not running for ${dataDir}`,
    );
    return;
  }
  if (sub === "start") {
    const running = findRunningEngine(dataDir);
    if (running !== null) {
      out.print(running, () => `already running  pid ${running.pid}, ${running.transport} ${running.address}`);
      return;
    }
    const transport = flag(args, "transport") ?? "socket";
    if (transport !== "socket" && transport !== "ws") throw new CliError("--transport must be socket or ws");
    const info = await startEngine(dataDir, transport);
    out.print(info, () => `started  pid ${info.pid}, ${info.transport} ${info.address}`);
    return;
  }
  if (sub === "stop") {
    const conn = await connect(dataDir, VERSION);
    await stopEngine(conn, dataDir);
    out.print({ ok: true }, () => `stopped  pid ${conn.info.pid}`);
    return;
  }
  throw new CliError(`unknown: engine ${sub}`);
}

export async function main(argv: readonly string[]): Promise<number> {
  const args = parseArgs(argv);
  const json = has(args, "json");
  const out: cmd.Output = {
    json,
    print(value, human) {
      process.stdout.write(`${json || human === undefined ? JSON.stringify(value, null, 2) : human()}\n`);
    },
  };
  const [group, sub] = args.positionals;
  if (group === undefined || has(args, "help") || group === "help") {
    process.stdout.write(`${USAGE}\n`);
    return 0;
  }

  const dataDir = dataDirOf(args);
  let conn: EngineConnection | undefined;
  const ctx: cmd.CommandContext = {
    args,
    out,
    engine: async () => (conn ??= await connect(dataDir, VERSION)),
  };

  try {
    switch (group) {
      case "engine": await engineCommand(args, out, dataDir); break;
      case "status": await cmd.status(ctx); break;
      case "workspaces": case "workspace": await cmd.workspaces(ctx, sub); break;
      case "mappings": case "mapping": await cmd.mappings(ctx, sub); break;
      case "services": await cmd.services(ctx, sub); break;
      case "requests": case "request": await cmd.requests(ctx, sub); break;
      case "call": await cmd.call(ctx); break;
      case "commands": cmd.commands(ctx); break;
      default: throw new CliError(`unknown command "${group}"\n\n${USAGE}`);
    }
    return 0;
  } catch (err) {
    const message = err instanceof CliError || !(err instanceof Error) ? String((err as Error)?.message ?? err) : err.message;
    const code = (err as { code?: string }).code;
    process.stderr.write(`bifurc: ${code && !(err instanceof CliError) ? `${code}: ` : ""}${message}\n`);
    return 1;
  } finally {
    await conn?.close().catch(() => undefined);
  }
}

if (typeof require !== "undefined" && require.main === module) {
  void main(process.argv.slice(2)).then((code) => {
    process.exitCode = code;
  });
}
