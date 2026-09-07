import React from "react";
import { RequestLogEntry } from "@/types";

type Via = RequestLogEntry["via"];

export const VIA_LABEL: Record<Via, string> = {
  rfc6761: "RFC 6761",
  proxy: "Proxy",
  rule: "Rule",
  mock: "Mock",
  error: "Error",
};

const VIA_COLOR: Record<Via, string> = {
  rfc6761: "text-signal",
  proxy: "text-signal",
  rule: "text-amber",
  mock: "text-violet",
  error: "text-destructive",
};

export default function ViaBadge({ via }: { via: Via }) {
  if (via === "mock") return <span style={{ color: "var(--c-violet)" }}>{VIA_LABEL.mock}</span>;
  return <span className={VIA_COLOR[via]}>{VIA_LABEL[via]}</span>;
}
