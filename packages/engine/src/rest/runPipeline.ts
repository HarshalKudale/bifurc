/**
 * **The request pipeline** — `plan/19` D24: `resolve → pre-script → send → post-script → test`.
 *
 * ## Why this file exists
 *
 * `plan/19` §1 rows 3 and 4 found the renderer holding **two copies of this same five-step sequence**:
 * `packages/gui/src/components/rest/useRestActions.ts:25` (`handleSend`) and
 * `packages/gui/src/lib/collectionRunner.ts:94` (`executeOneRequest`). They were not obviously copies —
 * different call sites, different surrounding code, one of them inside a loop — which is exactly how a
 * duplicated pipeline survives review. D24 makes it **one engine function**; the two renderer copies are
 * deleted by WI 3, and the collection runner (WI 4) calls this instead of its own.
 *
 * ## The one thing it does not know: the protocol
 *
 * `send` is injected. Everything else — which environment to resolve against, what the pre-script is
 * allowed to rewrite, what the post-script and test-script see, how the response body crosses the text /
 * base64 boundary — is protocol-independent and lives here once. A caller that needs a payload this
 * function's `PipelineRequest` shape cannot express (GraphQL's query and variables, SOAP's action, gRPC's
 * metadata) builds it **inside its `send` closure**, resolving those fields with the same
 * `utils/resolve.ts` this file uses. That is not a second resolver; it is the same one, applied to fields
 * this shape does not carry.
 *
 * ## ⚠ The body crosses one boundary, in one direction, in one place
 *
 * `plan/19` §8 trap 4: **two body encodings coexist** and it is the kind of pair a refactor mixes up.
 * The rule this file implements, so that it is asserted rather than described:
 *
 * | stage | encoding |
 * |---|---|
 * | `PipelineRequest.body` — what the caller hands in, and what the pre-script rewrites | **text** |
 * | `PipelineRawResponse.body` — what comes back off the wire | **base64** |
 * | what the post-script and test-script see as `lp.response.body` | **text**, decoded here |
 *
 * `request.replay` is the raw primitive and keeps base64 (`plan/19` §5); a REST `send` closure therefore
 * encodes text→base64 on the way out, reproducing `textToB64`'s **blank-means-empty** rule. That encode
 * is the REST closure's, not this file's — this file never encodes, and the asymmetry is why trap 4 wants
 * a test.
 *
 * ## Faithfully reproduced, including one oddity
 *
 * This is a **move**, so the semantics are the renderer's rather than an improved version of them — with
 * one exception noted here because it looks like a bug and is deliberately preserved:
 *
 * **The pre-script's `lp.environment.set(…)` values are dropped when the workspace has no environment at
 * all.** Both renderer copies guard the hand-off with `if (activeEnv && …)`, so a pre-script can set a
 * variable and see the post-script ignore it whenever `activeEnv` is `null`. It is inconsistent, it is
 * invisible unless a workspace has no environments (the desktop's `loadConfig()` auto-creates
 * `__global__`, so it rarely does), and it is preserved because changing it inside a move would be a
 * behaviour change smuggled in under a refactor — the same reasoning `rest/commands.ts` records for the
 * `durationMs` field it does not return. Fixing it is a separate, deliberate change.
 *
 * ## A failed send throws; a failed script does not
 *
 * `send`'s rejection propagates, because the two callers disagree about what a failure means: a tab
 * surfaces it to the user, while the collection runner records it against that request and **continues**
 * (WI 4). Deciding that here would take the choice away from both. Script errors are different — they are
 * values (`preScriptError` / `postScriptError`), because a script that throws has still produced a
 * request and a response, and the renderer has always displayed them beside a successful send.
 */
import type { Environment } from "../workspace/types";
import { resolveVars, resolveHeaders } from "../env/resolve";
import { loadMergedEnvironment } from "../env/envState";
import { executeIpcScript, type TestResultEntry } from "../mocks/scriptExecutor";

/** The protocol-independent view of a request. `body` is **text** — see the header's encoding table. */
export interface PipelineRequest {
  method: string;
  url: string;
  headers: Record<string, string>;
  body: string;
}

/** What comes back off the wire. `body` is **base64**, the wire shape `plan/19` §5 keeps. */
export interface PipelineRawResponse {
  status: number;
  headers: Record<string, string>;
  body: string;
}

/** The three scripts a request may carry. Absent or whitespace-only means "do not run it". */
export interface PipelineScripts {
  pre?: string;
  post?: string;
  test?: string;
}

