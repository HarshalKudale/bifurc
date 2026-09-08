import React from 'react';
import { Group as PanelGroup, Panel, Separator as PanelResizeHandle } from 'react-resizable-panels';
import { TabStrip } from '@/components/editor/RequestTab';
import HeaderTable from '@/components/common/HeaderTable';
import EnvVarHint from '@/components/editor/EnvVarHint';
import RandomizerHint from '@/components/editor/RandomizerHint';
import { Send, Radio } from '@/lib/icons';
import { strings } from '@/lib/strings';
import { KVRow, mkRowId } from '@/lib/utils';
import { Environment } from '@/types';
import WsMessageRow from './WsMessageRow';
import { WsMessage } from '@/hooks/useWebSocket';

interface WsStreamsProps {
  reqTab: "headers";
  setReqTab: (v: "headers") => void;
  headerCount: number;
  headers: KVRow[];
  setHeaders: React.Dispatch<React.SetStateAction<KVRow[]>>;
  activeEnv?: Environment | null;
  isConnected: boolean;
  isConnecting: boolean;
  outgoingMessages: WsMessage[];
  outgoingInput: string;
  setOutgoingInput: React.Dispatch<React.SetStateAction<string>>;
  handleSend: () => void;
  sendErr: string | null;
  outEndRef: React.RefObject<HTMLDivElement>;
  incomingMessages: WsMessage[];
  clearMessages: () => void;
  inEndRef: React.RefObject<HTMLDivElement>;
}

