import simpleGit, { SimpleGit } from "simple-git";
import * as path from "path";
import * as fs from "fs";
import * as os from "os";
import { app } from "electron";
import { AuditAction, AuditEntity, AuditEntry } from "@/store/types";
import { wsDir as _wsDir, setDataRootOverride } from "@/store/workspaceFs";

export type { AuditAction, AuditEntity, AuditEntry };

// ── Git singleton per workspace ───────────────────────────────────────────────

const _gitCache = new Map<string, SimpleGit>();

function deviceName(): string {
  return os.hostname() || "unknown";
}

function resolvedWsDir(wsId: string): string {
  return _wsDir(wsId);
}

export function getGit(wsId: string): SimpleGit {
  const dir = resolvedWsDir(wsId);
  if (!_gitCache.has(wsId)) _gitCache.set(wsId, simpleGit(dir));
  return _gitCache.get(wsId)!;
}

// For tests: override the data root so git operates on a temp dir
export function setDataDirOverride(root: string): void {
  setDataRootOverride(root || null);
  _gitCache.clear();
}

// ── Startup ────────────────────────────────────────────────────────────────────

export async function checkGitInstalled(): Promise<boolean> {
  try { await simpleGit().raw(["--version"]); return true; } catch { return false; }
}

export async function initWorkspaceRepo(wsId: string): Promise<void> {
  const dir = resolvedWsDir(wsId);
  const name = deviceName();
  if (!fs.existsSync(path.join(dir, ".git"))) {
    const g = simpleGit(dir);
    await g.init();
    await g.addConfig("user.email", `${name}@bifurc`, false, "local");
    await g.addConfig("user.name", name, false, "local");
    // .gitignore was written by initWorkspaceDir()
    if (fs.existsSync(path.join(dir, ".gitignore"))) {
      await g.add(".gitignore");
      await g.commit("chore: init workspace repo");
    }
    _gitCache.set(wsId, g);
  } else {
    const g = simpleGit(dir);
    // Update user config to current hostname (idempotent)
    try {
      await g.addConfig("user.email", `${name}@bifurc`, false, "local");
      await g.addConfig("user.name", name, false, "local");
    } catch {}
    _gitCache.set(wsId, g);
  }
}

// ── Commit ─────────────────────────────────────────────────────────────────────

export async function commitMutation(opts: {
  action: AuditAction;
  entity: AuditEntity;
  entityId: string;
  entityName: string;
  workspaceId: string;
  relPath: string;       // entity file path relative to workspace root
  actor?: string;
  changedFields?: string[];
}): Promise<string> {
  const actor = opts.actor ?? deviceName();
  const fieldsStr = opts.changedFields && opts.changedFields.length > 0
    ? ` [${opts.changedFields.join(",")}]`
    : "";
  // Subject IS the human-readable row label: "create mock EntityName" or "update mock [f1,f2] EntityName"
  const subject = `${opts.action} ${opts.entity}${fieldsStr} ${opts.entityName}`;
  const body = [
    `entity-id: ${opts.entityId}`,
    `workspace-id: ${opts.workspaceId}`,
    `actor: ${actor}`,
  ].join("\n");

  const g = getGit(opts.workspaceId);

  if (opts.action === "delete") {
    try { await g.raw(["rm", "--cached", "--ignore-unmatch", opts.relPath]); } catch {}
  } else {
    try { await g.add(opts.relPath); } catch {}
  }

  // Skip commit if nothing staged
  const status = await g.status();
  const hasStagedChanges =
    status.staged.length > 0 ||
    status.created.length > 0 ||
    status.deleted.length > 0 ||
    status.renamed.length > 0;

  if (!hasStagedChanges) return "";

  const result = await g.commit(`${subject}\n\n${body}`);
  return result.commit;
}

// ── Query ──────────────────────────────────────────────────────────────────────

export interface QueryLogOptions {
  workspaceId: string;
  entity?: AuditEntity;
  action?: AuditAction;
  entityId?: string;
  filePath?: string;     // filter by specific entity file (e.g. "mocks/root/mock_abc.json")
  fromTs?: number;
  toTs?: number;
  search?: string;
  limit?: number;
  offset?: number;
}