/**
 * Perform the send. Injected so this file knows no protocol — see the header.
 *
 * `env` is the **effective** environment: the workspace's merged environment, or that environment with
 * the pre-script's mutations applied. A closure that resolves its own fields must use this one, not a
 * freshly loaded one, or a pre-script's `lp.environment.set(…)` would not be visible to them.
 */
export type PipelineSend = (
  resolved: PipelineRequest,
  env: Environment | null,
) => Promise<PipelineRawResponse>;

export interface PipelineResult {
  /** What was actually sent — resolved, and rewritten by the pre-script if there was one. */
  request: PipelineRequest;
  response: PipelineRawResponse;
  tests: TestResultEntry[];
  testLogs: string[];
  preScriptError?: string;
  postScriptError?: string;
  /** The send's own duration, which is what the test-script's `lp.response.responseTime` reports. */
  durationMs: number;
}

export interface RunRequestPipelineOptions {
  workspaceId: string;
  /** The request as stored or authored, with every field **unresolved**. */
  request: PipelineRequest;
  scripts?: PipelineScripts;
  send: PipelineSend;
}

/** `Environment | null` → the flat map the sandbox's `lp.environment` reads. */
function envToMap(env: Environment | null): Record<string, string> {
  const map: Record<string, string> = {};
  if (!env) return map;
  for (const v of env.variables) map[v.key] = v.value;
  return map;
}

/**
 * The effective environment after a pre-script ran: its variables, keyed as it left them.
 *
 * Returns `base` untouched when there is no base environment — the preserved oddity in the header.
 */
function environmentAfterPreScript(
  base: Environment | null,
  envVars: Record<string, string>,
): Environment | null {
  if (!base) return base;
  if (Object.keys(envVars).length === 0) return base;
  return {
    ...base,
    variables: Object.entries(envVars).map(([key, value]) => ({ id: key, key, value })),
  };
}

/** base64 → text, reproducing the renderer's `b64ToText` — including its blank-means-empty rule. */
function b64ToText(b64: string): string {
  if (!b64) return "";
  return Buffer.from(b64, "base64").toString("utf-8");
}

export async function runRequestPipeline(
  opts: RunRequestPipelineOptions,
): Promise<PipelineResult> {
  // One read of the workspace's environment state for the whole pipeline. `plan/19` D23: the engine is
  // the authority for which environment a request resolves against, and the merge is part of that.
  const env = loadMergedEnvironment(opts.workspaceId);
  const scripts = opts.scripts ?? {};

  let resolved: PipelineRequest = {
    method: opts.request.method,
    url: resolveVars(opts.request.url, env),
    headers: resolveHeaders(opts.request.headers, env),
    body: resolveVars(opts.request.body, env),
  };
  let effectiveEnv = env;

  let preScriptError: string | undefined;
  if (scripts.pre?.trim()) {
    const pre = executeIpcScript({
      script: scripts.pre,
      context: "pre",
      request: { ...resolved, headers: { ...resolved.headers } },
      envVars: envToMap(effectiveEnv),
    });
    preScriptError = pre.error;
    // The pre-script's request is ephemeral by contract: it changes what this send uses and is never
    // written back to disk. `executeIpcScript` always returns one (it falls back to the input), so the
    // fallback here only covers a shape change in the sandbox, not a failure.
    if (pre.request) resolved = { ...pre.request };
    effectiveEnv = environmentAfterPreScript(effectiveEnv, pre.envVars);
  }

  const sendStart = Date.now();
  const response = await opts.send(resolved, effectiveEnv);
  const durationMs = Date.now() - sendStart;

  const responseBody = b64ToText(response.body);

  let postScriptError: string | undefined;
  if (scripts.post?.trim()) {
    const post = executeIpcScript({
      script: scripts.post,
      context: "post",
      response: { status: response.status, headers: response.headers, body: responseBody },
      envVars: envToMap(effectiveEnv),
    });
    postScriptError = post.error;
  }

  let tests: TestResultEntry[] = [];
  let testLogs: string[] = [];
  if (scripts.test?.trim()) {
    const test = executeIpcScript({
      script: scripts.test,
      context: "test",
      response: {
        status: response.status,
        headers: response.headers,
        body: responseBody,
        responseTime: durationMs,
      },
      envVars: envToMap(effectiveEnv),
    });
    tests = test.testResults ?? [];
    testLogs = test.testLogs ?? [];
  }

  return {
    request: resolved,
    response,
    tests,
    testLogs,
    preScriptError,
    postScriptError,
    durationMs,
  };
}
