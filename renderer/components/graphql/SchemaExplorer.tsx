import React, { useState, useCallback, useEffect } from "react";
import { SavedGraphQLSchema } from "@/types";
import CodeEditor from "@/components/common/CodeEditor";
import { cn } from "@/components/ui/cn";
import { strings } from "@/lib/strings";

// -- Types ------------------------------------------------------------------

interface ParsedOperation {
    name: string;
    type: "query" | "mutation" | "subscription";
    args: { name: string; type: string }[];
    returnType: string;
}

interface SchemaExplorerProps {
    schemaId: string | null;
    onInsertQuery?: (query: string) => void;
    onInsertVariables?: (variables: string) => void;
}

// -- Helpers: Parse introspection JSON into operation list -------------------

function parseIntrospectionToOperations(raw: string): ParsedOperation[] {
    try {
        const json = JSON.parse(raw);
        const schema = json?.data?.__schema ?? json?.__schema;
        if (!schema) return [];

        const ops: ParsedOperation[] = [];
        const typeMap: Record<string, any> = {};
        for (const t of schema.types ?? []) {
            typeMap[t.name] = t;
        }

        const extract = (typeName: string | undefined, opType: "query" | "mutation" | "subscription") => {
            if (!typeName) return;
            const t = typeMap[typeName];
            if (!t?.fields) return;
            for (const field of t.fields) {
                ops.push({
                    name: field.name,
                    type: opType,
                    args: (field.args ?? []).map((a: any) => ({ name: a.name, type: formatTypeRef(a.type) })),
                    returnType: formatTypeRef(field.type),
                });
            }
        };

        extract(schema.queryType?.name, "query");
        extract(schema.mutationType?.name, "mutation");
        extract(schema.subscriptionType?.name, "subscription");
        return ops;
    } catch {
        return [];
    }
}

function formatTypeRef(typeRef: any): string {
    if (!typeRef) return "Unknown";
    if (typeRef.kind === "NON_NULL") return formatTypeRef(typeRef.ofType) + "!";
    if (typeRef.kind === "LIST") return `[${formatTypeRef(typeRef.ofType)}]`;
    return typeRef.name ?? "Unknown";
}

function generateQueryFromOp(op: ParsedOperation): { query: string; variables: string } {
    const varDefs = op.args.map((a) => `$${a.name}: ${a.type}`).join(", ");
    const argPass = op.args.map((a) => `${a.name}: $${a.name}`).join(", ");
    const header = op.type;
    const opName = op.name.charAt(0).toUpperCase() + op.name.slice(1);
    const varSection = varDefs ? `(${varDefs})` : "";
    const argSection = argPass ? `(${argPass})` : "";

    const query = `${header} ${opName}${varSection} {\n  ${op.name}${argSection} {\n    # Add fields here\n    __typename\n  }\n}`;

    const varObj: Record<string, any> = {};
    for (const a of op.args) {
        varObj[a.name] = getDefaultForType(a.type);
    }
    const variables = op.args.length > 0 ? JSON.stringify(varObj, null, 2) : "{}";

    return { query, variables };
}

function getDefaultForType(type: string): any {
    const base = type.replace(/[!\[\]]/g, "");
    if (base === "String" || base === "ID") return "";
    if (base === "Int" || base === "Float") return 0;
    if (base === "Boolean") return false;
    if (type.startsWith("[")) return [];
    return null;
}

// -- Component --------------------------------------------------------------

