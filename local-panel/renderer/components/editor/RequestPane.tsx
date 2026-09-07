import React, { useRef } from "react";
import HeaderTable from "@/components/common/HeaderTable";
import BodyEditor, { BodyEditorHandle } from "@/components/common/BodyEditor";
import { TabStrip } from "@/components/editor/RequestTab";
import { strings } from "@/lib/strings";
import { Environment } from "@/types";
import { RequestPaneProps } from "./EditorTab";
import ScriptEditor from "./ScriptEditor";
import { TokenToolbar, appendTokenToFocusedRow, insertAtActiveInput } from "./TokenToolbar";

interface RequestPaneComponentProps extends RequestPaneProps {
  activeEnv: Environment | null;
}

export default function RequestPane({
  reqTab, onReqTabChange,
  reqParams = [], onReqParamsChange,
  reqHeaders, onReqHeadersChange,
  reqBody, onReqBodyChange,
  reqMode, onReqModeChange,
  reqReadOnly = false,
  preScript, onPreScriptChange,
  activeEnv
}: RequestPaneComponentProps) {
  const reqParamCount = reqParams.filter((r) => r.enabled && r.key.trim()).length;
  const reqHeaderCount = reqHeaders.filter((r) => r.enabled && r.key.trim()).length;
  const reqBodyRef = useRef<BodyEditorHandle>(null);

  const preScriptDot = preScript?.trim() ? " ●" : "";

  return (
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
}
