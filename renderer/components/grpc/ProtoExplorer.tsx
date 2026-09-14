import React, { useState, useCallback, useEffect } from "react";
import { SavedProtoFile } from "@/types";
import { cn } from "@/components/ui/cn";
import { strings } from "@/lib/strings";

// -- Types ------------------------------------------------------------------

interface ProtoService {
    name: string;
    methods: ProtoMethod[];
}

interface ProtoMethod {
    name: string;
    inputType: string;
    outputType: string;
    clientStreaming: boolean;
    serverStreaming: boolean;
}

interface ProtoExplorerProps {
    protoFileId: string | null;
    onSelectMethod?: (serviceName: string, methodName: string, streamingType: string, skeleton: string) => void;
    onProtoChange?: (protoId: string) => void;
}

// -- Helpers: Parse proto file to extract services --------------------------

function parseProtoServices(content: string): ProtoService[] {
    const services: ProtoService[] = [];
    const serviceRegex = /service\s+(\w+)\s*\{([^}]*(?:\{[^}]*\}[^}]*)*)\}/g;
    let serviceMatch;

    while ((serviceMatch = serviceRegex.exec(content)) !== null) {
        const serviceName = serviceMatch[1];
        const serviceBody = serviceMatch[2];
        const methods: ProtoMethod[] = [];

        const rpcRegex = /rpc\s+(\w+)\s*\(\s*(stream\s+)?(\w+)\s*\)\s*returns\s*\(\s*(stream\s+)?(\w+)\s*\)/g;
        let rpcMatch;
        while ((rpcMatch = rpcRegex.exec(serviceBody)) !== null) {
            methods.push({
                name: rpcMatch[1],
                inputType: rpcMatch[3],
                outputType: rpcMatch[5],
                clientStreaming: !!rpcMatch[2],
                serverStreaming: !!rpcMatch[4],
            });
        }

        services.push({ name: serviceName, methods });
    }
    return services;
}

function parseMessageFields(content: string, messageName: string): { name: string; type: string; repeated: boolean }[] {
    const msgRegex = new RegExp(`message\\s+${messageName}\\s*\\{([^}]*)\\}`, "m");
    const match = msgRegex.exec(content);
    if (!match) return [];

    const fields: { name: string; type: string; repeated: boolean }[] = [];
    const fieldRegex = /^\s*(repeated\s+)?(\w+)\s+(\w+)\s*=\s*\d+/gm;
    let fieldMatch;
    while ((fieldMatch = fieldRegex.exec(match[1])) !== null) {
        fields.push({
            name: fieldMatch[3],
            type: fieldMatch[2],
            repeated: !!fieldMatch[1],
        });
    }
    return fields;
}

function generateJsonSkeleton(content: string, messageName: string, depth: number = 0): Record<string, any> {
    if (depth > 3) return {};
    const fields = parseMessageFields(content, messageName);
    const obj: Record<string, any> = {};

    for (const field of fields) {
        let value: any;
        switch (field.type) {
            case "string": value = ""; break;
            case "int32": case "int64": case "uint32": case "uint64":
            case "sint32": case "sint64": case "fixed32": case "fixed64":
            case "sfixed32": case "sfixed64": value = 0; break;
            case "float": case "double": value = 0.0; break;
            case "bool": value = false; break;
            case "bytes": value = ""; break;
            default:
                // Nested message type
                value = generateJsonSkeleton(content, field.type, depth + 1);
                break;
        }
        obj[field.name] = field.repeated ? [value] : value;
    }
    return obj;
}

function getStreamingType(method: ProtoMethod): "unary" | "server" | "client" | "bidi" {
    if (method.clientStreaming && method.serverStreaming) return "bidi";
    if (method.clientStreaming) return "client";
    if (method.serverStreaming) return "server";
    return "unary";
}

const STREAMING_LABELS: Record<string, { label: string; color: string }> = {
    unary: { label: strings.grpc.streamUnary, color: "text-muted-foreground" },
    server: { label: strings.grpc.streamServer, color: "text-amber" },
    client: { label: strings.grpc.streamClient, color: "text-signal" },
    bidi: { label: strings.grpc.streamBidi, color: "text-signal" },
};