export default function SchemaExplorer({ schemaId, onInsertQuery, onInsertVariables }: SchemaExplorerProps) {
    const [schemas, setSchemas] = useState<SavedGraphQLSchema[]>([]);
    const [selectedSchema, setSelectedSchema] = useState<SavedGraphQLSchema | null>(null);
    const [operations, setOperations] = useState<ParsedOperation[]>([]);
    const [filter, setFilter] = useState("");
    const [loading, setLoading] = useState(false);
    const [introspectUrl, setIntrospectUrl] = useState("");
    const [error, setError] = useState<string | null>(null);
    const [mode, setMode] = useState<"list" | "introspect">("list");

    // Load schemas list
    useEffect(() => {
        window.api.listGraphQLSchemas().then(setSchemas).catch(() => { });
    }, []);

    // When schemaId changes or schemas load, select the right one
    useEffect(() => {
        if (schemaId) {
            const s = schemas.find((s) => s.id === schemaId);
            if (s) {
                setSelectedSchema(s);
                setOperations(parseIntrospectionToOperations(s.content));
            }
        }
    }, [schemaId, schemas]);

    const handleIntrospect = useCallback(async () => {
        if (!introspectUrl.trim()) return;
        setLoading(true);
        setError(null);
        try {
            const res = await window.api.graphqlIntrospect(introspectUrl, {});
            if (res.ok && res.sdl) {
                // Save as schema
                const saved = await window.api.addGraphQLSchema({
                    name: new URL(introspectUrl).hostname,
                    content: res.sdl,
                    endpointUrl: introspectUrl,
                    introspectedAt: Date.now(),
                });
                setSchemas((prev) => [...prev, saved]);
                setSelectedSchema(saved);
                setOperations(parseIntrospectionToOperations(res.sdl));
                setMode("list");
            } else {
                setError(res.error ?? strings.graphql.introspectionFailed);
            }
        } catch (err: any) {
            setError(err.message ?? strings.graphql.networkError);
        } finally {
            setLoading(false);
        }
    }, [introspectUrl]);

    const handleSelectOperation = useCallback((op: ParsedOperation) => {
        const { query, variables } = generateQueryFromOp(op);
        onInsertQuery?.(query);
        onInsertVariables?.(variables);
    }, [onInsertQuery, onInsertVariables]);

    const filteredOps = operations.filter(
        (op) => !filter || op.name.toLowerCase().includes(filter.toLowerCase()),
    );

    const grouped = {
        query: filteredOps.filter((o) => o.type === "query"),
        mutation: filteredOps.filter((o) => o.type === "mutation"),
        subscription: filteredOps.filter((o) => o.type === "subscription"),
    };

    return (
        <div className="flex flex-col h-full overflow-hidden bg-background">
            {/* Header */}
            <div className="flex items-center gap-2 px-3 py-2 border-b border-border flex-shrink-0">
                <span className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider flex-1">{strings.graphql.schema}</span>
                <button
                    onClick={() => setMode(mode === "introspect" ? "list" : "introspect")}
                    className="text-[10px] text-signal hover:text-signal/80 cursor-pointer"
                >
                    {mode === "introspect" ? strings.graphql.back : strings.graphql.introspect}
                </button>
            </div>

            {/* Introspect mode */}
            {mode === "introspect" && (
                <div className="p-3 flex flex-col gap-2 border-b border-border">
                    <input
                        className="bg-card border border-border rounded px-2.5 py-1.5 text-xs font-mono text-foreground outline-none focus:border-signal placeholder:text-muted-foreground"
                        placeholder="https://api.example.com/graphql"
                        value={introspectUrl}
                        onChange={(e) => setIntrospectUrl(e.target.value)}
                        onKeyDown={(e) => { if (e.key === "Enter") handleIntrospect(); }}
                    />
                    <button
                        onClick={handleIntrospect}
                        disabled={loading || !introspectUrl.trim()}
                        className="px-3 py-1.5 rounded bg-signal text-background text-xs font-semibold disabled:opacity-40 cursor-pointer"
                    >
                        {loading ? strings.graphql.introspecting : strings.graphql.fetchSchema}
                    </button>
                    {error && <span className="text-xs text-destructive">{error}</span>}
                </div>
            )}

            {/* Schema selector */}
            {mode === "list" && schemas.length > 0 && (
                <div className="px-3 py-2 border-b border-border">
                    <select
                        value={selectedSchema?.id ?? ""}
                        onChange={(e) => {
                            const s = schemas.find((s) => s.id === e.target.value);
                            if (s) {
                                setSelectedSchema(s);
                                setOperations(parseIntrospectionToOperations(s.content));
                            }
                        }}
                        className="w-full bg-card border border-border rounded px-2.5 py-1.5 text-xs text-foreground outline-none focus:border-signal"
                    >
                        <option value="">{strings.graphql.selectSchema}</option>
                        {schemas.map((s) => (
                            <option key={s.id} value={s.id}>{s.name}</option>
                        ))}
                    </select>
                </div>
            )}

            {/* Filter */}
            {selectedSchema && (
                <div className="px-3 py-2 border-b border-border">
                    <input
                        className="w-full bg-card border border-border rounded px-2.5 py-1.5 text-xs text-foreground outline-none focus:border-signal placeholder:text-muted-foreground"
                        placeholder={strings.graphql.filterOperations}
                        value={filter}
                        onChange={(e) => setFilter(e.target.value)}
                    />
                </div>
            )}

            {/* Operations list */}
            <div className="flex-1 overflow-y-auto">
                {!selectedSchema && schemas.length === 0 && (
                    <div className="p-4 text-xs text-muted-foreground text-center">
                        {strings.graphql.noSchemasLoaded}
                    </div>
                )}
                {selectedSchema && operations.length === 0 && (
                    <div className="p-4 text-xs text-muted-foreground text-center">
                        {strings.graphql.noOperationsInSchema}
                    </div>
                )}
                {(["query", "mutation", "subscription"] as const).map((type) => {
                    const ops = grouped[type];
                    if (ops.length === 0) return null;
                    return (
                        <div key={type}>
                            <div className="px-3 py-1.5 text-[9px] font-bold uppercase tracking-widest text-muted-foreground bg-surface border-b border-border sticky top-0">
                                {type}s ({ops.length})
                            </div>
                            {ops.map((op) => (
                                <button
                                    key={`${type}-${op.name}`}
                                    onClick={() => handleSelectOperation(op)}
                                    className="w-full text-left px-3 py-1.5 hover:bg-card transition-colors cursor-pointer border-b border-border/50"
                                    title={strings.graphql.insertQueryTemplate.replace("{name}", op.name)}
                                >
                                    <div className="text-xs text-foreground font-mono truncate">{op.name}</div>
                                    {op.args.length > 0 && (
                                        <div className="text-[10px] text-muted-foreground truncate">
                                            ({op.args.map((a) => `${a.name}: ${a.type}`).join(", ")})
                                        </div>
                                    )}
                                    <div className="text-[10px] text-signal/70 truncate">→ {op.returnType}</div>
                                </button>
                            ))}
                        </div>
                    );
                })}
            </div>
        </div>
    );
}
