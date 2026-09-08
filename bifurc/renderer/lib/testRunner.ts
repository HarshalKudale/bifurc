/**
 * Test script execution via the main-process vm sandbox.
 * Calls IPC with context: "test" and returns structured test results.
 */

import { Environment } from "@/types";

export interface TestResultEntry {
    name: string;
    passed: boolean;
    error?: string;
    durationMs: number;
}

export interface RunTestScriptResult {
    tests: TestResultEntry[];
    logs: string[];
    error?: string;
}

function envToMap(env: Environment | null): Record<string, string> {
    const map: Record<string, string> = {};
    if (!env) return map;
    for (const v of env.variables) map[v.key] = v.value;
    return map;
}

/**
 * Run the test script for a request via the main-process vm sandbox.
 * The response is read-only; tests register via lp.test() and assert via lp.expect().
 */
export async function runTestScript(
    script: string,
    response: { status: number; headers: Record<string, string>; body: string; responseTime?: number },
    env: Environment | null,
): Promise<RunTestScriptResult> {
    const vars = envToMap(env);
    try {
        const result = await window.api.executeScript({
            script,
            context: "test",
            response,
            envVars: vars,
        });
        if (result.error && (!result.testResults || result.testResults.length === 0)) {
            return { tests: [], logs: result.testLogs ?? [], error: result.error };
        }
        return {
            tests: result.testResults ?? [],
            logs: result.testLogs ?? [],
        };
    } catch (e) {
        return { tests: [], logs: [], error: e instanceof Error ? e.message : String(e) };
    }
}
