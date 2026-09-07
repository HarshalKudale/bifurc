import React, { useEffect, useRef, useImperativeHandle, forwardRef } from "react";
import { EditorView, placeholder as cmPlaceholder, keymap, lineNumbers } from "@codemirror/view";
import { EditorState, Compartment, Extension } from "@codemirror/state";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import { indentOnInput, bracketMatching, foldGutter } from "@codemirror/language";
import { closeBrackets, closeBracketsKeymap } from "@codemirror/autocomplete";
import { search, openSearchPanel, searchKeymap } from "@codemirror/search";
import { json } from "@codemirror/lang-json";
import { html } from "@codemirror/lang-html";
import { xml } from "@codemirror/lang-xml";
import { javascript } from "@codemirror/lang-javascript";
import { BifurcTheme, getHighlightExtension } from "@/lib/codemirrorTheme";

export type EditorLanguage = "json" | "html" | "xml" | "javascript" | "text";

export interface CodeEditorHandle {
  /** Insert text at the current cursor/selection, replacing any selection. */
  insertAtCursor(text: string): void;
  /** Open the find/replace panel in the editor. */
  openFind(): void;
}

export interface CodeEditorProps {
  value: string;
  onChange?: (value: string) => void;
  language?: EditorLanguage;
  readOnly?: boolean;
  placeholder?: string;
  className?: string;
  minHeight?: number;
}

function getLanguageExtension(lang: EditorLanguage): Extension {
  switch (lang) {
    case "json":       return json();
    case "html":       return html();
    case "xml":        return xml();
    case "javascript": return javascript();
    default:           return [];
  }
}

function isLightMode(): boolean {
  return document.documentElement.classList.contains("light");
}

export default forwardRef<CodeEditorHandle, CodeEditorProps>(function CodeEditor({
  value,
  onChange,
  language = "text",
  readOnly = false,
  placeholder,
  className,
  minHeight,
}, ref) {
  const containerRef = useRef<HTMLDivElement>(null);
  const viewRef      = useRef<EditorView | null>(null);
  // Compartments for dynamic reconfiguration without recreating editor
  const langComp      = useRef(new Compartment());
  const readOnlyComp  = useRef(new Compartment());
  const highlightComp = useRef(new Compartment());
  // Guard flag: skip onChange while we're programmatically updating the doc
  const updatingRef   = useRef(false);

  // Expose imperative handle
  useImperativeHandle(ref, () => ({
    insertAtCursor(text: string) {
      const view = viewRef.current;
      if (!view) return;
      const { from, to } = view.state.selection.main;
      view.dispatch({
        changes: { from, to, insert: text },
        selection: { anchor: from + text.length },
      });
      view.focus();
    },
    openFind() {
      const view = viewRef.current;
      if (!view) return;
      openSearchPanel(view);
      view.focus();
    },
  }), []);

  // -- Create editor on mount ------------------------------------------------

  useEffect(() => {
    if (!containerRef.current) return;

    const light = isLightMode();

    const editingExtensions: Extension = readOnly ? [] : [
      history(),
      keymap.of([...defaultKeymap, ...historyKeymap, ...closeBracketsKeymap, ...searchKeymap, indentWithTab]),
      closeBrackets(),
      bracketMatching(),
      indentOnInput(),
      search({ top: true }),
    ];

    const extensions: Extension[] = [
      BifurcTheme,
      highlightComp.current.of(getHighlightExtension(light)),
      langComp.current.of(getLanguageExtension(language)),
      readOnlyComp.current.of([
        EditorState.readOnly.of(readOnly),
        EditorView.editable.of(!readOnly),
      ]),
      lineNumbers(),
      foldGutter(),
      editingExtensions,
      EditorView.lineWrapping,
      EditorView.updateListener.of((update) => {
        if (update.docChanged && !updatingRef.current && onChange) {
          onChange(update.state.doc.toString());
        }
      }),
      ...(placeholder ? [cmPlaceholder(placeholder)] : []),
    ];

    const state = EditorState.create({ doc: value, extensions });
    const view  = new EditorView({ state, parent: containerRef.current });
    viewRef.current = view;

    // Listen for dark/light theme toggle on <html>
    const observer = new MutationObserver(() => {
      if (!viewRef.current) return;
      viewRef.current.dispatch({
        effects: highlightComp.current.reconfigure(getHighlightExtension(isLightMode())),
      });
    });
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ["class"] });

    return () => {
      observer.disconnect();
      viewRef.current?.destroy();
      viewRef.current = null;
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // intentionally empty - we manage all updates imperatively

  // -- Sync external value changes -------------------------------------------

  useEffect(() => {
    const view = viewRef.current;
    if (!view) return;
    const current = view.state.doc.toString();
    if (current === value) return;
    updatingRef.current = true;
    view.dispatch({
      changes: { from: 0, to: current.length, insert: value },
    });
    updatingRef.current = false;
  }, [value]);

  // -- Sync language changes -------------------------------------------------

  useEffect(() => {
    viewRef.current?.dispatch({
      effects: langComp.current.reconfigure(getLanguageExtension(language)),
    });
  }, [language]);

  // -- Sync readOnly changes -------------------------------------------------

  useEffect(() => {
    viewRef.current?.dispatch({
      effects: readOnlyComp.current.reconfigure([
        EditorState.readOnly.of(readOnly),
        EditorView.editable.of(!readOnly),
      ]),
    });
  }, [readOnly]);

  const style: React.CSSProperties = minHeight != null ? { minHeight } : {};

  return (
    <div
      ref={containerRef}
      className={className}
      style={style}
      data-testid="code-editor"
    />
  );
});
