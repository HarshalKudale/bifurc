import React from "react";
import CodeEditor from "@/components/common/CodeEditor";
import HeaderTable from "@/components/common/HeaderTable";
import { TabStrip } from "@/components/editor/RequestTab";
import ProtoExplorer from "@/components/grpc/ProtoExplorer";
import { strings } from "@/lib/strings";
import { KVRow } from "@/lib/utils";
import { GrpcTabState } from "./grpcTabReducer";

export type ReqSubTab = "message" | "metadata" | "pre-script" | "post-script" | "proto";

export interface GrpcLeftPaneProps {
  state: GrpcTabState;
  dispatch: any;
  set: (field: keyof GrpcTabState) => (value: any) => void;
  reqTab: ReqSubTab;
  setReqTab: (t: ReqSubTab) => void;
  metaRows: KVRow[];
  setMetaRows: (rows: KVRow[]) => void;
}

export function GrpcLeftPane({
  state, dispatch, set,
  reqTab, setReqTab,
  metaRows, setMetaRows
}: GrpcLeftPaneProps) {
  const reqSubTabs: { id: ReqSubTab; label: string }[] = [
    { id: "message", label: strings.grpc.tabMessage },
    { id: "metadata", label: strings.grpc.tabMetadata },
    { id: "pre-script", label: strings.grpc.tabPreScript },
    { id: "post-script", label: strings.grpc.tabPostScript },
    { id: "proto", label: strings.grpc.tabProto },
  ];

  return (
    <div className="flex flex-col h-full overflow-hidden">
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
    </div>
  );
}
