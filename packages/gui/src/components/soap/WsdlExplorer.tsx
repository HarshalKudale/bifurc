import React, { useState, useCallback, useEffect } from "react";
import { SavedWsdl } from "@/types";
import { cn } from "@/components/ui/cn";
import { strings } from "@/lib/strings";

// -- Types ------------------------------------------------------------------

interface WsdlOperation {
    name: string;
    soapAction: string;
    inputFields: { name: string; type: string }[];
    outputType: string;
}

interface WsdlExplorerProps {
    wsdlId: string | null;
    onInsertEnvelope?: (body: string, soapAction: string) => void;
}

// -- Helpers: Parse WSDL XML to extract operations --------------------------

function parseWsdlOperations(xml: string): WsdlOperation[] {
    const ops: WsdlOperation[] = [];
    try {
        // Extract portType operations
        const opRegex = /<wsdl:operation\s+name="([^"]+)"|<operation\s+name="([^"]+)"/g;
        const bindingActionRegex = /<soap:operation\s+soapAction="([^"]+)"|<soap12:operation\s+soapAction="([^"]+)"/g;

        // Collect all operation names
        const operationNames: string[] = [];
        let match;
        while ((match = opRegex.exec(xml)) !== null) {
            const name = match[1] || match[2];
            if (name && !operationNames.includes(name)) operationNames.push(name);
        }

        // Collect soapAction values
        const soapActions: string[] = [];
        while ((match = bindingActionRegex.exec(xml)) !== null) {
            soapActions.push(match[1] || match[2]);
        }

        // Extract element schemas for input messages
        for (let i = 0; i < operationNames.length; i++) {
            const name = operationNames[i];
            const soapAction = soapActions[i] || `urn:${name}`;

            // Try to find input message elements
            const inputFields = extractInputFields(xml, name);

            ops.push({
                name,
                soapAction,
                inputFields,
                outputType: `${name}Response`,
            });
        }
    } catch {
        // If parsing fails, return empty
    }
    return ops;
}

function extractInputFields(xml: string, operationName: string): { name: string; type: string }[] {
    const fields: { name: string; type: string }[] = [];
    // Look for element definition matching the operation's input message
    const elementPatterns = [
        new RegExp(`<(?:xsd:|xs:|s:)?element[^>]+name="${operationName}"[^>]*>([\\s\\S]*?)</(?:xsd:|xs:|s:)?element>`, "i"),
        new RegExp(`<(?:xsd:|xs:|s:)?complexType[^>]+name="${operationName}"[^>]*>([\\s\\S]*?)</(?:xsd:|xs:|s:)?complexType>`, "i"),
    ];

    for (const pattern of elementPatterns) {
        const match = pattern.exec(xml);
        if (match) {
            // Extract child elements
            const fieldRegex = /<(?:xsd:|xs:|s:)?element\s+[^>]*name="([^"]+)"[^>]*type="(?:[^":]+:)?([^"]+)"/g;
            let fieldMatch;
            while ((fieldMatch = fieldRegex.exec(match[1])) !== null) {
                fields.push({ name: fieldMatch[1], type: fieldMatch[2] });
            }
            break;
        }
    }
    return fields;
}

function generateSoapEnvelope(op: WsdlOperation, targetNamespace?: string): string {
    const ns = targetNamespace || "http://tempuri.org/";
    const fieldXml = op.inputFields.length > 0
        ? op.inputFields.map((f) => `      <${f.name}><!--${f.type}--></${f.name}>`).join("\n")
        : "      <!-- Add parameters here -->";

    return `<?xml version="1.0" encoding="utf-8"?>
<soap:Envelope xmlns:soap="http://schemas.xmlsoap.org/soap/envelope/"
               xmlns:tns="${ns}">
  <soap:Header/>
  <soap:Body>
    <tns:${op.name}>
${fieldXml}
    </tns:${op.name}>
  </soap:Body>
</soap:Envelope>`;
}

function extractTargetNamespace(xml: string): string {
    const match = /targetNamespace="([^"]+)"/.exec(xml);
    return match?.[1] || "http://tempuri.org/";
}

// -- Component --------------------------------------------------------------

