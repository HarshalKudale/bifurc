/**
 * Collection Runner engine - executes all requests in a folder sequentially,
 * running pre-script, the actual request, post-script, and test script for each.
 * Returns a structured report of all results.
 */

import { Environment, SavedRequest } from "@/types";
import { runPreScript, runPostScript } from "@/lib/scriptRunner";
import { runTestScript, TestResultEntry } from "@/lib/testRunner";
import { resolveVars, resolveHeaders } from "@/lib/resolveVars";
import { b64ToText, textToB64 } from "@/lib/utils";
import { contentTypeToMode } from "@/lib/bodyUtils";

export interface RunnerRequestResult {
    requestId: string;
    requestName: string;
    method: string;
    url: string;
    status: number | null;
    responseTime: number;
    error?: string;
    tests: TestResultEntry[];
    testLogs: string[];
    preScriptError?: string;
    postScriptError?: string;
}

export interface CollectionRunReport {
    folderId: string;
    folderName: string;
    startedAt: number;
    completedAt: number;
    totalRequests: number;
    totalTests: number;
    passedTests: number;
    failedTests: number;
    results: RunnerRequestResult[];
}

export interface RunnerCallbacks {
    /** Called before each request starts */
    onRequestStart?(index: number, request: SavedRequest): void;
    /** Called after each request completes */
    onRequestDone?(index: number, result: RunnerRequestResult): void;
    /** Check if run should be aborted */
    isCancelled?(): boolean;
}

/**
 * Execute all requests in the given list sequentially.
 */
export async function runCollection(
    requests: SavedRequest[],
    env: Environment | null,
    folderId: string,
    folderName: string,
    callbacks?: RunnerCallbacks,
    delayMs?: number,
): Promise<CollectionRunReport> {
    const startedAt = Date.now();
    const results: RunnerRequestResult[] = [];

    for (let i = 0; i < requests.length; i++) {
        if (callbacks?.isCancelled?.()) break;

        if (i > 0 && delayMs && delayMs > 0) {
            await new Promise((resolve) => setTimeout(resolve, delayMs));
        }

        const req = requests[i];
        callbacks?.onRequestStart?.(i, req);

        const result = await executeOneRequest(req, env);
        results.push(result);
        callbacks?.onRequestDone?.(i, result);
    }

    const completedAt = Date.now();
    const allTests = results.flatMap((r) => r.tests);

    return {
        folderId,
        folderName,
        startedAt,
        completedAt,
        totalRequests: results.length,
        totalTests: allTests.length,
        passedTests: allTests.filter((t) => t.passed).length,
        failedTests: allTests.filter((t) => !t.passed).length,
        results,
    };
}

async function executeOneRequest(
    req: SavedRequest,
    env: Environment | null,
): Promise<RunnerRequestResult> {
    const baseResult: RunnerRequestResult = {
        requestId: req.id,
        requestName: req.name,
        method: req.method,
        url: req.url,
        status: null,
        responseTime: 0,
        tests: [],
        testLogs: [],
    };

    try {
        let finalUrl = resolveVars(req.url, env);
        let finalHeaders = resolveHeaders(req.headers, env);
        let finalBody = resolveVars(req.body, env);
        let scriptEnv = env;

        // Pre-script
        if (req.preScript?.trim()) {
            const pre = await runPreScript(
                req.preScript,
                { method: req.method, url: finalUrl, headers: finalHeaders, body: finalBody },
                env,
            );
            if (pre.error) baseResult.preScriptError = pre.error;
            finalUrl = pre.req.url;
            finalHeaders = pre.req.headers;
            finalBody = pre.req.body;
            if (env && Object.keys(pre.envVars).length > 0) {
                scriptEnv = { ...env, variables: Object.entries(pre.envVars).map(([key, value]) => ({ id: key, key, value })) };
            }
        }

        // Execute request
        const sendStart = Date.now();
        const res = await window.api.replayRequest(req.method, finalUrl, finalHeaders, textToB64(finalBody));
        const responseTime = Date.now() - sendStart;

        baseResult.status = res.status;
        baseResult.responseTime = responseTime;
        baseResult.url = finalUrl;

        const resBody = b64ToText(res.body);

        // Post-script
        if (req.postScript?.trim()) {
            const post = await runPostScript(
                req.postScript,
                { status: res.status, headers: res.headers, body: resBody },
                scriptEnv,
            );
            if (post.error) baseResult.postScriptError = post.error;
        }

        // Test script
        if (req.testScript?.trim()) {
            const testResult = await runTestScript(
                req.testScript,
                { status: res.status, headers: res.headers, body: resBody, responseTime },
                scriptEnv,
            );
            baseResult.tests = testResult.tests;
            baseResult.testLogs = testResult.logs;
        }
    } catch (e) {
        baseResult.error = e instanceof Error ? e.message : String(e);
    }

    return baseResult;
}
