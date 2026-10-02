import * as fs from "node:fs";
import { COMMANDS } from "@bifurc/protocol";
import { flag, flags, has, type ParsedArgs } from "./args";
import { CliError, type EngineConnection } from "./engine";

export interface Output {
  json: boolean;
  /** A value: printed as JSON with `--json`, otherwise through `human`. */
  print(value: unknown, human?: () => string): void;
}

export interface CommandContext {
  args: ParsedArgs;
  out: Output;
  /** Opens the engine session on first use, so commands that need none (e.g. `commands`) never connect. */
  engine(): Promise<EngineConnection>;
}

type Row = Record<string, unknown>;

export function table(rows: Row[], columns: string[]): string {
  if (rows.length === 0) return "(none)";
  const cell = (v: unknown): string => (v === undefined || v === null ? "" : String(v));
  const widths = columns.map((c) => Math.max(c.length, ...rows.map((r) => cell(r[c]).length)));
  const line = (vals: string[]): string => vals.map((v, i) => v.padEnd(widths[i])).join("  ").trimEnd();
  return [line(columns.map((c) => c.toUpperCase())), ...rows.map((r) => line(columns.map((c) => cell(r[c]))))].join("\n");
}

/** The workspace a command acts on: `--workspace`, else the engine's active one. */
async function workspaceId(ctx: CommandContext): Promise<string> {
  const explicit = flag(ctx.args, "workspace");
  if (explicit !== undefined) return explicit;
  const cfg = await (await ctx.engine()).request<{ activeWorkspaceId: string }>("config.get");
  return cfg.activeWorkspaceId;
}

async function listEntities(ctx: CommandContext, kind: string): Promise<Row[]> {
  const wsId = await workspaceId(ctx);
  const res = await (await ctx.engine()).request<{ entities: Row[] }>("entity.list", { workspaceId: wsId, kind });
  return res.entities;
}

function need(value: string | undefined, what: string): string {
  if (value === undefined || value === "") throw new CliError(`missing ${what}`);
  return value;
}

/** `-H "name: value"` → a header map. */
function headersFrom(args: ParsedArgs): Record<string, string> {
  const headers: Record<string, string> = {};
  for (const h of flags(args, "header")) {
    const i = h.indexOf(":");
    if (i <= 0) throw new CliError(`header "${h}" must be "name: value"`);
    headers[h.slice(0, i).trim()] = h.slice(i + 1).trim();
  }
  return headers;
}

function bodyFrom(args: ParsedArgs): string {
  const file = flag(args, "body-file");
  if (file !== undefined) return fs.readFileSync(file, "utf8");
  return flag(args, "body") ?? "";
}

function createdId(res: unknown): string | undefined {
  const r = res as { id?: string; entity?: { id?: string } } | null;
  return r?.id ?? r?.entity?.id;
}

// ── commands ────────────────────────────────────────────────────────────────

export async function workspaces(ctx: CommandContext, sub = "list"): Promise<void> {
  const engine = await ctx.engine();
  if (sub === "list") {
    const cfg = await engine.request<{ activeWorkspaceId: string; workspaces: Row[] }>("config.get");
    const rows = cfg.workspaces.map((w) => ({ ...w, active: w.id === cfg.activeWorkspaceId ? "*" : "" }));
    ctx.out.print(rows, () => table(rows, ["active", "id", "name"]));
    return;
  }
  if (sub === "use") {
    const id = need(ctx.args.positionals[2], "workspace id");
    await engine.request("workspace.setActive", { id });
    ctx.out.print({ ok: true, activeWorkspaceId: id }, () => `Active workspace: ${id}`);
    return;
  }
  throw new CliError(`unknown: workspaces ${sub}`);
}

/**
 * Environments of the active workspace. `Global` is always applied underneath; the selected one overrides it.
 * Selection is the engine's (and survives restarts), so it is shared with the desktop UI.
 */
