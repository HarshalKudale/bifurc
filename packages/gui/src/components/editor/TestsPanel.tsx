import React, { useState, useEffect } from "react";
import CodeEditor from "@/components/common/CodeEditor";
import { strings } from "@/lib/strings";

interface TestsPanelProps {
  testScript: string;
  onTestScriptChange: (v: string) => void;
  testResults?: { name: string; passed: boolean; error?: string; durationMs: number }[];
  testLogs?: string[];
  testRunning?: boolean;
}

export default function TestsPanel({ testScript, onTestScriptChange, testResults, testLogs, testRunning }: TestsPanelProps) {
  const [view, setView] = useState<"script" | "results">("script");

  useEffect(() => {
    if (testResults && testResults.length > 0 && !testRunning) {
      setView("results");
    }
  }, [testResults, testRunning]);

  const passCount = testResults?.filter(t => t.passed).length ?? 0;
  const failCount = testResults?.filter(t => !t.passed).length ?? 0;

  return (
    <div className="flex flex-col h-full overflow-hidden">
      <div className="flex items-center border-b border-border/40 bg-background/30 flex-shrink-0">
        <button
          onClick={() => setView("script")}
          className={`px-3 py-1.5 text-[11px] font-medium cursor-pointer ${view === "script" ? "text-foreground border-b-2 border-signal" : "text-muted-foreground hover:text-foreground"}`}
        >
          {strings.editor.script}
        </button>
        <button
          onClick={() => setView("results")}
          className={`px-3 py-1.5 text-[11px] font-medium cursor-pointer ${view === "results" ? "text-foreground border-b-2 border-signal" : "text-muted-foreground hover:text-foreground"}`}
        >
          {strings.editor.results}
          {testResults && testResults.length > 0 && (
            <span className="ml-1.5 text-[10px]">
              <span className="text-signal">{passCount}</span>
              {failCount > 0 && <span className="text-destructive ml-1">{failCount}</span>}
            </span>
          )}
        </button>
        {testRunning && (
          <span className="ml-2 inline-block w-3 h-3 border-2 border-muted-foreground/30 border-t-signal rounded-full animate-spin" />
        )}
      </div>

      {view === "script" ? (
        <CodeEditor
          value={testScript}
          onChange={onTestScriptChange}
          language="javascript"
          placeholder={`// Write tests using lp.test() and lp.expect()\n// Example:\nlp.test("Status is 200", () => {\n  lp.expect(lp.response.status).to.equal(200);\n});\n\nlp.test("Response has data", () => {\n  const json = lp.response.json();\n  lp.expect(json).to.have.property("data");\n});`}
          className="flex-1 overflow-hidden"
        />
      ) : (
        <div className="flex-1 overflow-y-auto p-3 space-y-1.5">
          {(!testResults || testResults.length === 0) && !testRunning && (
            <div className="flex items-center justify-center h-full text-center">
              <p className="text-xs text-muted-foreground">{strings.editor.noTestResults}</p>
            </div>
          )}
          {testResults && testResults.map((t, i) => (
            <div key={i} className={`flex items-start gap-2 px-2.5 py-1.5 rounded text-xs font-mono ${t.passed ? "bg-signal/5 border border-signal/20" : "bg-destructive/5 border border-destructive/20"}`}>
              <span className={`flex-shrink-0 mt-0.5 text-[10px] font-bold ${t.passed ? "text-signal" : "text-destructive"}`}>
                {t.passed ? strings.editor.pass : strings.editor.fail}
              </span>
              <div className="flex-1 min-w-0">
                <span className="text-foreground">{t.name}</span>
                {t.error && <div className="text-destructive/80 mt-0.5 break-words">{t.error}</div>}
              </div>
              <span className="text-muted-foreground text-[10px] flex-shrink-0">{t.durationMs}ms</span>
            </div>
          ))}
          {testLogs && testLogs.length > 0 && (
            <div className="mt-3 pt-2 border-t border-border/40">
              <div className="text-[10px] text-muted-foreground uppercase tracking-wider mb-1">{strings.editor.console}</div>
              {testLogs.map((log, i) => (
                <div key={i} className="text-[11px] font-mono text-muted-foreground px-2 py-0.5">{log}</div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  );
}
