import React from "react";
import { ArrowUpRight, Braces, Network, FileCode } from "@/lib/icons";
import { ApiProtocol } from "@/types";

interface Props {
  mode: "request" | "mock";
  onSelect(protocol: ApiProtocol): void;
}

interface ProtocolCard {
  id: ApiProtocol;
  title: string;
  badge: string;
  badgeColor: string;
  badgeBg: string;
  icon: React.ReactNode;
  description: string;
  features: string[];
}

export default function ProtocolSelectorTab({ mode, onSelect }: Props) {
  const cards: ProtocolCard[] = [
    {
      id: "rest",
      title: mode === "request" ? "REST Request" : "REST Mock",
      badge: "REST",
      badgeColor: "var(--c-signal)",
      badgeBg: "oklch(var(--signal) / 0.15)",
      icon: <ArrowUpRight size={22} className="text-signal" />,
      description:
        mode === "request"
          ? "Standard HTTP request supporting GET, POST, PUT, DELETE, PATCH with query params, headers, body, and scripts."
          : "Intercept HTTP requests matching method and path, returning custom response status, headers, latency, and body.",
      features:
        mode === "request"
          ? ["HTTP/1.1 & HTTP/2", "JSON, Form & Raw bodies", "Pre & Post scripts"]
          : ["Method & URL matching", "Regex path support", "SSE & chunked streams"],
    },
    {
      id: "graphql",
      title: mode === "request" ? "GraphQL Operation" : "GraphQL Mock",
      badge: "GQL",
      badgeColor: "#d946ef",
      badgeBg: "rgba(217, 70, 239, 0.15)",
      icon: <Braces size={22} style={{ color: "#d946ef" }} />,
      description:
        mode === "request"
          ? "Execute GraphQL queries and mutations with variables, headers, and automatic schema introspection."
          : "Mock GraphQL queries and mutations by operation name and return simulated JSON payloads.",
      features:
        mode === "request"
          ? ["Query & Mutation editor", "JSON variables", "Schema explorer & introspection"]
          : ["Operation name matching", "Query/Mutation/Subscription", "Configurable response delay"],
    },
    {
      id: "grpc",
      title: mode === "request" ? "gRPC Call" : "gRPC Mock",
      badge: "gRPC",
      badgeColor: "#06b6d4",
      badgeBg: "rgba(6, 182, 212, 0.15)",
      icon: <Network size={22} style={{ color: "#06b6d4" }} />,
      description:
        mode === "request"
          ? "Call remote gRPC services using Protobuf definitions or server reflection with unary and streaming support."
          : "Run a local gRPC server stubbing service methods with mock responses, metadata, and status codes.",
      features:
        mode === "request"
          ? ["Unary & Streaming", ".proto file imports", "Server reflection"]
          : ["Local gRPC mock server", "Custom status codes", "Streaming responses"],
    },
    {
      id: "soap",
      title: mode === "request" ? "SOAP Request" : "SOAP Mock",
      badge: "SOAP",
      badgeColor: "#14b8a6",
      badgeBg: "rgba(20, 184, 166, 0.15)",
      icon: <FileCode size={22} style={{ color: "#14b8a6" }} />,
      description:
        mode === "request"
          ? "Send SOAP envelopes with WSDL operation discovery, SOAPAction headers, and XML response inspection."
          : "Stub SOAP services matching endpoint patterns and SOAPAction headers, returning XML responses.",
      features:
        mode === "request"
          ? ["WSDL import & fetch", "XML envelope builder", "SOAPAction header support"]
          : ["SOAPAction matching", "XML response stubs", "Custom response status"],
    },
  ];

  return (
    <div className="flex-1 overflow-y-auto flex flex-col items-center justify-center p-8 bg-surface">
      <div className="w-full max-w-2xl text-center mb-8">
        <div className="inline-flex items-center gap-2 px-3 py-1 rounded-full text-xs font-semibold bg-signal/10 text-signal border border-signal/30 mb-3">
          {mode === "request" ? "New Request" : "New Mock"}
        </div>
        <h2 className="text-xl font-bold text-foreground mb-1.5">
          Select {mode === "request" ? "Request" : "Mock"} Protocol
        </h2>
        <p className="text-xs text-muted-foreground max-w-md mx-auto">
          Choose the protocol for this {mode}. The relevant editor will load immediately.
          Once saved, the protocol type is fixed and cannot be changed.
        </p>
      </div>

      <div className="grid grid-cols-1 md:grid-cols-2 gap-4 w-full max-w-2xl">
        {cards.map((card) => (
          <button
            key={card.id}
            type="button"
            onClick={() => onSelect(card.id)}
            className="group relative flex flex-col text-left p-5 rounded-xl border border-border bg-card hover:border-signal/60 hover:shadow-lg transition-all duration-150 cursor-pointer focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-signal"
          >
            {/* Top row: Icon + Badge */}
            <div className="flex items-center justify-between mb-3 w-full">
              <div className="w-10 h-10 rounded-lg flex items-center justify-center bg-surface-2 border border-border/80 group-hover:scale-105 transition-transform">
                {card.icon}
              </div>
              <span
                className="text-[10px] font-bold font-mono px-2 py-0.5 rounded"
                style={{ color: card.badgeColor, background: card.badgeBg }}
              >
                {card.badge}
              </span>
            </div>

            {/* Title */}
            <h3 className="text-sm font-semibold text-foreground group-hover:text-signal transition-colors mb-1.5">
              {card.title}
            </h3>

            {/* Description */}
            <p className="text-xs text-muted-foreground leading-relaxed mb-4 flex-1">
              {card.description}
            </p>

            {/* Feature tags */}
            <div className="flex flex-wrap gap-1.5 pt-3 border-t border-border/60">
              {card.features.map((feat) => (
                <span
                  key={feat}
                  className="text-[10px] px-2 py-0.5 rounded bg-surface-2 text-muted-foreground"
                >
                  {feat}
                </span>
              ))}
            </div>
          </button>
        ))}
      </div>
    </div>
  );
}
