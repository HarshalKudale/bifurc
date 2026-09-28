import React from "react";
import CodeEditor from "@/components/common/CodeEditor";
import { TabStrip } from "@/components/editor/RequestTab";
import { SplitOrientationToggle, type SplitOrientation } from "@/components/editor/TabPage";
import { strings } from "@/lib/strings";
import { cn } from "@/components/ui/cn";
import { GrpcTabState } from "./grpcTabReducer";

export type ResSubTab = "response" | "res-metadata";

export interface GrpcResponsePaneProps {
    state: GrpcTabState;
    resTab: ResSubTab;
    onResTabChange: (tab: ResSubTab) => void;
    /** Current request/response split orientation (shared, persisted). */
    splitOrientation?: SplitOrientation;
    /** Flips the split between side-by-side and stacked. */
    onToggleSplitOrientation?: () => void;
}

export function GrpcResponsePane({
    state,
    resTab,
    onResTabChange,
    splitOrientation,
    onToggleSplitOrientation,
}: GrpcResponsePaneProps) {
    const resSubTabs: { id: ResSubTab; label: string }[] = [
        { id: "response", label: strings.grpc.tabResponse },
        { id: "res-metadata", label: strings.grpc.tabTrailingMetadata },
    ];

    return (
        <div className="flex flex-col h-full overflow-hidden">
            {/*
                The status used to sit in a bar of its own **above** this strip, which was a third
                position for the same information across the four protocols (REST: strip suffix,
                GraphQL: bespoke header row, gRPC: bar above, SOAP: strip prefix). It is now in the
                suffix, beside the layout toggle.

                ⚠ Deliberately **not** the shared `ResponseStatus`. A gRPC status is not an HTTP
                status code, and `statusColor()` — which the shared component uses — maps anything
                below 300 to the success colour. gRPC `14` is UNAVAILABLE, and would have been drawn
                in green. The container's classes match the shared suffix so the two are visually
                identical; only the number's semantics differ.
            */}
            <TabStrip
                tabs={resSubTabs}
                active={resTab}
                onChange={(t) => onResTabChange(t as ResSubTab)}
                suffix={
                    <div className="flex items-center gap-2 pr-3">
                        {splitOrientation && onToggleSplitOrientation && (
                            <SplitOrientationToggle
                                orientation={splitOrientation}
                                onToggle={onToggleSplitOrientation}
                            />
                        )}
                        {state.resStatus !== null && (
                            <>
                                <span className={cn(
                                    "text-xs font-semibold",
                                    state.resStatus === 0 ? "text-success" : "text-destructive"
                                )}>
                                    {strings.grpc.status} {state.resStatus}
                                </span>
                                {state.resStatusMessage && (
                                    <span className="text-xs text-muted-foreground">{state.resStatusMessage}</span>
                                )}
                                {state.resDuration !== null && (
                                    <span className="text-xs font-mono text-muted-foreground">{state.resDuration}ms</span>
                                )}
                            </>
                        )}
                    </div>
                }
            />
            {state.resError && (
                <div className="px-4 py-2 border-b border-border bg-destructive/5 flex-shrink-0">
                    <p className="text-xs text-destructive">{state.resError}</p>
                </div>
            )}

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
