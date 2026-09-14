import React, { useState, useEffect, useRef, forwardRef, useImperativeHandle } from "react";
import { strings } from "@/lib/strings";
import { BodyMode, KVPair, parseFormBody, serializeFormBody } from "@/lib/bodyUtils";
import { mkRowId } from "@/lib/utils";
import { X } from "@/lib/icons";
import CodeEditor, { EditorLanguage, CodeEditorHandle } from "@/components/common/CodeEditor";
import MultipartEditor from "@/components/editor/MultipartEditor";
import BinaryViewer from "@/components/common/BinaryViewer";

export interface BodyEditorHandle {
  /** Insert text at the current cursor/selection in the code editor. */
  insertAtCursor(text: string): void;
  /** Open the find/replace panel in the code editor. */
  openFind(): void;
}

const MODE_LABELS: Record<BodyMode, string> = {
  json: strings.editor.modeJson,
  text: strings.editor.modeText,
  html: strings.editor.modeHtml,
  xml:  strings.editor.modeXml,
  form: strings.editor.modeForm,
  multipart: strings.editor.modeMultipart,
  binary: strings.editor.modeBinary,
  image: strings.editor.modeImage,
  none: strings.editor.modeNone,
};

const EDITABLE_MODES: BodyMode[] = ["json", "text", "html", "xml", "form", "multipart", "binary", "image", "none"];

interface Props {
  value: string;
  onChange?: (v: string) => void;
  placeholder?: string;
  readOnly?: boolean;
  minHeight?: number;
  mode?: BodyMode;
  onModeChange?: (mode: BodyMode) => void;
  /** Content-Type header value - used for binary/image display */
  contentType?: string;
  /** When true, value is base64-encoded binary data */
  isBase64?: boolean;
}

