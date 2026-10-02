import { useCallback } from "react";
import { textToB64, b64ToText, tryFormat, rowsToHeaders } from "@/lib/utils";
import { contentTypeToMode, isBinaryContentType } from "@/lib/bodyUtils";
import { strings } from "@/lib/strings";
import { TabState, TabType } from "./restTabReducer";
import { MockRule, RequestSendResult } from "@/types";

/**
 * `savedEntityId` is the saved `requests` entity to run **by id**, or `null` to send what the editor
 * holds — computed by the caller (`RestTab`) through `canSendByEntityId`, which owns the reasoning.
 * Taking the *decision* rather than its inputs keeps this hook free of the tab-state questions the gate
 * asks and leaves exactly one place where "is the id form equivalent?" is answered.
 *
 * ## What this hook no longer does, and why that is the point
 *
 * Until 2026-09-30 it **was** the request pipeline — `resolveVars` → `runPreScript` →
 * `window.api.replayRequest` → `runPostScript` → `runTestScript` — and `collectionRunner.ts:94` held a
 * second copy of the same five steps (`plan/19` §1 rows 3 and 4). D24 makes that one engine function, and
 * `window.api.requestSend` now runs it: this hook assembles the request **unresolved**, sends it, and
 * dispatches what came back.
 *
 * `plan/19` §8 trap 1 is why both halves moved together rather than in two steps — resolving in both
 * places, even briefly, double-resolves a value that itself contains `{{…}}`. So the tokens in
 * `state.url` / `state.reqBody` now reach the engine **verbatim**, and so do the three scripts. That is the
 * whole change: the renderer stopped knowing what `{{host}}` is.
 */
export function useRestActions(
  state: TabState,
  dispatch: any,
  tabType: TabType,
  onCreateMock?: (mock: Partial<MockRule>) => void,
  savedEntityId?: string | null,
) {
  const handleSend = useCallback(async () => {
    if (!state.url.trim()) return;
    dispatch({ type: "SEND_START" });
    try {
      /**
       * **The one branch, and it is now only about *where the request comes from*.** The id form lets the
       * engine read the stored entity; the inline form carries what the editor holds, for a draft or for a
       * tab with unsaved edits. Both run the same pipeline — that is D24 — so there is no second
       * preparation path that has to be argued to stay in step with the first.
       */
      const res: RequestSendResult = savedEntityId
        ? await window.api.requestSend({ id: savedEntityId })
        : await window.api.requestSend({
            request: {
              method: state.method,
              url: state.url.trim(),
              headers: rowsToHeaders(state.reqHeaders),
              body: state.reqBody,
            },
            scripts: {
              pre: state.preScript,
              post: state.postScript,
              test: state.testScript,
            },
          });

      const ct = res.headers["content-type"];
      const resMode = ct ? contentTypeToMode(ct) : state.resMode;
      dispatch({ type: "SEND_SUCCESS", result: res, resMode, durationMs: res.durationMs });

      // The two script errors are the engine's now, and they arrive as **values on a successful send**: a
      // script that threw still produced a request and a response, and this pane has always shown the
      // error beside them. Appended to whatever is already there, which is the previous behaviour.
      const scriptErrs: string[] = [];
      if (res.preScriptError) {
        scriptErrs.push(strings.editor.preScriptError.replace("{error}", res.preScriptError));
      }
      if (res.postScriptError) {
        scriptErrs.push(strings.editor.postScriptError.replace("{error}", res.postScriptError));
      }
      if (scriptErrs.length > 0) {
        const joined = scriptErrs.join("; ");
        dispatch({
          type: "SET_FIELD",
          field: "scriptErr",
          value: state.scriptErr ? `${state.scriptErr}; ${joined}` : joined,
        });
      }

      if (state.testScript.trim()) {
        dispatch({ type: "RUN_TESTS_START" });
        dispatch({ type: "RUN_TESTS_DONE", results: res.tests, logs: res.testLogs });
      }
    } catch (e) {
      dispatch({ type: "SEND_ERROR", error: e instanceof Error ? e.message : strings.editor.requestFailed });
    }
  }, [state.url, state.method, state.reqHeaders, state.reqBody, state.preScript, state.postScript, state.testScript, state.resMode, state.scriptErr, dispatch, savedEntityId]);

  const handleTest = useCallback(async () => {
    if (!state.url.trim()) return;
    dispatch({ type: "TEST_START" });
    try {
      const testMethod = state.method === "*" ? "GET" : state.method;
      /**
       * `scripts: {}` — the Test action sends the request and shows the response; it has never run the
       * three scripts, and the engine is told so rather than left to infer it.
       *
       * Resolution, though, is the engine's here **too**, and that is deliberate: a `{{host}}` in the url
       * must resolve the same way on Test as on Send, and after D23 the only way to guarantee that is for
       * one implementation to do both.
       */
      const res: RequestSendResult = await window.api.requestSend({
        request: {
          method: testMethod,
          url: state.url.trim(),
          headers: rowsToHeaders(state.reqHeaders),
          body: state.reqBody,
        },
        scripts: {},
      });
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
  }, [state.url, state.method, state.reqHeaders, state.reqBody, state.resMode, state.resHeaders, dispatch]);

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
