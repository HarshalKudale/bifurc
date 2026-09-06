import { useCallback } from "react";
import { resolveVars, resolveHeaders } from "@/lib/resolveVars";
import { textToB64, b64ToText, tryFormat, rowsToHeaders } from "@/lib/utils";
import { runPreScript, runPostScript } from "@/lib/scriptRunner";
import { runTestScript } from "@/lib/testRunner";
import { contentTypeToMode, isBinaryContentType } from "@/lib/bodyUtils";
import { strings } from "@/lib/strings";
import { TabState, TabType } from "./restTabReducer";
import { ReplayResult, Environment, MockRule } from "@/types";

export function useRestActions(state: TabState, dispatch: any, activeEnv: Environment | null, tabType: TabType, onCreateMock?: (mock: Partial<MockRule>) => void) {
  const handleSend = useCallback(async () => {
    if (!state.url.trim()) return;
    dispatch({ type: "SEND_START" });
    try {
      const resolvedUrl = resolveVars(state.url.trim(), activeEnv);
      const resolvedHdr = resolveHeaders(rowsToHeaders(state.reqHeaders), activeEnv);
      const resolvedBod = resolveVars(state.reqBody, activeEnv);

      let finalUrl = resolvedUrl;
      let finalHeaders = resolvedHdr;
      let finalBody = resolvedBod;
      let scriptEnv = activeEnv;

      if (state.preScript.trim()) {
        const pre = await runPreScript(state.preScript, { method: state.method, url: finalUrl, headers: finalHeaders, body: finalBody }, activeEnv);
        if (pre.error) dispatch({ type: "SET_FIELD", field: "scriptErr", value: strings.editor.preScriptError.replace("{error}", pre.error) });
        finalUrl = pre.req.url;
        finalHeaders = pre.req.headers;
        finalBody = pre.req.body;
        if (activeEnv && Object.keys(pre.envVars).length > 0) {
          scriptEnv = { ...activeEnv, variables: Object.entries(pre.envVars).map(([key, value]) => ({ id: key, key, value })) };
        }
      }

      const sendStart = Date.now();
      const res: ReplayResult = await window.api.replayRequest(state.method, finalUrl, finalHeaders, textToB64(finalBody));
      const responseTime = Date.now() - sendStart;
      const ct = res.headers["content-type"];
      const resMode = ct ? contentTypeToMode(ct) : state.resMode;
      dispatch({ type: "SEND_SUCCESS", result: res, resMode, durationMs: responseTime });

      if (state.postScript.trim()) {
        const post = await runPostScript(
          state.postScript,
          { status: res.status, headers: res.headers, body: b64ToText(res.body) },
          scriptEnv,
        );
        if (post.error) {
          const existing = state.scriptErr;
          const postErr = strings.editor.postScriptError.replace("{error}", post.error);
          dispatch({ type: "SET_FIELD", field: "scriptErr", value: existing ? `${existing}; ${postErr}` : postErr });
        }
      }

      if (state.testScript.trim()) {
        dispatch({ type: "RUN_TESTS_START" });
        const testResult = await runTestScript(
          state.testScript,
          { status: res.status, headers: res.headers, body: b64ToText(res.body), responseTime },
          scriptEnv,
        );
        dispatch({ type: "RUN_TESTS_DONE", results: testResult.tests, logs: testResult.logs });
      }
    } catch (e) {
      dispatch({ type: "SEND_ERROR", error: e instanceof Error ? e.message : strings.editor.requestFailed });
    }
  }, [state.url, state.method, state.reqHeaders, state.reqBody, state.preScript, state.postScript, state.testScript, state.resMode, state.scriptErr, activeEnv, dispatch]);

  const handleTest = useCallback(async () => {
    if (!state.url.trim()) return;
    dispatch({ type: "TEST_START" });
    try {
      const testMethod = state.method === "*" ? "GET" : state.method;
      const resolvedUrl = resolveVars(state.url.trim(), activeEnv);
      const resolvedHdr = resolveHeaders(rowsToHeaders(state.reqHeaders), activeEnv);
      const resolvedBod = resolveVars(state.reqBody, activeEnv);
      const bodyB64 = resolvedBod.trim() ? textToB64(resolvedBod) : "";
      const res: ReplayResult = await window.api.replayRequest(testMethod, resolvedUrl, resolvedHdr, bodyB64);
      const ct = res.headers["content-type"];
      const resMode = ct ? contentTypeToMode(ct) : state.resMode;
      const { headersToRows } = await import("@/lib/utils");
      const isBinaryRes = ct ? isBinaryContentType(ct) : false;
      const resBody = isBinaryRes
        ? res.body
        : (resMode === "json" ? tryFormat(b64ToText(res.body)) : b64ToText(res.body));
      dispatch({
        type: "TEST_SUCCESS",
        resStatus: res.status,
        resHeaders: Object.keys(res.headers).length > 0 ? headersToRows(res.headers) : state.resHeaders,
        resBody,
        resMode,
        resBodyEncoding: isBinaryRes ? "base64" : "utf8",
      });
    } catch (e) {
      dispatch({ type: "TEST_ERROR", error: e instanceof Error ? e.message : strings.editor.requestFailed });
    }
  }, [state.url, state.method, state.reqHeaders, state.reqBody, state.resMode, state.resHeaders, activeEnv, dispatch]);

  const handleCreateMock = useCallback(() => {
    if (!onCreateMock) return;
    const { result, resMode } = state;
    const resCt = result?.headers?.["content-type"] ?? "";
    const isBinaryRes = isBinaryContentType(resCt);
    onCreateMock({
      name: state.name.trim() || "",
      method: state.method,
      urlPattern: state.url.trim(),
      useRegex: false,
      capturedHeaders: rowsToHeaders(state.reqHeaders),
      capturedBody: textToB64(state.reqBody),
      responseStatus: result?.status ?? 200,
      responseStatusMocked: true,
      responseHeaders: result?.headers ?? {},
      mockedResponseHeaders: [],
      responseBody: isBinaryRes
        ? (result?.body ?? "")
        : (result ? (resMode === "json" ? tryFormat(b64ToText(result.body)) : b64ToText(result.body)) : "{}"),
      responseBodyMocked: true,
      responseBodyEncoding: isBinaryRes ? "base64" : undefined,
      responseDelayMocked: true,
    });
  }, [state, onCreateMock]);

  return { handleSend, handleTest, handleCreateMock };
}
