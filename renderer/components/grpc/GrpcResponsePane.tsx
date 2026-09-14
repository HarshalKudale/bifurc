import React from "react";
import CodeEditor from "@/components/common/CodeEditor";
import { TabStrip } from "@/components/editor/RequestTab";
import { strings } from "@/lib/strings";
import { cn } from "@/components/ui/cn";
import { GrpcTabState } from "./grpcTabReducer";

export type ResSubTab = "response" | "res-metadata";

export interface GrpcResponsePaneProps {
  state: GrpcTabState;
  resTab: ResSubTab;
  onResTabChange: (tab: ResSubTab) => void;
}

export function GrpcResponsePane({ state, resTab, onResTabChange }: GrpcResponsePaneProps) {
  const resSubTabs: { id: ResSubTab; label: string }[] = [
    { id: "response", label: strings.grpc.tabResponse },
    { id: "res-metadata", label: strings.grpc.tabTrailingMetadata },
  ];

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Status bar */}
      {state.resStatus !== null && (
        <div className="flex items-center gap-3 px-4 py-2 border-b border-border bg-background/30 flex-shrink-0">
          <span className={cn(
            "text-xs font-semibold",
            state.resStatus === 0 ? "text-signal" : "text-destructive"
          )}>
            {strings.grpc.status} {state.resStatus}
          </span>
          {state.resStatusMessage && (
            <span className="text-xs text-muted-foreground">{state.resStatusMessage}</span>
          )}
          {state.resDuration !== null && (
            <span className="text-xs text-muted-foreground ml-auto">{state.resDuration}ms</span>
          )}
        </div>
      )}
      {state.resError && (
        <div className="px-4 py-2 border-b border-border bg-destructive/5 flex-shrink-0">
          <p className="text-xs text-destructive">{state.resError}</p>
        </div>
      )}

      <TabStrip tabs={resSubTabs} active={resTab} onChange={(t) => onResTabChange(t as ResSubTab)} />
      <div className="flex-1 overflow-hidden">
        {resTab === "response" && (
          state.responses.length === 0 ? (
            <div className="flex items-center justify-center h-full text-xs text-muted-foreground">
              {state.sending ? strings.grpc.sending : strings.grpc.noResponseYet}
            </div>
          ) : state.responses.length === 1 ? (
            <CodeEditor value={state.responses[0]} language="json" readOnly className="h-full" />
          ) : (
            <div className="flex flex-col h-full overflow-y-auto p-2 gap-1">
              {state.responses.map((r, i) => (
                <div key={i} className="border border-border rounded p-2">
                  <div className="text-[10px] text-muted-foreground font-semibold mb-1">{strings.grpc.responseNumber.replace("{n}", String(i + 1))}</div>
                  <pre className="text-xs text-foreground font-mono whitespace-pre-wrap break-all">{r}</pre>
                </div>
              ))}
            </div>
          )
        )}
        {resTab === "res-metadata" && (
          <div className="p-4 overflow-y-auto">
            {Object.keys(state.resMetadata).length === 0 ? (
              <p className="text-xs text-muted-foreground italic">{strings.grpc.noTrailingMetadata}</p>
            ) : (
              <table className="w-full text-xs">
                <tbody>
                  {Object.entries(state.resMetadata).map(([k, v]) => (
                    <tr key={k} className="border-b border-border/30">
                      <td className="py-1.5 pr-4 text-signal font-mono">{k}</td>
                      <td className="py-1.5 text-foreground font-mono">{v}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </div>
        )}
      </div>
    </div>
  );
}
