import React from "react";
import EditorTab from "@/components/editor/EditorTab";
import { TabState, TabType } from "./restTabReducer";

export interface RestEditorPaneProps {
  tabType: TabType;
  state: TabState;
  dispatch: any;
  activeEnv: any;
  resBodyText: string;
}

export function RestEditorPane({ tabType, state, dispatch, activeEnv, resBodyText }: RestEditorPaneProps) {
  return (
    <EditorTab
      mode={tabType === "request" ? "request" : "mock"}
      activeEnv={activeEnv}
      reqTab={state.reqTab as "params" | "headers" | "body" | "pre-script"}
      onReqTabChange={(v) => dispatch({ type: "SET_FIELD", field: "reqTab", value: v })}
      reqParams={tabType === "request" ? state.reqParams : undefined}
      onReqParamsChange={tabType === "request" ? (rows) => dispatch({ type: "SET_PARAMS", params: rows }) : undefined}
      reqHeaders={state.reqHeaders}
      onReqHeadersChange={(rows) => dispatch({ type: "SET_HEADERS", target: "req", rows })}
      reqBody={state.reqBody}
      onReqBodyChange={(v) => dispatch({ type: "SET_FIELD", field: "reqBody", value: v })}
      reqMode={state.reqMode}
      onReqModeChange={(m) => dispatch({ type: "SET_REQ_MODE", mode: m })}
      reqReadOnly={tabType === "mock"}
      preScript={tabType === "request" ? state.preScript : undefined}
      onPreScriptChange={tabType === "request" ? (v) => dispatch({ type: "SET_FIELD", field: "preScript", value: v }) : undefined}
      resTab={state.resTab as "body" | "headers" | "post-script" | "tests"}
      onResTabChange={(v) => dispatch({ type: "SET_FIELD", field: "resTab", value: v })}
      resMode={state.resMode}
      loading={tabType === "request" ? state.loading : undefined}
      sendErr={tabType === "request" ? state.sendErr : undefined}
      result={tabType === "request" ? state.result : undefined}
      resBodyText={tabType === "request" ? resBodyText : undefined}
      durationMs={tabType === "request" ? state.durationMs : undefined}
      postScript={tabType === "request" ? state.postScript : undefined}
      onPostScriptChange={tabType === "request" ? (v) => dispatch({ type: "SET_FIELD", field: "postScript", value: v }) : undefined}
      scriptErr={tabType === "request" ? state.scriptErr : undefined}
      testScript={tabType === "request" ? state.testScript : undefined}
      onTestScriptChange={tabType === "request" ? (v) => dispatch({ type: "SET_FIELD", field: "testScript", value: v }) : undefined}
      testResults={tabType === "request" ? state.testResults : undefined}
      testLogs={tabType === "request" ? state.testLogs : undefined}
      testRunning={tabType === "request" ? state.testRunning : undefined}
      resBody={tabType === "mock" ? state.resBody : undefined}
      onResBodyChange={tabType === "mock" ? (v) => dispatch({ type: "SET_FIELD", field: "resBody", value: v }) : undefined}
      resHeaders={tabType === "mock" ? state.resHeaders : undefined}
      onResHeadersChange={tabType === "mock" ? (rows) => dispatch({ type: "SET_HEADERS", target: "res", rows }) : undefined}
      onResModeChange={tabType === "mock" ? (m) => dispatch({ type: "SET_RES_MODE", mode: m }) : undefined}
      resStatus={tabType === "mock" ? state.resStatus : undefined}
      onResStatusChange={tabType === "mock" ? (s) => dispatch({ type: "SET_FIELD", field: "resStatus", value: s }) : undefined}
      resStatusMocked={tabType === "mock" ? state.resStatusMocked : undefined}
      onResStatusMockedChange={tabType === "mock" ? (mocked) => dispatch({ type: "SET_FIELD", field: "resStatusMocked", value: mocked }) : undefined}
      resDelay={tabType === "mock" ? state.resDelay : undefined}
      onResDelayChange={tabType === "mock" ? (ms) => dispatch({ type: "SET_FIELD", field: "resDelay", value: ms }) : undefined}
      resDelayMocked={tabType === "mock" ? state.resDelayMocked : undefined}
      onResDelayMockedChange={tabType === "mock" ? (mocked) => dispatch({ type: "SET_FIELD", field: "resDelayMocked", value: mocked }) : undefined}
      resBodyEncoding={tabType === "mock" ? state.resBodyEncoding : undefined}
      resBodyMocked={tabType === "mock" ? state.resBodyMocked : undefined}
      onResBodyMockedChange={tabType === "mock" ? (mocked) => dispatch({ type: "SET_FIELD", field: "resBodyMocked", value: mocked }) : undefined}
      streamingMode={tabType === "mock" ? state.streamingMode : undefined}
      onStreamingModeChange={tabType === "mock" ? (m) => dispatch({ type: "SET_FIELD", field: "streamingMode", value: m }) : undefined}
      streamingChunkDelay={tabType === "mock" ? state.streamingChunkDelay : undefined}
      onStreamingChunkDelayChange={tabType === "mock" ? (ms) => dispatch({ type: "SET_FIELD", field: "streamingChunkDelay", value: ms }) : undefined}
      streamingChunkSeparator={tabType === "mock" ? state.streamingChunkSeparator : undefined}
      onStreamingChunkSeparatorChange={tabType === "mock" ? (sep) => dispatch({ type: "SET_FIELD", field: "streamingChunkSeparator", value: sep }) : undefined}
    />
  );
}
