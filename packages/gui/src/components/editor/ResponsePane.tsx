import React, { useRef } from "react";
import HeaderTable from "@/components/common/HeaderTable";
import BodyEditor, { BodyEditorHandle } from "@/components/common/BodyEditor";
import BinaryViewer from "@/components/common/BinaryViewer";
import { TabStrip } from "@/components/editor/RequestTab";
import { strings } from "@/lib/strings";
import { Environment } from "@/types";
import { statusColor, headersToRows } from "@/lib/utils";
import { isBinaryContentType } from "@/lib/bodyUtils";
import { ResponsePaneProps, EditorMode } from "./EditorTab";
import ScriptEditor from "./ScriptEditor";
import TestsPanel from "./TestsPanel";
import { TokenToolbar, appendTokenToFocusedRow, insertAtActiveInput } from "./TokenToolbar";

interface ResponsePaneComponentProps extends ResponsePaneProps {
  mode: EditorMode;
  activeEnv: Environment | null;
}

export default function ResponsePane({
  mode, activeEnv,
  resTab, onResTabChange,
  resBody, onResBodyChange,
  resHeaders, onResHeadersChange,
  resMode, onResModeChange,
  resStatus, onResStatusChange, resStatusMocked, onResStatusMockedChange,
  resDelay, onResDelayChange, resDelayMocked, onResDelayMockedChange,
  resBodyEncoding,
  resBodyMocked, onResBodyMockedChange,
  streamingMode, onStreamingModeChange,
  streamingChunkDelay, onStreamingChunkDelayChange,
  streamingChunkSeparator, onStreamingChunkSeparatorChange,
  loading, sendErr, result, resBodyText, durationMs, onCreateMock,
  postScript, onPostScriptChange,
  scriptErr,
  testScript, onTestScriptChange,
  testResults, testLogs, testRunning,
}: ResponsePaneComponentProps) {
  const resHeaderCount = (resHeaders ?? []).filter((r) => r.enabled && r.key.trim()).length;
  const resBodyRef = useRef<BodyEditorHandle>(null);

  if (mode === "mock") {
    return (
      <div className="flex flex-col h-full overflow-hidden">
        <TabStrip
          tabs={[
            { id: "body" as const, label: strings.editor.response },
            { id: "headers" as const, label: `${strings.editor.headers}${resHeaderCount > 0 ? ` (${resHeaderCount})` : ""}` },
          ]}
          active={resTab}
          onChange={onResTabChange}
          suffix={
            <div className="flex items-center gap-2 pr-3">
              <span className="text-[10px] text-muted-foreground flex-shrink-0">{strings.editor.delay}</span>
              <label className="flex items-center gap-1 text-[10px] text-muted-foreground cursor-pointer">
                <input
                  type="checkbox"
                  checked={resDelayMocked ?? true}
                  onChange={(e) => onResDelayMockedChange?.(e.target.checked)}
                  className="accent-signal"
                />
                <span>Mock</span>
              </label>
              <input
                type="number"
                min={0}
                step={100}
                value={resDelay ?? 0}
                onChange={(e) => onResDelayChange?.(Math.max(0, parseInt(e.target.value, 10) || 0))}
                className="bg-card border border-border rounded px-2 py-1 text-xs font-mono text-foreground outline-none focus:border-signal w-16 text-center"
                placeholder="0"
                title={strings.editor.responseDelayTitle}
              />
              <span className="text-[10px] text-muted-foreground flex-shrink-0">{strings.editor.ms}</span>
              <select
                value={streamingMode ?? "none"}
                onChange={(e) => onStreamingModeChange?.(e.target.value as "none" | "sse" | "chunked")}
                className="bg-card border border-border rounded px-1.5 py-1 text-[10px] font-mono text-foreground outline-none focus:border-signal cursor-pointer"
                title={strings.editor.streamingModeTitle}
              >
                <option value="none">{strings.editor.streamNone}</option>
                <option value="sse">{strings.editor.streamSse}</option>
                <option value="chunked">{strings.editor.streamChunked}</option>
              </select>
              {streamingMode && streamingMode !== "none" && (
                <>
                  <input
                    type="number"
                    min={10}
                    step={50}
                    value={streamingChunkDelay ?? 100}
                    onChange={(e) => onStreamingChunkDelayChange?.(Math.max(10, parseInt(e.target.value, 10) || 100))}
                    className="bg-card border border-border rounded px-2 py-1 text-xs font-mono text-foreground outline-none focus:border-signal w-14 text-center"
                    title={strings.editor.chunkDelayTitle}
                  />
                  <span className="text-[10px] text-muted-foreground flex-shrink-0">{strings.editor.msPerChunk}</span>
                </>
              )}
              <label className="flex items-center gap-1 text-[10px] text-muted-foreground cursor-pointer">
                <input
                  type="checkbox"
                  checked={resStatusMocked ?? true}
                  onChange={(e) => onResStatusMockedChange?.(e.target.checked)}
                  className="accent-signal"
                />
                <span>Mock</span>
              </label>
              <input
                type="number"
                min={100}
                max={599}
                value={resStatus ?? 200}
                onChange={(e) => onResStatusChange?.(parseInt(e.target.value, 10) || 200)}
                className="bg-card border border-border rounded px-2 py-1 text-sm font-bold font-mono outline-none focus:border-signal w-16 text-center"
                style={{ color: statusColor(resStatus ?? 200) }}
                title={strings.editor.responseStatusTitle}
              />
            </div>
          }
        />
        <div className="flex-1 overflow-y-auto min-h-0">
          {resTab === "body" && (
            <>
              {(resMode !== "binary" && resMode !== "image") && (
                <div>
                  <div className="flex items-center justify-between px-3 py-1.5 border-b border-border/20 bg-background/20">
                    <label className="flex items-center gap-2 text-[10px] text-muted-foreground cursor-pointer">
                      <input
                        type="checkbox"
                        checked={resBodyMocked ?? true}
                        onChange={(e) => onResBodyMockedChange?.(e.target.checked)}
                        className="accent-signal"
                      />
                      <span className="font-semibold uppercase tracking-wider">Mock body</span>
                    </label>
                  </div>
                  <TokenToolbar
                    env={activeEnv ?? null}
                    onInsert={(token) => resBodyRef.current?.insertAtCursor(token)}
                  />
                </div>
              )}
              <BodyEditor
                ref={resBodyRef}
                value={resBody ?? ""}
                onChange={onResBodyChange ?? (() => { })}
                placeholder='{"mocked": true}'
                mode={resMode ?? "json"}
                onModeChange={onResModeChange}
                isBase64={resBodyEncoding === "base64"}
                contentType={
                  (resHeaders ?? []).find(r => r.key.toLowerCase() === "content-type")?.value ?? undefined
                }
              />
            </>
          )}
          {resTab === "headers" && (
            <>
              <TokenToolbar
                env={activeEnv ?? null}
                onInsert={(token) => {
                  if (!insertAtActiveInput(token)) {
                    onResHeadersChange?.(appendTokenToFocusedRow(resHeaders ?? [], token));
                  }
                }}
              />
              <HeaderTable
                rows={resHeaders ?? []}
                onChange={onResHeadersChange ?? (() => { })}
                mockControls={{
                  allMocked: (resHeaders ?? []).filter((row) => row.key.trim()).length > 0 && (resHeaders ?? []).filter((row) => row.key.trim()).every((row) => !!row.mocked),
                  anyMocked: (resHeaders ?? []).some((row) => !!row.mocked),
                  onToggleAll: (mocked) => {
                    onResHeadersChange?.((resHeaders ?? []).map((row) => ({ ...row, mocked })));
                  },
                }}
              />
            </>
          )}
        </div>
      </div>
    );
  } else {
    const postScriptDot = postScript?.trim() ? " ●" : "";
    const testScriptDot = testScript?.trim() ? " ●" : "";
    const testBadge = testResults && testResults.length > 0
      ? ` (${testResults.filter(t => t.passed).length}/${testResults.length})`
      : "";

    return (
      <div className="flex flex-col h-full overflow-hidden">
        <TabStrip
          tabs={[
            { id: "body" as const, label: strings.editor.response },
            { id: "headers" as const, label: strings.editor.headers },
            { id: "post-script" as const, label: `${strings.editor.postScript}${postScriptDot}` },
            { id: "tests" as const, label: `${strings.editor.tests}${testScriptDot}${testBadge}` },
          ]}
          active={resTab}
          onChange={onResTabChange}
          suffix={
            <div className="flex items-center gap-2 pr-3">
              {result && (
                <span className="text-sm font-bold font-mono" style={{ color: statusColor(result.status) }}>
                  {result.status}
                </span>
              )}
              {result && durationMs != null && (
                <span className="text-xs font-mono text-muted-foreground">
                  {durationMs}ms
                </span>
              )}
            </div>
          }
        />
        <div className="flex-1 overflow-y-auto min-h-0">
          {resTab === "post-script" ? (
            <ScriptEditor
              value={postScript ?? ""}
              onChange={onPostScriptChange ?? (() => { })}
              placeholder={strings.editor.postScriptPlaceholder}
              error={scriptErr ?? undefined}
            />
          ) : resTab === "tests" ? (
            <TestsPanel
              testScript={testScript ?? ""}
              onTestScriptChange={onTestScriptChange ?? (() => { })}
              testResults={testResults}
              testLogs={testLogs}
              testRunning={testRunning}
            />
          ) : (
            <>
              {loading && (
                <div className="flex items-center justify-center h-full">
                  <div className="flex flex-col items-center gap-2 text-muted-foreground">
                    <span className="inline-block w-5 h-5 border-2 border-muted-foreground/30 border-t-signal rounded-full animate-spin" />
                    <span className="text-xs">{strings.server.sending}</span>
                  </div>
                </div>
              )}
              {!loading && sendErr && (
                <div className="p-4">
                  <div className="px-3 py-2 rounded border border-destructive/30 bg-destructive/5 text-xs text-destructive font-mono">{sendErr}</div>
                </div>
              )}
              {!loading && !sendErr && !result && (
                <div className="flex items-center justify-center h-full text-center">
                  <div>
                    <div className="text-3xl opacity-20 mb-2">→</div>
                    <p className="text-xs text-muted-foreground">{strings.requests.hitSendPrompt}</p>
                  </div>
                </div>
              )}
              {result && !loading && (
                <>
                  {resTab === "body" && (
                    (() => {
                      const resCt = (result.headers["content-type"] ?? "").toLowerCase();
                      if (isBinaryContentType(resCt) && result.body) {
                        return <BinaryViewer data={result.body} contentType={resCt.split(";")[0].trim()} />;
                      }
                      return (
                        <BodyEditor
                          value={resBodyText ?? ""}
                          placeholder={strings.common.emptyBody}
                          readOnly
                          mode={resMode ?? "json"}
                        />
                      );
                    })()
                  )}
                  {resTab === "headers" && (
                    <HeaderTable rows={headersToRows(result.headers)} onChange={() => { }} readOnly />
                  )}
                </>
              )}
            </>
          )}
        </div>
      </div>
    );
  }
}