export async function env(ctx: CommandContext, sub = "list"): Promise<void> {
  const engine = await ctx.engine();
  const cfg = await engine.request<{ activeWorkspaceId: string; activeEnvironmentId: string | null }>("config.get");
  const wsId = flag(ctx.args, "workspace") ?? cfg.activeWorkspaceId;
  if (sub === "list") {
    const res = await engine.request<{ entities: Row[] }>("entity.list", { workspaceId: wsId, kind: "environments" });
    const selectable = wsId === cfg.activeWorkspaceId;
    const rows = res.entities.map((e) => ({
      ...e,
      active: e.id === "__global__" ? "base" : selectable && e.id === cfg.activeEnvironmentId ? "*" : "",
      vars: Array.isArray(e.variables) ? e.variables.length : 0,
    }));
    ctx.out.print(rows, () => table(rows, ["active", "id", "name", "vars"]));
    return;
  }
  if (sub === "show") {
    const merged = await engine.request("env.getMerged", { workspaceId: wsId });
    ctx.out.print(merged, () => JSON.stringify(merged, null, 2));
    return;
  }
  if (sub === "use") {
    if (wsId !== cfg.activeWorkspaceId) {
      throw new CliError(`Environments are selected for the active workspace; run: bifurc workspaces use ${wsId}`);
    }
    const want = need(ctx.args.positionals[2], "environment id or name");
    const res = await engine.request<{ entities: Row[] }>("entity.list", { workspaceId: wsId, kind: "environments" });
    const hit =
      res.entities.find((e) => e.id === want) ??
      res.entities.find((e) => String(e.name).toLowerCase() === want.toLowerCase());
    if (hit === undefined) throw new CliError(`no environment "${want}" (see: bifurc env list)`);
    const out = await engine.request<{ ok: boolean; error?: string }>("env.setActive", { id: hit.id });
    if (!out.ok) throw new CliError(out.error === "cannot_activate_global" ? "Global is always active; pick another environment." : String(out.error));
    ctx.out.print({ ok: true, activeEnvironmentId: hit.id }, () => `Active environment: ${String(hit.name)} (${String(hit.id)})`);
    return;
  }
  throw new CliError(`unknown: env ${sub}`);
}

export async function mappings(ctx: CommandContext, sub = "list"): Promise<void> {
  const engine = await ctx.engine();
  const pos = ctx.args.positionals;
  if (sub === "list") {
    const rows = await listEntities(ctx, "mappings");
    ctx.out.print(rows, () => table(rows, ["id", "domain", "target", "label"]));
    return;
  }
  if (sub === "add") {
    const name = need(pos[2], "domain, e.g. api or api.localhost");
    const domain = name.toLowerCase().endsWith(".localhost") ? name.toLowerCase() : `${name.toLowerCase()}.localhost`;
    const target = need(pos[3], "target, e.g. 127.0.0.1:3000 or http://host:port");
    const res = await engine.request("entity.create", {
      kind: "mappings",
      workspaceId: await workspaceId(ctx),
      entity: { domain, target, label: flag(ctx.args, "label") ?? "", enabled: true },
    });
    ctx.out.print(res, () => `Mapped http://${domain} → ${target}  (id ${createdId(res) ?? "?"})`);
    return;
  }
  if (sub === "remove") {
    const id = need(pos[2], "mapping id");
    await engine.request("entity.delete", { kind: "mappings", id });
    ctx.out.print({ ok: true }, () => `Removed mapping ${id}`);
    return;
  }
  if (sub === "enable" || sub === "disable") {
    const id = need(pos[2], "mapping id");
    const res = await engine.request("entity.setEnabled", {
      workspaceId: await workspaceId(ctx), kind: "mappings", id, enabled: sub === "enable",
    });
    ctx.out.print(res, () => `${sub === "enable" ? "Enabled" : "Disabled"} mapping ${id}`);
    return;
  }
  throw new CliError(`unknown: mappings ${sub}`);
}

export async function services(ctx: CommandContext, sub = "list"): Promise<void> {
  if (sub !== "list") throw new CliError(`unknown: services ${sub}`);
  const rows = await (await ctx.engine()).request<Row[]>("services.discover");
  ctx.out.print(rows, () => table(rows, ["port", "pid", "processName", "address"]));
}

interface SendResult {
  status: number;
  headers: Record<string, string>;
  body: string;
  durationMs: number;
  tests?: { name: string; passed: boolean; error?: string }[];
  preScriptError?: string;
  postScriptError?: string;
}

function describeSend(r: SendResult): string {
  const lines = [`HTTP ${r.status}  (${r.durationMs} ms)`];
  for (const [k, v] of Object.entries(r.headers)) lines.push(`${k}: ${v}`);
  lines.push("", Buffer.from(r.body, "base64").toString("utf8"));
  if (r.preScriptError) lines.push("", `pre-script error: ${r.preScriptError}`);
  if (r.postScriptError) lines.push("", `post-script error: ${r.postScriptError}`);
  for (const t of r.tests ?? []) lines.push(`${t.passed ? "PASS" : "FAIL"}  ${t.name}${t.error ? ` — ${t.error}` : ""}`);
  return lines.join("\n");
}

