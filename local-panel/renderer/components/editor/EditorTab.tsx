/**
 * EditorTab - unified tab body used by both RequestEditor and MockEditorModal.
 *
 * mode="request"  Left: editable request  | Right: live response (read-only, populated after Send)
 * mode="mock"     Left: captured request (read-only) | Right: editable mock response
 *
 * The left/right split is 50/50 by default and freely resizable by dragging the divider.
 * All behaviour is driven by props; callers own state, draft-save, etc.
 */

import React, { useCallback, useRef, useState, useEffect } from "react";
import { Group as PanelGroup, Panel, Separator as ResizeHandle } from "react-resizable-panels";
import HeaderTable from "@/components/editor/HeaderTable";
import BodyEditor, { BodyEditorHandle } from "@/components/editor/BodyEditor";
import CodeEditor from "@/components/common/CodeEditor";
import BinaryViewer from "@/components/common/BinaryViewer";
import EnvVarHint from "@/components/editor/EnvVarHint";
import RandomizerHint from "@/components/editor/RandomizerHint";
import { TabStrip } from "@/components/editor/RequestTab";
import { KVRow, headersToRows, statusColor, mkRowId } from "@/lib/utils";
import { BodyMode, isBinaryContentType } from "@/lib/bodyUtils";
import { strings } from "@/lib/strings";
import { Environment } from "@/types";

// -- Status dropdown options (mock only) ------------------------------------

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

// -- Props ------------------------------------------------------------------

export type EditorMode = "request" | "mock";

/** Props relevant to the request (left) pane */
export interface RequestPaneProps {
  reqTab: "params" | "headers" | "body" | "pre-script";
  onReqTabChange(t: "params" | "headers" | "body" | "pre-script"): void;
  reqParams?: KVRow[];
  onReqParamsChange?(rows: KVRow[]): void;
  reqHeaders: KVRow[];
  onReqHeadersChange(rows: KVRow[]): void;
  reqBody: string;
  onReqBodyChange(v: string): void;
  reqMode: BodyMode;
  onReqModeChange?(m: BodyMode): void;
  /** Set true when the left pane is read-only (mock mode captured request) */
  reqReadOnly?: boolean;
  preScript?: string;
  onPreScriptChange?(v: string): void;
}

/** Props relevant to the response (right) pane - varies by mode */
export interface ResponsePaneProps {
  resTab: "body" | "headers" | "post-script" | "tests";
  onResTabChange(t: "body" | "headers" | "post-script" | "tests"): void;

  // mock mode: editable response
  resBody?: string;
  onResBodyChange?(v: string): void;
  resHeaders?: KVRow[];
  onResHeadersChange?(rows: KVRow[]): void;
  resMode?: BodyMode;
  onResModeChange?(m: BodyMode): void;
  resStatus?: number;
  onResStatusChange?(s: number): void;
  resStatusMocked?: boolean;
  onResStatusMockedChange?(mocked: boolean): void;
  /** Response delay in ms (mock mode only) */
  resDelay?: number;
  onResDelayChange?(ms: number): void;
  resDelayMocked?: boolean;
  onResDelayMockedChange?(mocked: boolean): void;
  /** Response body encoding (mock mode): "base64" for binary bodies */
  resBodyEncoding?: "utf8" | "base64";
  resBodyMocked?: boolean;
  onResBodyMockedChange?(mocked: boolean): void;
  /** Streaming mode (mock only) */
  streamingMode?: "none" | "sse" | "chunked";
  onStreamingModeChange?(mode: "none" | "sse" | "chunked"): void;
  streamingChunkDelay?: number;
  onStreamingChunkDelayChange?(ms: number): void;
  streamingChunkSeparator?: string;
  onStreamingChunkSeparatorChange?(sep: string): void;

  // request mode: read-only result populated after Send
  loading?: boolean;
  sendErr?: string | null;
  result?: { status: number; headers: Record<string, string>; body: string } | null;
  resBodyText?: string;
  durationMs?: number | null;
  onCreateMock?(): void;
  postScript?: string;
  onPostScriptChange?(v: string): void;
  scriptErr?: string | null;
  // test script
  testScript?: string;
  onTestScriptChange?(v: string): void;
  testResults?: { name: string; passed: boolean; error?: string; durationMs: number }[];
  testLogs?: string[];
  testRunning?: boolean;
}