export default function WsdlExplorer({ wsdlId, onInsertEnvelope }: WsdlExplorerProps) {
    const [wsdls, setWsdls] = useState<SavedWsdl[]>([]);
    const [selectedWsdl, setSelectedWsdl] = useState<SavedWsdl | null>(null);
    const [operations, setOperations] = useState<WsdlOperation[]>([]);
    const [filter, setFilter] = useState("");
    const [loading, setLoading] = useState(false);
    const [fetchUrl, setFetchUrl] = useState("");
    const [error, setError] = useState<string | null>(null);
    const [mode, setMode] = useState<"list" | "fetch">("list");

    // Load WSDLs list
    useEffect(() => {
        window.api.listWsdls().then(setWsdls).catch(() => { });
    }, []);

    // Select WSDL by ID
    useEffect(() => {
        if (wsdlId) {
            const w = wsdls.find((w) => w.id === wsdlId);
            if (w) {
                setSelectedWsdl(w);
                setOperations(parseWsdlOperations(w.content));
            }
        }
    }, [wsdlId, wsdls]);

    const handleFetchWsdl = useCallback(async () => {
        if (!fetchUrl.trim()) return;
        setLoading(true);
        setError(null);
        try {
            const res = await window.api.soapFetchWsdl(fetchUrl);
            if (res.ok && res.content) {
                const saved = await window.api.addWsdl({
                    name: new URL(fetchUrl).hostname,
                    content: res.content,
                    sourceUrl: fetchUrl,
                    importedAt: Date.now(),
                });
                setWsdls((prev) => [...prev, saved]);
                setSelectedWsdl(saved);
                setOperations(parseWsdlOperations(res.content));
                setMode("list");
            } else {
                setError(res.error ?? strings.soap.fetchWsdlFailed);
            }
        } catch (err: any) {
            setError(err.message ?? strings.soap.networkError);
        } finally {
            setLoading(false);
        }
    }, [fetchUrl]);

    const handleSelectOperation = useCallback((op: WsdlOperation) => {
        if (!selectedWsdl) return;
        const ns = extractTargetNamespace(selectedWsdl.content);
        const envelope = generateSoapEnvelope(op, ns);
        onInsertEnvelope?.(envelope, op.soapAction);
    }, [selectedWsdl, onInsertEnvelope]);

    const filteredOps = operations.filter(
        (op) => !filter || op.name.toLowerCase().includes(filter.toLowerCase()),
    );

    return (
        <div className="flex flex-col h-full overflow-hidden bg-background">
            {/* Header */}
            <div className="flex items-center gap-2 px-3 py-2 border-b border-border flex-shrink-0">
                <span className="text-[10px] font-semibold text-muted-foreground uppercase tracking-wider flex-1">{strings.soap.wsdl}</span>
                <button
                    onClick={() => setMode(mode === "fetch" ? "list" : "fetch")}
                    className="text-[10px] text-signal hover:text-signal/80 cursor-pointer"
                >
                    {mode === "fetch" ? strings.soap.back : strings.soap.fetchWsdl}
                </button>
            </div>

            {/* Fetch mode */}
            {mode === "fetch" && (
                <div className="p-3 flex flex-col gap-2 border-b border-border">
                    <input
                        className="bg-card border border-border rounded px-2.5 py-1.5 text-xs font-mono text-foreground outline-none focus:border-signal placeholder:text-muted-foreground"
                        placeholder="https://service.example.com?wsdl"
                        value={fetchUrl}
                        onChange={(e) => setFetchUrl(e.target.value)}
                        onKeyDown={(e) => { if (e.key === "Enter") handleFetchWsdl(); }}
                    />
                    <button
                        onClick={handleFetchWsdl}
                        disabled={loading || !fetchUrl.trim()}
                        className="px-3 py-1.5 rounded bg-signal text-background text-xs font-semibold disabled:opacity-40 cursor-pointer"
                    >
                        {loading ? strings.soap.fetching : strings.soap.fetchAndImport}
                    </button>
                    {error && <span className="text-xs text-destructive">{error}</span>}
                </div>
            )}

            {/* WSDL selector */}
            {mode === "list" && wsdls.length > 0 && (
                <div className="px-3 py-2 border-b border-border">
                    <select
                        value={selectedWsdl?.id ?? ""}
                        onChange={(e) => {
                            const w = wsdls.find((w) => w.id === e.target.value);
                            if (w) {
                                setSelectedWsdl(w);
                                setOperations(parseWsdlOperations(w.content));
                            }
                        }}
                        className="w-full bg-card border border-border rounded px-2.5 py-1.5 text-xs text-foreground outline-none focus:border-signal"
                    >
                        <option value="">{strings.soap.selectWsdl}</option>
                        {wsdls.map((w) => (
                            <option key={w.id} value={w.id}>{w.name}{w.sourceUrl ? ` (${w.sourceUrl})` : ""}</option>
                        ))}
                    </select>
                </div>
            )}

            {/* Filter */}
            {selectedWsdl && operations.length > 0 && (
                <div className="px-3 py-2 border-b border-border">
                    <input
                        className="w-full bg-card border border-border rounded px-2.5 py-1.5 text-xs text-foreground outline-none focus:border-signal placeholder:text-muted-foreground"
                        placeholder={strings.soap.filterOperations}
                        value={filter}
                        onChange={(e) => setFilter(e.target.value)}
                    />
                </div>
            )}

            {/* Operations list */}
            <div className="flex-1 overflow-y-auto">
                {!selectedWsdl && wsdls.length === 0 && (
                    <div className="p-4 text-xs text-muted-foreground text-center">
                        {strings.soap.noWsdlsImported}
                    </div>
                )}
                {selectedWsdl && operations.length === 0 && (
                    <div className="p-4 text-xs text-muted-foreground text-center">
                        {strings.soap.noOperationsInWsdl}
                    </div>
                )}
                {filteredOps.map((op) => (
                    <button
                        key={op.name}
                        onClick={() => handleSelectOperation(op)}
                        className="w-full text-left px-3 py-2 hover:bg-card transition-colors cursor-pointer border-b border-border/50"
                        title={strings.soap.insertEnvelope.replace("{name}", op.name)}
                    >
                        <div className="text-xs text-foreground font-mono truncate">{op.name}</div>
                        <div className="text-[10px] text-muted-foreground truncate mt-0.5">
                            {strings.soap.soapAction} {op.soapAction}
                        </div>
                        {op.inputFields.length > 0 && (
                            <div className="text-[10px] text-signal/70 truncate">
                                {op.inputFields.map((f) => `${f.name}: ${f.type}`).join(", ")}
                            </div>
                        )}
                    </button>
                ))}
            </div>
        </div>
    );
}
