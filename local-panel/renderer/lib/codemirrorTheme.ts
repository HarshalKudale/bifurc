import { EditorView } from "@codemirror/view";
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { tags as t } from "@lezer/highlight";
import { Extension } from "@codemirror/state";

/* ── Editor chrome theme (auto-follows dark/light via CSS vars) ─────── */

export const localPanelTheme = EditorView.theme({
  "&": {
    height: "100%",
    fontSize: "12px",
    fontFamily: "var(--font-mono)",
    background: "transparent",
  },
  ".cm-scroller": {
    overflow: "auto",
    fontFamily: "inherit",
    lineHeight: "1.6",
  },
  "&.cm-focused": {
    outline: "none",
  },
  ".cm-content": {
    padding: "12px 16px",
    caretColor: "var(--c-signal)",
  },
  ".cm-line": {
    padding: "0",
  },
  ".cm-cursor, .cm-dropCursor": {
    borderLeftColor: "var(--c-signal)",
  },
  ".cm-selectionBackground, ::selection": {
    background: "oklch(var(--signal) / 0.18) !important",
  },
  "&.cm-focused .cm-selectionBackground": {
    background: "oklch(var(--signal) / 0.18) !important",
  },
  ".cm-activeLine": {
    background: "oklch(var(--signal) / 0.04)",
  },
  ".cm-gutters": {
    background: "var(--c-card)",
    color: "var(--c-muted-foreground)",
    border: "none",
    borderRight: "1px solid var(--c-border)",
  },
  ".cm-activeLineGutter": {
    background: "oklch(var(--signal) / 0.08)",
    color: "var(--c-signal)",
  },
  ".cm-matchingBracket": {
    background: "oklch(var(--signal) / 0.25)",
    outline: "none",
  },
  ".cm-nonmatchingBracket": {
    background: "oklch(var(--destructive) / 0.2)",
  },
  ".cm-placeholder": {
    color: "var(--c-muted-foreground)",
    opacity: "0.5",
    fontStyle: "italic",
  },
  ".cm-tooltip": {
    background: "var(--c-card)",
    border: "1px solid var(--c-border)",
    borderRadius: "var(--radius-sm)",
  },
  ".cm-tooltip-autocomplete ul li[aria-selected]": {
    background: "oklch(var(--signal) / 0.2)",
    color: "var(--c-foreground)",
  },
});

/* ── Syntax highlight (CSS-var-driven, auto-switches with mode) ─────── */

const highlight = HighlightStyle.define([
  { tag: t.keyword, color: "var(--syn-keyword)", fontWeight: "bold" },
  { tag: [t.bool, t.null], color: "var(--syn-bool)" },
  { tag: t.string, color: "var(--syn-string)" },
  { tag: [t.number, t.integer], color: "var(--syn-number)" },
  { tag: t.comment, color: "var(--syn-comment)", fontStyle: "italic" },
  { tag: t.propertyName, color: "var(--syn-property)" },
  { tag: t.tagName, color: "var(--syn-tag)" },
  { tag: t.attributeName, color: "var(--syn-attr-name)" },
  { tag: t.attributeValue, color: "var(--syn-attr-val)" },
  { tag: t.angleBracket, color: "var(--syn-bracket)" },
  { tag: t.variableName, color: "var(--syn-variable)" },
  { tag: t.definition(t.variableName), color: "var(--syn-defn)" },
  { tag: t.function(t.variableName), color: "var(--syn-defn)" },
  { tag: t.typeName, color: "var(--syn-type)" },
  { tag: t.className, color: "var(--syn-type)" },
  { tag: t.operator, color: "var(--syn-operator)" },
  { tag: t.punctuation, color: "var(--syn-operator)" },
  { tag: t.separator, color: "var(--syn-operator)" },
  { tag: t.regexp, color: "var(--syn-regexp)" },
  { tag: t.escape, color: "var(--syn-escape)" },
  { tag: t.url, color: "var(--syn-string)", textDecoration: "underline" },
  { tag: t.invalid, color: "var(--syn-invalid)" },
]);

/**
 * Single highlight extension — CSS variables auto-switch between
 * dark and light modes so no Compartment swap is needed.
 */
export function getHighlightExtension(_light?: boolean): Extension {
  return syntaxHighlighting(highlight);
}