export interface EditorTabProps extends RequestPaneProps, ResponsePaneProps {
  mode: EditorMode;
  /** Active environment for variable/env hints */
  activeEnv?: Environment | null;
}

// -- Component --------------------------------------------------------------

export default function EditorTab({
  mode,
  activeEnv = null,
  // request pane
  reqTab, onReqTabChange,
  reqParams = [], onReqParamsChange,
  reqHeaders, onReqHeadersChange,
  reqBody, onReqBodyChange,
  reqMode, onReqModeChange,
  reqReadOnly = false,
  preScript, onPreScriptChange,
  // response pane
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
}: EditorTabProps) {
  const reqParamCount = reqParams.filter((r) => r.enabled && r.key.trim()).length;
  const reqHeaderCount = reqHeaders.filter((r) => r.enabled && r.key.trim()).length;
  const resHeaderCount = (resHeaders ?? []).filter((r) => r.enabled && r.key.trim()).length;

  // Refs for body editors - used to insert tokens at cursor
  const reqBodyRef = useRef<BodyEditorHandle>(null);
  const resBodyRef = useRef<BodyEditorHandle>(null);

  // -- Left pane (Request) --------------------------------------------------

  const preScriptDot = preScript?.trim() ? " ●" : "";

  const leftPane = (
    <div className="flex flex-col h-full overflow-hidden">
      <TabStrip
        tabs={[
          ...(!reqReadOnly ? [{ id: "params" as const, label: `${strings.editor.params}${reqParamCount > 0 ? ` (${reqParamCount})` : ""}` }] : []),
          { id: "headers" as const, label: `${strings.editor.headers}${reqHeaderCount > 0 ? ` (${reqHeaderCount})` : ""}` },
          { id: "body" as const, label: strings.editor.body },
          ...(!reqReadOnly ? [{ id: "pre-script" as const, label: `${strings.editor.preScript}${preScriptDot}` }] : []),
        ]}
        active={reqTab}
        onChange={onReqTabChange}
        suffix={
          reqReadOnly
            ? <span className="px-3 text-[9px] text-muted-foreground italic opacity-60">{strings.editor.readOnly}</span>
            : undefined
        }
      />
      <div className="flex-1 overflow-y-auto min-h-0">
        {reqTab === "params" && !reqReadOnly && (
          <HeaderTable
            rows={reqParams}
            onChange={onReqParamsChange ?? (() => { })}
            emptyMessage={strings.editor.noQueryParams}
          />
        )}
        {reqTab === "headers" && (
          <>
            {!reqReadOnly && (
              <TokenToolbar
                env={activeEnv ?? null}
                onInsert={(token) => {
                  if (!insertAtActiveInput(token)) {
                    onReqHeadersChange(appendTokenToFocusedRow(reqHeaders, token));
                  }
                }}
              />
            )}
            <HeaderTable
              rows={reqHeaders}
              onChange={onReqHeadersChange}
              readOnly={reqReadOnly}
              emptyMessage={reqReadOnly ? strings.common.noHeadersCaptured : undefined}
            />
          </>
        )}
        {reqTab === "body" && (
          <>
            {!reqReadOnly && (
              <TokenToolbar
                env={activeEnv ?? null}
                onInsert={(token) => reqBodyRef.current?.insertAtCursor(token)}
              />
            )}
            <BodyEditor
              ref={reqReadOnly ? undefined : reqBodyRef}
              value={reqBody}
              onChange={onReqBodyChange}
              readOnly={reqReadOnly}
              placeholder={reqReadOnly ? strings.editor.noRequestBodyCaptured : strings.editor.requestBodyOptional}
              mode={reqMode}
              onModeChange={reqReadOnly ? undefined : onReqModeChange}
            />
          </>
        )}
        {reqTab === "pre-script" && !reqReadOnly && (
          <ScriptEditor
            value={preScript ?? ""}
            onChange={onPreScriptChange ?? (() => { })}
            placeholder={strings.editor.scriptPlaceholder}
          />
        )}
      </div>
    </div>
  );

  // -- Right pane (Response) ------------------------------------------------

  let rightPane: React.ReactNode;

  if (mode === "mock") {
    // Editable mock response
    rightPane = (
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
              {/* Streaming mode selector */}
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
    // Read-only live response (request mode)
    const postScriptDot = postScript?.trim() ? " ●" : "";
    const testScriptDot = testScript?.trim() ? " ●" : "";
    const testBadge = testResults && testResults.length > 0
      ? ` (${testResults.filter(t => t.passed).length}/${testResults.length})`
      : "";
    rightPane = (
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

  // -- Layout ---------------------------------------------------------------
  // Mock mode: response pane only (no captured-request left pane needed)
  // Request mode: resizable 50/50 split

  if (mode === "mock") {
    return (
      <div className="flex flex-col flex-1 min-h-0 overflow-hidden">
        {rightPane}
      </div>
    );
  }

  return (
    <PanelGroup orientation="horizontal" className="flex flex-1 min-h-0 overflow-hidden">
      <Panel defaultSize={50} minSize={20} className="flex flex-col overflow-hidden">
        {leftPane}
      </Panel>
      <ResizeHandle className="w-1 bg-border hover:bg-signal/40 active:bg-signal/60 transition-colors cursor-col-resize flex-shrink-0" />
      <Panel defaultSize={50} minSize={20} className="flex flex-col overflow-hidden">
        {rightPane}
      </Panel>
    </PanelGroup>
  );
}

// -- ScriptEditor -------------------------------------------------------------

function ScriptEditor({ value, onChange, placeholder, error }: {
  value: string;
  onChange(v: string): void;
  placeholder?: string;
  error?: string;
}) {
  return (
    <div className="flex flex-col h-full overflow-hidden">
      {error && (
        <div className="px-3 py-1.5 border-b border-destructive/30 bg-destructive/5 flex-shrink-0">
          <span className="text-[11px] text-destructive font-mono">{strings.editor.scriptError}: {error}</span>
        </div>
      )}
      <CodeEditor
        value={value}
        onChange={onChange}
        language="javascript"
        placeholder={placeholder}
        className="flex-1 overflow-hidden"
      />
    </div>
  );
}

// -- TokenToolbar --------------------------------------------------------------

/**
 * A slim toolbar row that renders EnvVarHint + RandomizerHint buttons.
 * Placed above HeaderTable or BodyEditor to let users insert tokens.
 */
function TokenToolbar({ env, onInsert }: { env: Environment | null; onInsert(token: string): void }) {
  return (
    <div className="flex items-center gap-1.5 px-3 py-1.5 border-b border-border/40 bg-background/10 flex-shrink-0 justify-end">
      <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider mr-1">{strings.editor.insert}</span>
      <EnvVarHint env={env} onInsert={onInsert} />
      <RandomizerHint onInsert={onInsert} />
    </div>
  );
}

// -- appendTokenToFocusedRow ---------------------------------------------------

/**
 * Appends a token to the last enabled row's value in the header table.
 * If the table is empty, a new row is created.
 */
function appendTokenToFocusedRow(rows: KVRow[], token: string): KVRow[] {
  if (rows.length === 0) {
    return [{ id: mkRowId(), enabled: true, key: "", value: token }];
  }
  // Find last enabled row
  for (let i = rows.length - 1; i >= 0; i--) {
    if (rows[i].enabled) {
      return rows.map((r, idx) => idx === i ? { ...r, value: r.value + token } : r);
    }
  }
  // All disabled - append to last
  return rows.map((r, idx) => idx === rows.length - 1 ? { ...r, value: r.value + token } : r);
}

// -- insertAtActiveInput -------------------------------------------------------

/**
 * Inserts a token at the cursor position of the currently focused input element.
 * Relies on the hint buttons using onMouseDown + preventDefault to keep focus.
 * Returns true if insertion succeeded, false if no suitable input was focused.
 */
function insertAtActiveInput(token: string): boolean {
  const el = document.activeElement as HTMLInputElement | null;
  if (!el || el.tagName !== "INPUT" || el.readOnly) return false;
  const start = el.selectionStart ?? el.value.length;
  const end = el.selectionEnd ?? el.value.length;
  const newValue = el.value.slice(0, start) + token + el.value.slice(end);
  // Trigger React's synthetic onChange via the native value setter trick
  const nativeSetter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, "value")?.set;
  if (!nativeSetter) return false;
  nativeSetter.call(el, newValue);
  el.dispatchEvent(new Event("input", { bubbles: true }));
  // Restore cursor after state update flush
  const cursorPos = start + token.length;
  setTimeout(() => el.setSelectionRange(cursorPos, cursorPos), 0);
  return true;
}

// -- TestsPanel ----------------------------------------------------------------

function TestsPanel({ testScript, onTestScriptChange, testResults, testLogs, testRunning }: {
  testScript: string;
  onTestScriptChange(v: string): void;
  testResults?: { name: string; passed: boolean; error?: string; durationMs: number }[];
  testLogs?: string[];
  testRunning?: boolean;
}) {
  const [view, setView] = useState<"script" | "results">("script");

  // Auto-switch to results when tests finish running
  useEffect(() => {
    if (testResults && testResults.length > 0 && !testRunning) {
      setView("results");
    }
  }, [testResults, testRunning]);

  const passCount = testResults?.filter(t => t.passed).length ?? 0;
  const failCount = testResults?.filter(t => !t.passed).length ?? 0;

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Sub-tabs: Script / Results */}
      <div className="flex items-center border-b border-border/40 bg-background/30 flex-shrink-0">
        <button
          onClick={() => setView("script")}
          className={`px-3 py-1.5 text-[11px] font-medium cursor-pointer ${view === "script" ? "text-foreground border-b-2 border-signal" : "text-muted-foreground hover:text-foreground"}`}
        >
          {strings.editor.script}
        </button>
        <button
          onClick={() => setView("results")}
          className={`px-3 py-1.5 text-[11px] font-medium cursor-pointer ${view === "results" ? "text-foreground border-b-2 border-signal" : "text-muted-foreground hover:text-foreground"}`}
        >
          {strings.editor.results}
          {testResults && testResults.length > 0 && (
            <span className="ml-1.5 text-[10px]">
              <span className="text-signal">{passCount}</span>
              {failCount > 0 && <span className="text-destructive ml-1">{failCount}</span>}
            </span>
          )}
        </button>
        {testRunning && (
          <span className="ml-2 inline-block w-3 h-3 border-2 border-muted-foreground/30 border-t-signal rounded-full animate-spin" />
        )}
      </div>

      {/* Content */}
      {view === "script" ? (
        <CodeEditor
          value={testScript}
          onChange={onTestScriptChange}
          language="javascript"
          placeholder={`// Write tests using lp.test() and lp.expect()\n// Example:\nlp.test("Status is 200", () => {\n  lp.expect(lp.response.status).to.equal(200);\n});\n\nlp.test("Response has data", () => {\n  const json = lp.response.json();\n  lp.expect(json).to.have.property("data");\n});`}
          className="flex-1 overflow-hidden"
        />
      ) : (
        <div className="flex-1 overflow-y-auto p-3 space-y-1.5">
          {(!testResults || testResults.length === 0) && !testRunning && (
            <div className="flex items-center justify-center h-full text-center">
              <p className="text-xs text-muted-foreground">{strings.editor.noTestResults}</p>
            </div>
          )}
          {testResults && testResults.map((t, i) => (
            <div key={i} className={`flex items-start gap-2 px-2.5 py-1.5 rounded text-xs font-mono ${t.passed ? "bg-signal/5 border border-signal/20" : "bg-destructive/5 border border-destructive/20"}`}>
              <span className={`flex-shrink-0 mt-0.5 text-[10px] font-bold ${t.passed ? "text-signal" : "text-destructive"}`}>
                {t.passed ? strings.editor.pass : strings.editor.fail}
              </span>
              <div className="flex-1 min-w-0">
                <span className="text-foreground">{t.name}</span>
                {t.error && <div className="text-destructive/80 mt-0.5 break-words">{t.error}</div>}
              </div>
              <span className="text-muted-foreground text-[10px] flex-shrink-0">{t.durationMs}ms</span>
            </div>
          ))}
          {testLogs && testLogs.length > 0 && (
            <div className="mt-3 pt-2 border-t border-border/40">
              <div className="text-[10px] text-muted-foreground uppercase tracking-wider mb-1">{strings.editor.console}</div>
              {testLogs.map((log, i) => (
                <div key={i} className="text-[11px] font-mono text-muted-foreground px-2 py-0.5">{log}</div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
