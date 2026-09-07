import React, { useState, useEffect } from 'react';
// -- WsTabHeader - green/red dot based on connection status -----------------

export default function WsTabHeader({ tabId, label }: { tabId: string; label: string }) {
  const [dotColor, setDotColor] = useState("var(--c-muted-foreground)");

  useEffect(() => {
    const handler = (e: CustomEvent<{ tabId: string; color: string }>) => {
      if (e.detail.tabId === tabId) setDotColor(e.detail.color);
    };
    window.addEventListener("ws:statuscolor" as any, handler as any);
    return () => window.removeEventListener("ws:statuscolor" as any, handler as any);
  }, [tabId]);

  return (
    <div className="flex items-center gap-1.5 min-w-0">
      <span style={{ display: "inline-block", width: 6, height: 6, borderRadius: "50%", background: dotColor, flexShrink: 0 }} />
      <span className="truncate">{label}</span>
    </div>
  );
}