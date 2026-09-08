import React from "react";
import CodeEditor from "@/components/common/CodeEditor";
import HeaderTable from "@/components/common/HeaderTable";
import { TabStrip } from "@/components/editor/RequestTab";
import ProtoExplorer from "@/components/grpc/ProtoExplorer";
import { strings } from "@/lib/strings";
import { KVRow } from "@/lib/utils";
import { GrpcTabState, GrpcTabType } from "./grpcTabReducer";

export type ReqSubTab = "message" | "metadata" | "pre-script" | "post-script" | "proto";
export type MockSubTab = "response" | "metadata" | "settings" | "proto";

export interface GrpcLeftPaneProps {
  tabType: GrpcTabType;
  state: GrpcTabState;
  dispatch: any;
  set: (field: keyof GrpcTabState) => (value: any) => void;
  reqTab: ReqSubTab;
  setReqTab: (t: ReqSubTab) => void;
  mockTab: MockSubTab;
  setMockTab: (t: MockSubTab) => void;
  metaRows: KVRow[];
  setMetaRows: (rows: KVRow[]) => void;
  resMetaRows: KVRow[];
  setResMetaRows: (rows: KVRow[]) => void;
}

export function GrpcLeftPane({
  tabType, state, dispatch, set,
  reqTab, setReqTab, mockTab, setMockTab,
  metaRows, setMetaRows, resMetaRows, setResMetaRows
}: GrpcLeftPaneProps) {
  const reqSubTabs: { id: ReqSubTab; label: string }[] = tabType === "request"
    ? [{ id: "message", label: strings.grpc.tabMessage }, { id: "metadata", label: strings.grpc.tabMetadata }, { id: "pre-script", label: strings.grpc.tabPreScript }, { id: "post-script", label: strings.grpc.tabPostScript }, { id: "proto", label: strings.grpc.tabProto }]
    : [{ id: "message", label: strings.grpc.tabMessage }, { id: "metadata", label: strings.grpc.tabMetadata }, { id: "proto", label: strings.grpc.tabProto }];

  const mockSubTabs: { id: MockSubTab; label: string }[] = [
    { id: "response", label: strings.grpc.tabResponseBody },
    { id: "metadata", label: strings.grpc.tabResponseMetadata },
    { id: "settings", label: strings.grpc.tabSettings },
    { id: "proto", label: strings.grpc.tabProto },
  ];

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {tabType === "request" ? (
        <>
          <TabStrip tabs={reqSubTabs} active={reqTab} onChange={(t) => setReqTab(t as ReqSubTab)} />
          <div className="flex-1 overflow-hidden">
            {reqTab === "message" && (
              <CodeEditor value={state.requestBody} onChange={(v) => set("requestBody")(v)} language="json" placeholder='{"key": "value"}' className="h-full" />
            )}
            {reqTab === "metadata" && (
              <HeaderTable rows={metaRows} onChange={setMetaRows} emptyMessage={strings.grpc.noMetadata} />
            )}
            {reqTab === "pre-script" && (
              <CodeEditor value={state.preScript} onChange={(v) => set("preScript")(v)} language="javascript" placeholder="// Pre-request script" className="h-full" />
            )}
            {reqTab === "post-script" && (
              <CodeEditor value={state.postScript} onChange={(v) => set("postScript")(v)} language="javascript" placeholder="// Post-response script" className="h-full" />
            )}
            {reqTab === "proto" && (
              <ProtoExplorer
                protoFileId={state.protoFileId}
                onSelectMethod={(serviceName, methodName, streamingType, skeleton) => {
                  dispatch({ type: "SET_FIELD", field: "serviceName", value: serviceName });
                  dispatch({ type: "SET_FIELD", field: "methodName", value: methodName });
                  dispatch({ type: "SET_FIELD", field: "streamingType", value: streamingType });
                  if (skeleton && skeleton !== "{}") dispatch({ type: "SET_FIELD", field: "requestBody", value: skeleton });
                }}
                onProtoChange={(id) => dispatch({ type: "SET_FIELD", field: "protoFileId", value: id })}
              />
            )}
          </div>
        </>
      ) : (
        <>
          <TabStrip tabs={mockSubTabs} active={mockTab} onChange={(t) => setMockTab(t as MockSubTab)} />
          <div className="flex-1 overflow-hidden">
            {mockTab === "response" && (
              <CodeEditor value={state.responseBody} onChange={(v) => set("responseBody")(v)} language="json" placeholder='{"result": "mocked"}' className="h-full" />
            )}
            {mockTab === "metadata" && (
              <HeaderTable rows={resMetaRows} onChange={setResMetaRows} emptyMessage={strings.grpc.noResponseMetadata} />
            )}
            {mockTab === "settings" && (
              <div className="p-4 space-y-4 overflow-y-auto">
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">{strings.grpc.responseDelay}</label>
                  <input
                    type="number"
                    min={0}
                    className="w-32 bg-card border border-border focus:border-signal rounded px-3 py-1.5 text-sm text-foreground outline-none"
                    value={state.responseDelay}
                    onChange={(e) => set("responseDelay")(Number(e.target.value))}
                  />
                </div>
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">{strings.grpc.errorCode}</label>
                  <input
                    type="number"
                    min={0}
                    max={16}
                    className="w-32 bg-card border border-border focus:border-signal rounded px-3 py-1.5 text-sm text-foreground outline-none"
                    value={state.errorCode}
                    onChange={(e) => set("errorCode")(Number(e.target.value))}
                  />
                </div>
                <div className="space-y-1">
                  <label className="text-xs font-medium text-muted-foreground">{strings.grpc.errorMessage}</label>
                  <input
                    className="w-full bg-card border border-border focus:border-signal rounded px-3 py-1.5 text-sm text-foreground outline-none placeholder:text-muted-foreground"
                    placeholder={strings.grpc.errorMessagePlaceholder}
                    value={state.errorMessage}
                    onChange={(e) => set("errorMessage")(e.target.value)}
                  />
                </div>
              </div>
            )}
            {mockTab === "proto" && (
              <ProtoExplorer
                protoFileId={state.protoFileId}
                onSelectMethod={(serviceName, methodName, streamingType, skeleton) => {
                  dispatch({ type: "SET_FIELD", field: "serviceName", value: serviceName });
                  dispatch({ type: "SET_FIELD", field: "methodName", value: methodName });
                  dispatch({ type: "SET_FIELD", field: "streamingType", value: streamingType });
                  if (skeleton && skeleton !== "{}") dispatch({ type: "SET_FIELD", field: "responseBody", value: skeleton });
                }}
                onProtoChange={(id) => dispatch({ type: "SET_FIELD", field: "protoFileId", value: id })}
              />
            )}
          </div>
        </>
      )}
    </div>
  );
}
