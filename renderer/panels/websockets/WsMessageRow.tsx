import React from 'react';
import { WsMessage } from '@/hooks/useWebSocket';
import CodeEditor from '@/components/common/CodeEditor';
import { tryFormat } from '@/lib/utils';

// -- Message row ------------------------------------------------------------

function looksLikeJson(s: string): boolean {
  const t = s.trimStart();
  return (t.startsWith("{") || t.startsWith("[")) && t.length > 10;
}

export default function MessageRow({ msg }: { msg: WsMessage }) {
  const time = new Date(msg.ts).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit", second: "2-digit" });
  const useEditor = looksLikeJson(msg.data) || msg.data.length > 200;
  const displayData = looksLikeJson(msg.data) ? tryFormat(msg.data) : msg.data;
  return (
    <div className="flex items-start gap-2 py-1 border-b border-border/30 last:border-0 group">
      <span className="text-[9px] font-mono text-muted-foreground flex-shrink-0 mt-0.5 w-16">{time}</span>
      {useEditor
        ? <div className="flex-1 overflow-hidden border border-border/30 rounded" style={{ maxHeight: 192 }}>
          <CodeEditor
            value={displayData}
            readOnly
            language={looksLikeJson(msg.data) ? "json" : "text"}
            className="h-full"
          />
        </div>
        : <pre className="text-[11px] font-mono text-foreground flex-1 whitespace-pre-wrap break-all leading-relaxed">{msg.data}</pre>
      }
    </div>
  );
}