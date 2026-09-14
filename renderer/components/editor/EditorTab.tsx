import React from "react";
import { Group as PanelGroup, Panel, Separator as ResizeHandle } from "react-resizable-panels";
import { Environment } from "@/types";
import { strings } from "@/lib/strings";
import { KVRow } from "@/lib/utils";
import { BodyMode } from "@/lib/bodyUtils";

import RequestPane from "./RequestPane";
import ResponsePane from "./ResponsePane";

export type EditorMode = "request" | "mock";

export interface RequestPaneProps {
  reqTab: "params" | "headers" | "body" | "pre-script";
  onReqTabChange: (t: "params" | "headers" | "body" | "pre-script") => void;
  reqParams?: KVRow[];
  onReqParamsChange?: (rows: KVRow[]) => void;
  reqHeaders: KVRow[];
  onReqHeadersChange: (rows: KVRow[]) => void;
  reqBody: string;
  onReqBodyChange: (v: string) => void;
  reqMode: BodyMode;
  onReqModeChange?: (m: BodyMode) => void;
  reqReadOnly?: boolean;
  preScript?: string;
  onPreScriptChange?: (v: string) => void;
}

export interface ResponsePaneProps {
  resTab: "body" | "headers" | "post-script" | "tests";
  onResTabChange: (t: "body" | "headers" | "post-script" | "tests") => void;

  resBody?: string;
  onResBodyChange?: (v: string) => void;
  resHeaders?: KVRow[];
  onResHeadersChange?: (rows: KVRow[]) => void;
  resMode?: BodyMode;
  onResModeChange?: (m: BodyMode) => void;
  resStatus?: number;
  onResStatusChange?: (s: number) => void;
  resStatusMocked?: boolean;
  onResStatusMockedChange?: (mocked: boolean) => void;
  resDelay?: number;
  onResDelayChange?: (ms: number) => void;
  resDelayMocked?: boolean;
  onResDelayMockedChange?: (mocked: boolean) => void;
  resBodyEncoding?: "utf8" | "base64";
  resBodyMocked?: boolean;
  onResBodyMockedChange?: (mocked: boolean) => void;
  streamingMode?: "none" | "sse" | "chunked";
  onStreamingModeChange?: (mode: "none" | "sse" | "chunked") => void;
  streamingChunkDelay?: number;
  onStreamingChunkDelayChange?: (ms: number) => void;
  streamingChunkSeparator?: string;
  onStreamingChunkSeparatorChange?: (sep: string) => void;

  loading?: boolean;
  sendErr?: string | null;
  result?: { status: number; headers: Record<string, string>; body: string } | null;
  resBodyText?: string;
  durationMs?: number | null;
  onCreateMock?: () => void;
  postScript?: string;
  onPostScriptChange?: (v: string) => void;
  scriptErr?: string | null;
  testScript?: string;
  onTestScriptChange?: (v: string) => void;
  testResults?: { name: string; passed: boolean; error?: string; durationMs: number }[];
  testLogs?: string[];
  testRunning?: boolean;
}

export interface EditorTabProps extends RequestPaneProps, ResponsePaneProps {
  mode: EditorMode;
  activeEnv?: Environment | null;
}

const STATUS_OPTIONS = [
  { v: 200, label: strings.editor.status200 },
  { v: 201, label: strings.editor.status201 },
  { v: 204, label: strings.editor.status204 },
  { v: 301, label: strings.editor.status301 },
  { v: 302, label: strings.editor.status302 },
  { v: 304, label: strings.editor.status304 },
  { v: 400, label: strings.editor.status400 },
  { v: 401, label: strings.editor.status401 },
  { v: 403, label: strings.editor.status403 },
  { v: 404, label: strings.editor.status404 },
  { v: 405, label: strings.editor.status405 },
  { v: 409, label: strings.editor.status409 },
  { v: 422, label: strings.editor.status422 },
  { v: 429, label: strings.editor.status429 },
  { v: 500, label: strings.editor.status500 },
  { v: 502, label: strings.editor.status502 },
  { v: 503, label: strings.editor.status503 },
];

export default function EditorTab(props: EditorTabProps) {
  const { mode, activeEnv = null } = props;

  if (mode === "mock") {
    return (
      <div className="flex flex-col flex-1 min-h-0 overflow-hidden">
        <ResponsePane {...props} activeEnv={activeEnv} />
      </div>
    );
  }

  return (
    <PanelGroup orientation="horizontal" className="flex flex-1 min-h-0 overflow-hidden">
      <Panel defaultSize={50} minSize={20} className="flex flex-col overflow-hidden">
        <RequestPane {...props} activeEnv={activeEnv} />
      </Panel>
      <ResizeHandle className="w-1 bg-border hover:bg-signal/40 active:bg-signal/60 transition-colors cursor-col-resize flex-shrink-0" />
      <Panel defaultSize={50} minSize={20} className="flex flex-col overflow-hidden">
        <ResponsePane {...props} activeEnv={activeEnv} />
      </Panel>
    </PanelGroup>
  );
}