// -- Component --------------------------------------------------------------

export default function ProtoExplorer({ protoFileId, onSelectMethod, onProtoChange }: ProtoExplorerProps) {
    const [protos, setProtos] = useState<SavedProtoFile[]>([]);
    const [selectedProto, setSelectedProto] = useState<SavedProtoFile | null>(null);
    const [services, setServices] = useState<ProtoService[]>([]);
    const [filter, setFilter] = useState("");
    const [mode, setMode] = useState<"list" | "import">("list");
    const [reflectAddress, setReflectAddress] = useState("");
    const [loading, setLoading] = useState(false);
    const [error, setError] = useState<string | null>(null);

    // Load protos list
    useEffect(() => {
        window.api.listProtoFiles().then(setProtos).catch(() => { });
    }, []);

    // Select proto by ID
    useEffect(() => {
        if (protoFileId) {
            const p = protos.find((p) => p.id === protoFileId);
            if (p) {
                setSelectedProto(p);
                // Use cached services if available, otherwise parse
                if (p.parsedServices) {
                    setServices(p.parsedServices);
                } else {
                    setServices(parseProtoServices(p.content));
                }
            }
        }
    }, [protoFileId, protos]);

    const handleImportFile = useCallback(async () => {
        const result = await window.api.openFileDialog();
        if (!result || "error" in result) return;
        const content = atob(result.base64);
        const name = result.name.replace(/\.proto$/, "");
        const parsedServices = parseProtoServices(content);
        const saved = await window.api.addProtoFile({ name, content, parsedServices });
        setProtos((prev) => [...prev, saved]);
        setSelectedProto(saved);
        setServices(parsedServices);
        onProtoChange?.(saved.id);
        setMode("list");
    }, [onProtoChange]);

    const handleReflect = useCallback(async () => {
        if (!reflectAddress.trim()) return;
        setLoading(true);
        setError(null);
        try {
            const res = await window.api.grpcReflect(reflectAddress);
            if (res.ok && res.services) {
                setServices(res.services);
                setError(null);
            } else {
                setError(res.error ?? strings.grpc.reflectionFailed);
            }
        } catch (err: any) {
            setError(err.message ?? strings.grpc.connectionFailed);
        } finally {
            setLoading(false);
        }
    }, [reflectAddress]);

    const handleSelectMethod = useCallback((service: ProtoService, method: ProtoMethod) => {
        const streamingType = getStreamingType(method);
        let skeleton = "{}";
        if (selectedProto) {
            const fields = generateJsonSkeleton(selectedProto.content, method.inputType);
            skeleton = JSON.stringify(fields, null, 2);
        }
        onSelectMethod?.(service.name, method.name, streamingType, skeleton);
    }, [selectedProto, onSelectMethod]);

    const filteredServices = services.map((s) => ({
        ...s,
        methods: s.methods.filter(
            (m) => !filter || m.name.toLowerCase().includes(filter.toLowerCase()) || s.name.toLowerCase().includes(filter.toLowerCase()),
        ),
    })).filter((s) => s.methods.length > 0);

    return (
        <div className="flex flex-col h-full overflow-hidden bg-background">
            {/* Header */}
            <div className="flex items-center gap-2 px-3 py-2 border-b border-border flex-shrink-0">
                <span className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider flex-1">{strings.grpc.proto}</span>
                <button
                    onClick={() => setMode(mode === "import" ? "list" : "import")}
                    className="text-[10px] text-signal hover:text-signal/80 cursor-pointer"
                >
                    {mode === "import" ? strings.grpc.back : strings.grpc.import}
                </button>
            </div>

            {/* Import mode */}
            {mode === "import" && (
                <div className="p-3 flex flex-col gap-2 border-b border-border">
                    <button
                        onClick={handleImportFile}
                        className="px-3 py-1.5 rounded bg-signal text-background text-xs font-semibold cursor-pointer"
                    >
                        {strings.grpc.importProtoFile}
                    </button>
                    <div className="flex items-center gap-2">
                        <div className="flex-1 h-px bg-border" />
                        <span className="text-[9px] text-muted-foreground">{strings.grpc.or}</span>
                        <div className="flex-1 h-px bg-border" />
                    </div>
                    <input
                        className="bg-card border border-border rounded px-2.5 py-1.5 text-xs font-mono text-foreground outline-none focus:border-signal placeholder:text-muted-foreground"
                        placeholder="localhost:50051"
                        value={reflectAddress}
                        onChange={(e) => setReflectAddress(e.target.value)}
                        onKeyDown={(e) => { if (e.key === "Enter") handleReflect(); }}
                    />
                    <button
                        onClick={handleReflect}
                        disabled={loading || !reflectAddress.trim()}
                        className="px-3 py-1.5 rounded bg-surface-2 border border-border text-foreground text-xs font-semibold disabled:opacity-40 cursor-pointer"
                    >
                        {loading ? strings.grpc.reflecting : strings.grpc.serverReflection}
                    </button>
                    {error && <span className="text-xs text-destructive">{error}</span>}
                </div>
            )}

            {/* Proto selector */}
            {mode === "list" && protos.length > 0 && (
                <div className="px-3 py-2 border-b border-border">
                    <select
                        value={selectedProto?.id ?? ""}
                        onChange={(e) => {
                            const p = protos.find((p) => p.id === e.target.value);
                            if (p) {
                                setSelectedProto(p);
                                setServices(p.parsedServices ?? parseProtoServices(p.content));
                                onProtoChange?.(p.id);
                            }
                        }}
                        className="w-full bg-card border border-border rounded px-2.5 py-1.5 text-xs text-foreground outline-none focus:border-signal"
                    >
                        <option value="">{strings.grpc.selectProtoFile}</option>
                        {protos.map((p) => (
                            <option key={p.id} value={p.id}>{p.name}</option>
                        ))}
                    </select>
                </div>
            )}

            {/* Filter */}
            {services.length > 0 && (
                <div className="px-3 py-2 border-b border-border">
                    <input
                        className="w-full bg-card border border-border rounded px-2.5 py-1.5 text-xs text-foreground outline-none focus:border-signal placeholder:text-muted-foreground"
                        placeholder={strings.grpc.filterMethods}
                        value={filter}
                        onChange={(e) => setFilter(e.target.value)}
                    />
                </div>
            )}

            {/* Services/Methods list */}
            <div className="flex-1 overflow-y-auto">
                {services.length === 0 && protos.length === 0 && mode === "list" && (
                    <div className="p-4 text-xs text-muted-foreground text-center">
                        {strings.grpc.noProtoFiles}
                    </div>
                )}
                {filteredServices.map((service) => (
                    <div key={service.name}>
                        <div className="px-3 py-1.5 text-[9px] font-bold uppercase tracking-widest text-muted-foreground bg-surface border-b border-border sticky top-0">
                            {service.name}
                        </div>
                        {service.methods.map((method) => {
                            const st = getStreamingType(method);
                            const badge = STREAMING_LABELS[st];
                            return (
                                <button
                                    key={`${service.name}.${method.name}`}
                                    onClick={() => handleSelectMethod(service, method)}
                                    className="w-full text-left px-3 py-2 hover:bg-card transition-colors cursor-pointer border-b border-border/50"
                                    title={strings.grpc.selectMethod.replace("{service}", service.name).replace("{method}", method.name)}
                                >
                                    <div className="flex items-center gap-2">
                                        <span className="text-xs text-foreground font-mono truncate flex-1">{method.name}</span>
                                        <span className={cn("text-[9px] font-semibold", badge.color)}>{badge.label}</span>
                                    </div>
                                    <div className="text-[10px] text-muted-foreground truncate mt-0.5">
                                        {method.inputType} → {method.outputType}
                                    </div>
                                </button>
                            );
                        })}
                    </div>
                ))}
            </div>
        </div>
    );
}
