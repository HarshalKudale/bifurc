import React from "react";
import CodeEditor from "@/components/common/CodeEditor";
import { strings } from "@/lib/strings";

interface ScriptEditorProps {
  value: string;
  onChange: (v: string) => void;
  placeholder?: string;
  error?: string;
}

export default function ScriptEditor({ value, onChange, placeholder, error }: ScriptEditorProps) {
  return (
    <div className="flex flex-col h-full overflow-hidden">
      {error && (
        <div className="px-3 py-1.5 border-b border-destructive/30 bg-destructive/5 flex-shrink-0">
          <span className="text-[11px] text-destructive font-mono">{strings.editor.scriptError}: {error}</span>
        </div>
      )}
      <CodeEditor
        value={value}
        onChange={onChange}
        language="javascript"
        placeholder={placeholder}
        className="flex-1 overflow-hidden"
      />
    </div>
  );
}