export async function requests(ctx: CommandContext, sub = "list"): Promise<void> {
  const engine = await ctx.engine();
  const pos = ctx.args.positionals;
  if (sub === "list") {
    const rows = await listEntities(ctx, "requests");
    ctx.out.print(rows, () => table(rows, ["id", "method", "name", "url"]));
    return;
  }
  if (sub === "show") {
    const id = need(pos[2], "request id");
    const res = await engine.request<{ ok: boolean; entity?: Row }>("entity.load", {
      workspaceId: await workspaceId(ctx), kind: "requests", id,
    });
    if (!res.ok || res.entity === undefined) throw new CliError(`no request ${id}`);
    ctx.out.print(res.entity, () => JSON.stringify(res.entity, null, 2));
    return;
  }
  if (sub === "add") {
    const url = need(flag(ctx.args, "url") ?? pos[2], "--url");
    const method = (flag(ctx.args, "method") ?? "GET").toUpperCase();
    const res = await engine.request("entity.create", {
      kind: "requests",
      workspaceId: await workspaceId(ctx),
      entity: {
        name: flag(ctx.args, "name") ?? `${method} ${url}`,
        method, url, headers: headersFrom(ctx.args), body: bodyFrom(ctx.args), protocol: "rest",
      },
    });
    ctx.out.print(res, () => `Saved request ${createdId(res) ?? "?"}`);
    return;
  }
  if (sub === "send") {
    const wsId = await workspaceId(ctx);
    const url = flag(ctx.args, "url");
    // A saved request goes by id — the engine loads it, resolves it and runs its scripts. A request that is
    // not saved goes as a full payload.
    const payload = url === undefined
      ? { workspaceId: wsId, id: need(pos[2], "request id, or --url for an unsaved request") }
      : {
          workspaceId: wsId,
          request: {
            method: (flag(ctx.args, "method") ?? "GET").toUpperCase(),
            url, headers: headersFrom(ctx.args), body: bodyFrom(ctx.args),
          },
          scripts: {},
        };
    const res = await engine.request<SendResult>("request.send", payload);
    ctx.out.print(res, () => describeSend(res));
    return;
  }
  throw new CliError(`unknown: requests ${sub}`);
}

/** Any protocol command, with a JSON payload — everything the UI can do, the CLI can. */
export async function call(ctx: CommandContext): Promise<void> {
  const command = need(ctx.args.positionals[1], "command name (see: bifurc commands)");
  if (!(command in COMMANDS)) throw new CliError(`unknown command "${command}" (see: bifurc commands)`);
  const raw = ctx.args.positionals[2] ?? flag(ctx.args, "payload") ?? "{}";
  let payload: unknown;
  try {
    payload = JSON.parse(raw);
  } catch {
    throw new CliError(`payload is not valid JSON: ${raw}`);
  }
  const res = await (await ctx.engine()).request(command, payload);
  ctx.out.print(res, () => JSON.stringify(res, null, 2));
}

export function commands(ctx: CommandContext): void {
  const names = Object.keys(COMMANDS).sort();
  ctx.out.print(names, () => names.join("\n"));
}

export async function status(ctx: CommandContext): Promise<void> {
  const engine = await ctx.engine();
  const server = await engine.request<Row>("server.status");
  const cfg = await engine.request<{ activeWorkspaceId: string; workspaces: Row[] }>("config.get");
  const active = cfg.workspaces.find((w) => w.id === cfg.activeWorkspaceId);
  const value = { engine: engine.info, server, activeWorkspace: active ?? cfg.activeWorkspaceId };
  ctx.out.print(value, () =>
    [
      `engine     pid ${engine.info.pid}, ${engine.info.transport} ${engine.info.address}, v${engine.info.engineVersion}`,
      `data dir   ${engine.info.dataDir}`,
      `proxy      ${server.running ? `listening on ${server.port}` : `stopped${server.error ? ` (${server.error})` : ""}`}`,
      `workspace  ${active ? `${active.name} (${active.id})` : cfg.activeWorkspaceId}`,
    ].join("\n"),
  );
}

export { has };