export default function WsStreams({
  reqTab, setReqTab, headerCount, headers, setHeaders, activeEnv,
  isConnected, isConnecting, outgoingMessages, outgoingInput,
  setOutgoingInput, handleSend, sendErr, outEndRef,
  incomingMessages, clearMessages, inEndRef
}: WsStreamsProps) {
  return (
    <PanelGroup orientation="horizontal" className="flex flex-1 min-h-0 overflow-hidden">
      {/* -- OutputStream (left) ------------------------------------------- */}
      <Panel defaultSize={50} minSize={20} className="flex flex-col overflow-hidden">
        <div className="flex flex-col h-full overflow-hidden">
          <TabStrip
            tabs={[
              { id: "headers" as const, label: `Headers${headerCount > 0 ? ` (${headerCount})` : ""}` },
            ]}
            active={reqTab}
            onChange={(t) => setReqTab(t as "headers")}
            prefix={
              <span className="px-4 py-2.5 text-[10px] font-semibold uppercase tracking-widest text-muted-foreground border-r border-border whitespace-nowrap">
                {strings.sockets.outputStream}
              </span>
            }
          />

          <div className="flex-1 overflow-y-auto min-h-0">
            {!(isConnected || isConnecting) && (
              <div className="flex items-center gap-1.5 px-3 py-1.5 border-b border-border/40 bg-background/10 flex-shrink-0 justify-end">
                <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider mr-1">Insert</span>
                <EnvVarHint
                  env={activeEnv}
                  onInsert={(token) => {
                    setHeaders((prev) => {
                      if (prev.length === 0) return [{ id: mkRowId(), enabled: true, key: "", value: token }];
                      return prev.map((r, i) => i === prev.length - 1 ? { ...r, value: r.value + token } : r);
                    });
                  }}
                />
                <RandomizerHint
                  onInsert={(token) => {
                    setHeaders((prev) => {
                      if (prev.length === 0) return [{ id: mkRowId(), enabled: true, key: "", value: token }];
                      return prev.map((r, i) => i === prev.length - 1 ? { ...r, value: r.value + token } : r);
                    });
                  }}
                />
              </div>
            )}
            <HeaderTable
              rows={headers}
              onChange={setHeaders}
              readOnly={isConnected || isConnecting}
              emptyMessage={isConnected || isConnecting ? strings.common.noHeaders : undefined}
            />
          </div>

          {/* Send message section */}
          <div className="border-t border-border flex-shrink-0">
            {/* Sent messages list */}
            <div className="max-h-40 overflow-y-auto px-4 py-2 space-y-1">
              {outgoingMessages.length === 0 ? (
                <p className="text-[10px] text-muted-foreground italic py-1">{strings.sockets.noMessagesSent}</p>
              ) : (
                outgoingMessages.map((m) => (
                  <WsMessageRow key={m.id} msg={m} />
                ))
              )}
              <div ref={outEndRef} />
            </div>

            {/* Token hints for message input */}
            {isConnected && (
              <div className="flex items-center gap-1.5 px-3 py-1 border-t border-border/40 bg-background/10 justify-end">
                <span className="text-[9px] text-muted-foreground/60 uppercase tracking-wider mr-1">Insert</span>
                <EnvVarHint
                  env={activeEnv}
                  onInsert={(token) => setOutgoingInput((v) => v + token)}
                />
                <RandomizerHint
                  onInsert={(token) => setOutgoingInput((v) => v + token)}
                />
              </div>
            )}

            {/* Input row */}
            <div className="flex items-center gap-2 px-4 py-2.5 border-t border-border">
              <input
                className="flex-1 bg-card border border-border focus:border-signal rounded px-3 py-2 text-xs font-mono text-foreground outline-none placeholder:text-muted-foreground/60 transition-colors"
                placeholder={isConnected ? strings.sockets.typeMessage : strings.sockets.connectToSend}
                value={outgoingInput}
                onChange={(e) => setOutgoingInput(e.target.value)}
                onKeyDown={(e) => { if (e.key === "Enter") handleSend(); }}
                disabled={!isConnected}
              />
              <button
                onClick={handleSend}
                disabled={!isConnected || !outgoingInput.trim()}
                className="px-4 py-2 rounded bg-signal hover:bg-signal/80 disabled:opacity-40 disabled:cursor-not-allowed text-background text-xs font-semibold transition-all cursor-pointer flex-shrink-0"
              >
                <Send size={12} />
              </button>
            </div>
            {sendErr && (
              <div className="px-4 pb-2">
                <span className="text-[10px] text-destructive font-mono">{sendErr}</span>
              </div>
            )}
          </div>
        </div>
      </Panel>

      <PanelResizeHandle className="w-1 bg-border hover:bg-signal/40 active:bg-signal/60 transition-colors cursor-col-resize flex-shrink-0" />

      {/* -- InputStream (right) ------------------------------------------- */}
      <Panel defaultSize={50} minSize={20} className="flex flex-col overflow-hidden">
        <div className="flex flex-col h-full overflow-hidden">
          <TabStrip
            tabs={[{ id: "inputstream" as const, label: "InputStream" }]}
            active="inputstream"
            onChange={() => { }}
            prefix={
              <span className="px-4 py-2.5 text-[10px] font-semibold uppercase tracking-widest text-muted-foreground border-r border-border whitespace-nowrap">
                {isConnected
                  ? <span className="flex items-center gap-1.5">
                    <span style={{ display: "inline-block", width: 6, height: 6, borderRadius: "50%", background: "var(--c-signal)", boxShadow: "0 0 5px var(--c-signal)" }} />
                    {strings.sockets.live}
                  </span>
                  : strings.sockets.waiting}
              </span>
            }
            suffix={
              incomingMessages.length > 0
                ? <button
                  onClick={clearMessages}
                  className="px-3 text-[10px] text-muted-foreground hover:text-foreground cursor-pointer transition-colors"
                  title="Clear all messages"
                >{strings.sockets.clear}</button>
                : undefined
            }
          />
          <div className="flex-1 overflow-y-auto min-h-0 px-4 py-2 space-y-1">
            {incomingMessages.length === 0 ? (
              <div className="flex flex-col items-center justify-center h-full text-center gap-2 py-8">
                <div className="opacity-15"><Radio size={28} /></div>
                <p className="text-xs text-muted-foreground">
                  {isConnected ? strings.sockets.waitingForMessages : strings.sockets.connectToReceive}
                </p>
              </div>
            ) : (
              incomingMessages.map((m) => (
                <WsMessageRow key={m.id} msg={m} />
              ))
            )}
            <div ref={inEndRef} />
          </div>
        </div>
      </Panel>
    </PanelGroup>
  );
}
