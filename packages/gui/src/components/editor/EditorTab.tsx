import React from "react";
import { Environment } from "@/types";
import { KVRow } from "@/lib/utils";
import { BodyMode } from "@/lib/bodyUtils";

import { SplitPane, useSplitOrientation, type SplitOrientation } from "./TabPage";
import RequestPane from "./RequestPane";
import ResponsePane from "./ResponsePane";

/**
 * Re-exported so the module's public shape is unchanged. The type now lives in `TabPage.tsx`,
 * beside the `SplitPane` and the toggle that consume it — one orientation vocabulary for all five
 * tab pages rather than one per protocol.
 */
export type { SplitOrientation };

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

  /** Current request/response split orientation (persisted). */
  splitOrientation?: SplitOrientation;
  /** Flips the split between side-by-side and stacked. */
  onToggleSplitOrientation?: () => void;
}

export interface EditorTabProps extends RequestPaneProps, ResponsePaneProps {
  mode: EditorMode;
  activeEnv?: Environment | null;
}

/**
 * The REST editor's two panes, in the shared split.
 *
 * This component used to own the orientation itself, with its own `usePersistedState` call and its
 * own `PanelGroup`. Both now come from `TabPage.tsx`, which is what lets the GraphQL, gRPC and SOAP
 * pages offer the same toggle without each growing a second copy of the resize handle's classes.
 *
 * `mode === "mock"` still returns the response pane alone — a mock has no request to send, so there
 * is no second pane and nothing to orient. It is deliberately *not* wrapped in a `SplitPane`; a
 * single-panel `PanelGroup` would add a resize handle for a divider that cannot move.
 */
export default function EditorTab(props: EditorTabProps) {
  const { mode, activeEnv = null } = props;
  const [splitOrientation, toggleSplitOrientation] = useSplitOrientation();

  if (mode === "mock") {
    return (
      <div className="flex flex-col flex-1 min-h-0 overflow-hidden">
        <ResponsePane {...props} activeEnv={activeEnv} />
      </div>
    );
  }

  return (
    <SplitPane
      orientation={splitOrientation}
      left={<RequestPane {...props} activeEnv={activeEnv} />}
      right={
        <ResponsePane
          {...props}
          activeEnv={activeEnv}
          splitOrientation={splitOrientation}
          onToggleSplitOrientation={toggleSplitOrientation}
        />
      }
    />
  );
}
