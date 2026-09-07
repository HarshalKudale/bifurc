import React, { useState, useEffect } from "react";
import { MultipartPart, formatBytes } from "@/lib/bodyUtils";
import { X, Upload } from "@/lib/icons";
import { strings } from "@/lib/strings";

const MAX_FILE_SIZE = 1024 * 1024; // 1 MB

interface Props {
    value: string;
    onChange?: (v: string) => void;
    readOnly?: boolean;
}

function parseParts(value: string): MultipartPart[] {
    if (!value.trim()) return [];
    try {
        const parsed = JSON.parse(value);
        if (parsed && Array.isArray(parsed.parts)) return parsed.parts;
    } catch { /* not JSON multipart format */ }
    return [];
}

function serializeParts(parts: MultipartPart[]): string {
    return JSON.stringify({ parts });
}

export default function MultipartEditor({ value, onChange, readOnly = false }: Props) {
    const [parts, setParts] = useState<(MultipartPart & { _id: string })[]>(() =>
        parseParts(value).map((p) => ({ ...p, _id: crypto.randomUUID() }))
    );
    const [error, setError] = useState<string | null>(null);

    useEffect(() => {
        const incoming = parseParts(value);
        if (JSON.stringify(incoming) !== JSON.stringify(parts.map(({ _id, ...rest }) => rest))) {
            setParts(incoming.map((p) => ({ ...p, _id: crypto.randomUUID() })));
        }
        // eslint-disable-next-line react-hooks/exhaustive-deps
    }, [value]);

    const emitChange = (updated: (MultipartPart & { _id: string })[]) => {
        setParts(updated);
        onChange?.(serializeParts(updated.map(({ _id, ...rest }) => rest)));
    };

    const handleFieldChange = (id: string, field: keyof MultipartPart, val: string) => {
        emitChange(parts.map((p) => p._id === id ? { ...p, [field]: val } : p));
    };

    const handleTypeToggle = (id: string, newType: "text" | "file") => {
        emitChange(parts.map((p) =>
            p._id === id ? { ...p, type: newType, value: "", fileName: undefined, mimeType: undefined } : p
        ));
    };

    const handleAddPart = () => {
        emitChange([...parts, { _id: crypto.randomUUID(), key: "", type: "text", value: "" }]);
    };

    const handleRemovePart = (id: string) => {
        emitChange(parts.filter((p) => p._id !== id));
    };

    const handleFilePick = async (id: string) => {
        setError(null);
        try {
            const result = await (window as any).api.openFileDialog();
            if (!result) return; // cancelled
            if (result.size > MAX_FILE_SIZE) {
                setError(strings.editor.fileTooLarge.replace("{size}", formatBytes(result.size)));
                return;
            }
            emitChange(parts.map((p) =>
                p._id === id
                    ? { ...p, value: result.base64, fileName: result.name, mimeType: result.mimeType }
                    : p
            ));
        } catch (e) {
            setError(e instanceof Error ? e.message : strings.editor.failedToOpenFile);
        }
    };

    return (
        <div className="flex flex-col flex-1 overflow-y-auto">
            {error && (
                <div className="px-3 py-1.5 border-b border-destructive/30 bg-destructive/5 flex-shrink-0">
                    <span className="text-[11px] text-destructive font-mono">{error}</span>
                </div>
            )}

            {/* Column headers */}
            <div className="flex items-center border-b border-border/40 bg-background/10 flex-shrink-0">
                <div className="w-32 px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground border-r border-border/40">
                    {strings.common.key}
                </div>
                <div className="w-16 px-2 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground border-r border-border/40 text-center">
                    {strings.editor.colType}
                </div>
                <div className="flex-1 px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
                    {strings.common.value}
                </div>
                {!readOnly && <div className="w-9 flex-shrink-0" />}
            </div>

            {parts.length === 0 && readOnly && (
                <p className="px-4 py-5 text-xs text-muted-foreground italic">{strings.editor.noMultipartFields}</p>
            )}

            {parts.map((part) => (
                <div key={part._id} className="flex items-stretch border-b border-border/25 last:border-0 group hover:bg-card/30 transition-colors">
                    {/* Key */}
                    <div className="w-32 border-r border-border/25 min-w-0">
                        <input
                            className="w-full h-full bg-transparent font-mono text-xs px-3 py-2 outline-none focus:bg-card/60"
                            style={{ color: "var(--c-signal)" }}
                            placeholder={readOnly ? "—" : strings.editor.placeholderFieldName}
                            value={part.key}
                            onChange={(e) => handleFieldChange(part._id, "key", e.target.value)}
                            readOnly={readOnly}
                        />
                    </div>

                    {/* Type toggle */}
                    <div className="w-16 border-r border-border/25 flex items-center justify-center">
                        {readOnly ? (
                            <span className="text-[10px] text-muted-foreground">{part.type}</span>
                        ) : (
                            <button
                                onClick={() => handleTypeToggle(part._id, part.type === "text" ? "file" : "text")}
                                className="px-1.5 py-0.5 rounded text-[10px] font-medium transition-colors cursor-pointer bg-card hover:bg-signal/20 text-muted-foreground hover:text-signal"
                            >
                                {part.type === "text" ? strings.editor.typeTextToggle : strings.editor.typeFileToggle}
                            </button>
                        )}
                    </div>

                    {/* Value */}
                    <div className="flex-1 min-w-0 flex items-center">
                        {part.type === "text" ? (
                            <input
                                className="w-full h-full bg-transparent font-mono text-xs text-foreground px-3 py-2 outline-none focus:bg-card/60"
                                placeholder={readOnly ? "—" : strings.editor.placeholderValue}
                                value={part.value}
                                onChange={(e) => handleFieldChange(part._id, "value", e.target.value)}
                                readOnly={readOnly}
                            />
                        ) : (
                            <div className="flex items-center gap-2 px-3 py-1.5 w-full">
                                {part.fileName ? (
                                    <span className="text-xs text-foreground font-mono truncate">
                                        {part.fileName}
                                        {part.value && (
                                            <span className="ml-2 text-muted-foreground">
                                                ({formatBytes(Math.ceil(part.value.length * 3 / 4))})
                                            </span>
                                        )}
                                    </span>
                                ) : (
                                    <span className="text-xs text-muted-foreground italic">{strings.editor.noFileSelected}</span>
                                )}
                                {!readOnly && (
                                    <button
                                        onClick={() => handleFilePick(part._id)}
                                        className="flex items-center gap-1 px-2 py-1 rounded text-[10px] font-medium bg-card border border-border hover:border-signal text-muted-foreground hover:text-signal cursor-pointer transition-colors ml-auto"
                                    >
                                        <Upload size={10} />
                                        {strings.editor.browse}
                                    </button>
                                )}
                            </div>
                        )}
                    </div>

                    {/* Remove button */}
                    {!readOnly && (
                        <button
                            onClick={() => handleRemovePart(part._id)}
                            className="w-9 flex-shrink-0 flex items-center justify-center text-muted-foreground hover:text-destructive opacity-0 group-hover:opacity-100 transition-all cursor-pointer"
                        ><X size={13} /></button>
                    )}
                </div>
            ))}

            {!readOnly && (
                <button
                    onClick={handleAddPart}
                    className="flex items-center gap-2 px-4 py-2.5 text-xs text-muted-foreground hover:text-foreground hover:bg-card/30 transition-colors cursor-pointer text-left border-t border-border/20"
                >
                    <span className="text-signal font-semibold text-sm leading-none">+</span>
                    {strings.editor.addField}
                </button>
            )}
        </div>
    );
}