export default forwardRef<BodyEditorHandle, Props>(function BodyEditor({
  value,
  onChange,
  placeholder = "",
  readOnly = false,
  minHeight = 80,
  mode = "json",
  onModeChange,
  contentType,
  isBase64 = false,
}, ref) {
  const codeEditorRef = useRef<CodeEditorHandle>(null);

  useImperativeHandle(ref, () => ({
    insertAtCursor(text: string) {
      codeEditorRef.current?.insertAtCursor(text);
    },
    openFind() {
      codeEditorRef.current?.openFind();
    },
  }), []);

  const [jsonError, setJsonError] = useState("");
  const [xmlError,  setXmlError]  = useState("");
  const [formPairs, setFormPairs] = useState<(KVPair & { id: string })[]>(() => {
    if (mode === "form") {
      return parseFormBody(value).map((p) => ({ ...p, id: mkRowId() }));
    }
    return [];
  });

  useEffect(() => {
    if (mode === "form") {
      const incoming = parseFormBody(value);
      setFormPairs(incoming.map((p) => ({ ...p, id: mkRowId() })));
    }
  // Only resync if mode flips to form from outside
  // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [mode === "form" ? value : undefined]);

  useEffect(() => {
    if (mode !== "json") { setJsonError(""); return; }
    if (!value.trim() || value.includes("{{")) { setJsonError(""); return; }
    try { JSON.parse(value); setJsonError(""); } catch { setJsonError("Invalid JSON"); }
  }, [value, mode]);

  useEffect(() => {
    if (mode !== "xml") { setXmlError(""); return; }
    if (!value.trim() || value.includes("{{")) { setXmlError(""); return; }
    try {
      const parser = new DOMParser();
      const doc = parser.parseFromString(value, "application/xml");
      const err = doc.querySelector("parsererror");
      setXmlError(err ? "Invalid XML" : "");
    } catch {
      setXmlError("Invalid XML");
    }
  }, [value, mode]);

  const handleTextChange = (v: string) => {
    onChange?.(v);
    if (mode === "json") {
      if (!v.trim() || v.includes("{{")) { setJsonError(""); return; }
      try { JSON.parse(v); setJsonError(""); } catch { setJsonError("Invalid JSON"); }
    }
  };

  const handleFormat = () => {
    if (!onChange) return;
    if (mode === "json") {
      try { onChange(JSON.stringify(JSON.parse(value), null, 2)); setJsonError(""); } catch {}
    }
  };

  const handleFormPairChange = (id: string, field: "key" | "value", val: string) => {
    const updated = formPairs.map((p) => p.id === id ? { ...p, [field]: val } : p);
    setFormPairs(updated);
    onChange?.(serializeFormBody(updated));
  };

  const handleFormPairAdd = () => {
    const updated = [...formPairs, { id: mkRowId(), key: "", value: "" }];
    setFormPairs(updated);
    onChange?.(serializeFormBody(updated));
  };

  const handleFormPairRemove = (id: string) => {
    const updated = formPairs.filter((p) => p.id !== id);
    setFormPairs(updated);
    onChange?.(serializeFormBody(updated));
  };

  const handleModeChange = (newMode: BodyMode) => {
    if (newMode === "form" && mode !== "form") {
      const pairs = parseFormBody(value).map((p) => ({ ...p, id: mkRowId() }));
      setFormPairs(pairs.length > 0 ? pairs : [{ id: mkRowId(), key: "", value: "" }]);
      onChange?.(serializeFormBody(pairs));
    }
    if (newMode !== "form" && mode === "form") {
      onChange?.(serializeFormBody(formPairs));
    }
    // Don't call onChange for none - the reducer stashes/restores body via SET_REQ_MODE
    onModeChange?.(newMode);
  };

  const langLabel = mode === "json" ? "json"
    : mode === "form" ? "form-urlencoded"
    : mode === "none" ? "no body"
    : mode;

  const syntaxError = mode === "json" ? jsonError : mode === "xml" ? xmlError : "";

  return (
    <div className="flex flex-col h-full">
      {/* Toolbar */}
      <div className="flex items-center border-b border-border/40 flex-shrink-0 bg-background/20">
        {/* Mode selector (left) */}
        <div className="flex items-center gap-0.5 px-2 py-1 border-r border-border/40">
          {EDITABLE_MODES.map((m) => (
            <button
              key={m}
              onClick={() => !readOnly && handleModeChange(m)}
              disabled={readOnly}
              className={`px-2 py-0.5 rounded text-[10px] font-medium transition-colors cursor-pointer disabled:cursor-default ${
                mode === m
                  ? "bg-signal/20 text-signal"
                  : "text-muted-foreground hover:text-foreground hover:bg-card/50"
              }`}
            >
              {MODE_LABELS[m]}
            </button>
          ))}
        </div>

        {/* Right section */}
        <div className="flex items-center gap-3 px-3 py-1.5 flex-1 justify-between">
          <span className="text-[10px] font-mono text-muted-foreground tracking-widest uppercase">
            {langLabel}
          </span>
          <div className="flex items-center gap-3">
            {syntaxError && <span className="text-[10px] text-destructive font-mono">{syntaxError}</span>}
            {!readOnly && mode === "json" && (
              <button
                onClick={handleFormat}
                className="text-[10px] text-muted-foreground hover:text-signal cursor-pointer transition-colors font-medium"
              >
                {strings.common.format}
              </button>
            )}
          </div>
        </div>
      </div>

      {/* Body area */}
      {mode === "none" ? (
        <div className="flex items-center justify-center flex-1 text-xs text-muted-foreground italic opacity-60">
          {strings.editor.noBody}
        </div>
      ) : mode === "multipart" ? (
        <MultipartEditor
          value={value}
          onChange={onChange}
          readOnly={readOnly}
        />
      ) : mode === "binary" || mode === "image" ? (
        <BinaryViewer
          data={isBase64 ? value : (value ? btoa(unescape(encodeURIComponent(value))) : "")}
          contentType={contentType ?? (mode === "image" ? "image/png" : "application/octet-stream")}
          editable={!readOnly}
          onChange={onChange}
        />
      ) : mode === "form" ? (
        <div className="flex flex-col flex-1 overflow-y-auto">
          {/* Form column headers */}
          <div className="flex items-center border-b border-border/40 bg-background/10 flex-shrink-0">
            <div className="flex-1 px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground border-r border-border/40">
              {strings.common.key}
            </div>
            <div className="flex-1 px-3 py-1.5 text-[10px] font-semibold uppercase tracking-wider text-muted-foreground">
              {strings.common.value}
            </div>
            {!readOnly && <div className="w-9 flex-shrink-0" />}
          </div>

          {formPairs.length === 0 && readOnly && (
            <p className="px-4 py-5 text-xs text-muted-foreground italic">{strings.editor.noFormFields}</p>
          )}

          {formPairs.map((pair) => (
            <div key={pair.id} className="flex items-stretch border-b border-border/25 last:border-0 group hover:bg-card/30 transition-colors">
              <div className="flex-1 border-r border-border/25 min-w-0">
                <input
                  className="w-full h-full bg-transparent font-mono text-xs px-3 py-2 outline-none focus:bg-card/60 min-w-0"
                  style={{ color: "var(--c-signal)" }}
                  placeholder={readOnly ? "—" : strings.editor.placeholderKey}
                  value={pair.key}
                  onChange={(e) => handleFormPairChange(pair.id, "key", e.target.value)}
                  readOnly={readOnly}
                />
              </div>
              <div className="flex-1 min-w-0">
                <input
                  className="w-full h-full bg-transparent font-mono text-xs text-foreground px-3 py-2 outline-none focus:bg-card/60 min-w-0"
                  placeholder={readOnly ? "—" : strings.editor.placeholderValue}
                  value={pair.value}
                  onChange={(e) => handleFormPairChange(pair.id, "value", e.target.value)}
                  readOnly={readOnly}
                />
              </div>
              {!readOnly && (
                <button
                  onClick={() => handleFormPairRemove(pair.id)}
                  className="w-9 flex-shrink-0 flex items-center justify-center text-muted-foreground hover:text-destructive opacity-0 group-hover:opacity-100 transition-all cursor-pointer"
                ><X size={13} /></button>
              )}
            </div>
          ))}

          {!readOnly && (
            <button
              onClick={handleFormPairAdd}
              className="flex items-center gap-2 px-4 py-2.5 text-xs text-muted-foreground hover:text-foreground hover:bg-card/30 transition-colors cursor-pointer text-left border-t border-border/20"
            >
              <span className="text-signal font-semibold text-sm leading-none">+</span>
              {strings.editor.addField}
            </button>
          )}
        </div>
      ) : (
        <CodeEditor
          ref={codeEditorRef}
          value={value}
          onChange={readOnly ? undefined : handleTextChange}
          language={
            mode === "json" ? "json"
            : mode === "html" ? "html"
            : mode === "xml" ? "xml"
            : "text" as EditorLanguage
          }
          readOnly={readOnly}
          placeholder={placeholder}
          minHeight={minHeight}
          className="flex-1 overflow-hidden"
        />
      )}
    </div>
  );
});