export async function queryLog(opts: QueryLogOptions): Promise<{ entries: AuditEntry[]; total: number }> {
  const dir = resolvedWsDir(opts.workspaceId);
  if (!fs.existsSync(path.join(dir, ".git"))) return { entries: [], total: 0 };

  const g = getGit(opts.workspaceId);

  const args: string[] = ["log", "--format=%H%n%at%n%s%n%b%n---END---"];
  if (opts.filePath) args.push("--", opts.filePath);

  let raw: string;
  try { raw = await g.raw(args); } catch { return { entries: [], total: 0 }; }

  const blocks = raw.split("---END---\n").filter((b) => b.trim());
  let entries: AuditEntry[] = blocks.map((block): AuditEntry | null => {
    const lines = block.trim().split("\n");
    const [hash, tsStr, subject, ...bodyLines] = lines;
    if (!hash || !tsStr || !subject) return null;
    const body = bodyLines.join("\n");
    const entityId  = body.match(/entity-id: (.+)/)?.[1]?.trim() ?? "";
    const wsId      = body.match(/workspace-id: (.+)/)?.[1]?.trim() ?? "";
    const actor     = body.match(/actor: (.+)/)?.[1]?.trim() ?? "local";

    // New format: "create mock EntityName" or "update mock [f1,f2] EntityName"
    const newMatch = subject.match(/^(create|update|delete) (\w+)(?: \[([^\]]*)\])? (.+)$/);
    // Legacy format: "create(mock): EntityName" or "update(mock): EntityName — msg"
    const legacyMatch = subject.match(/^(create|update|delete)\((\w+)\): (.+)$/);

    let action: AuditAction, entity: AuditEntity, entityName: string, changedFields: string[] | undefined;

    if (newMatch) {
      action  = newMatch[1] as AuditAction;
      entity  = newMatch[2] as AuditEntity;
      changedFields = newMatch[3] ? newMatch[3].split(",").map((s) => s.trim()).filter(Boolean) : undefined;
      entityName = newMatch[4].trim();
    } else if (legacyMatch) {
      action  = legacyMatch[1] as AuditAction;
      entity  = legacyMatch[2] as AuditEntity;
      entityName = legacyMatch[3].split(" — ")[0].trim();
      changedFields = undefined;
    } else {
      return null;
    }

    return {
      commitHash: hash.trim(),
      ts: parseInt(tsStr, 10) * 1000,
      action,
      entity,
      entityName,
      entityId,
      workspaceId: wsId,
      actor,
      changedFields,
    };
  }).filter((e): e is AuditEntry => e !== null);

  if (opts.entity)   entries = entries.filter((e) => e.entity === opts.entity);
  if (opts.action)   entries = entries.filter((e) => e.action === opts.action);
  if (opts.entityId) entries = entries.filter((e) => e.entityId === opts.entityId);
  if (opts.fromTs)   entries = entries.filter((e) => e.ts >= opts.fromTs!);
  if (opts.toTs)     entries = entries.filter((e) => e.ts <= opts.toTs!);
  if (opts.search)   entries = entries.filter((e) => e.entityName.toLowerCase().includes(opts.search!.toLowerCase()));

  const total  = entries.length;
  const offset = opts.offset ?? 0;
  const limit  = opts.limit === 0 ? entries.length : (opts.limit ?? 200);
  return { entries: entries.slice(offset, offset + limit), total };
}

// ── Point-in-time reads ────────────────────────────────────────────────────────

export async function getEntityAtCommit(
  commitRef: string,
  wsId: string,
  relPath: string,       // e.g. "mocks/root/mock_abc.json"
): Promise<unknown | null> {
  try {
    const content = await getGit(wsId).show(`${commitRef}:${relPath}`);
    return JSON.parse(content);
  } catch { return null; }
}

/** Return the list of files changed by a given commit (paths relative to workspace root). */
export async function getCommitChangedFiles(commitRef: string, wsId: string): Promise<string[]> {
  try {
    const raw = await getGit(wsId).raw(["diff-tree", "--no-commit-id", "-r", "--name-only", commitRef]);
    return raw.split("\n").map((l) => l.trim()).filter(Boolean);
  } catch { return []; }
}
