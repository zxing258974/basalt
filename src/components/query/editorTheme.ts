// CodeMirror 的外观:全部取设计令牌,深浅主题跟着 CSS 变量自动切换,不用两套主题。
import { HighlightStyle, syntaxHighlighting } from "@codemirror/language";
import { EditorView } from "@codemirror/view";
import { tags } from "@lezer/highlight";

const theme = EditorView.theme({
  "&": {
    height: "100%",
    color: "var(--text-strong)",
    backgroundColor: "var(--surface-canvas)",
    fontSize: "var(--text-md)",
  },
  "&.cm-focused": { outline: "none" },
  ".cm-scroller": { fontFamily: "var(--font-mono)", lineHeight: "1.6" },
  ".cm-content": { padding: "var(--space-4) 0", caretColor: "var(--accent)" },
  ".cm-line": { padding: "0 var(--space-6) 0 var(--space-4)" },
  ".cm-cursor, .cm-dropCursor": { borderLeftColor: "var(--accent)", borderLeftWidth: "2px" },
  "&.cm-focused > .cm-scroller > .cm-selectionLayer .cm-selectionBackground, .cm-selectionBackground, ::selection": {
    backgroundColor: "var(--surface-selected)",
  },
  ".cm-sql-error": {
    textDecoration: "underline wavy var(--danger)",
    textDecorationSkipInk: "none",
    textUnderlineOffset: "3px",
    backgroundColor: "var(--danger-soft)",
    borderRadius: "var(--radius-xs)",
  },
  ".cm-activeLine": { backgroundColor: "color-mix(in oklch, var(--surface-hover) 45%, transparent)" },
  ".cm-gutters": {
    backgroundColor: "var(--surface-canvas)",
    color: "var(--text-faint)",
    border: "none",
    borderRight: "1px solid var(--line-hairline)",
  },
  ".cm-lineNumbers .cm-gutterElement": { padding: "0 var(--space-4) 0 var(--space-5)", minWidth: "36px" },
  ".cm-activeLineGutter": { backgroundColor: "transparent", color: "var(--text-muted)" },
  ".cm-matchingBracket": {
    backgroundColor: "var(--accent-soft)",
    outline: "1px solid color-mix(in oklch, var(--accent) 40%, transparent)",
  },
  ".cm-selectionMatch": { backgroundColor: "color-mix(in oklch, var(--accent-soft) 70%, transparent)" },
  ".cm-searchMatch": { backgroundColor: "var(--accent-soft)", outline: "1px solid var(--accent)" },
  ".cm-tooltip": {
    border: "none",
    borderRadius: "var(--radius-md)",
    backgroundColor: "var(--surface-raised)",
    boxShadow: "var(--shadow-popover)",
    overflow: "hidden",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul": {
    fontFamily: "var(--font-mono)",
    fontSize: "var(--text-sm)",
    maxHeight: "16em",
    padding: "var(--space-2)",
  },
  ".cm-tooltip.cm-tooltip-autocomplete > ul > li": {
    padding: "var(--space-1) var(--space-4)",
    borderRadius: "var(--radius-xs)",
    color: "var(--text)",
  },
  ".cm-tooltip-autocomplete ul li[aria-selected]": {
    backgroundColor: "var(--surface-selected-strong)",
    color: "var(--text-on-accent)",
  },
  ".cm-completionDetail": { color: "var(--text-faint)", fontStyle: "normal", marginLeft: "var(--space-5)" },
  ".cm-tooltip-autocomplete ul li[aria-selected] .cm-completionDetail": { color: "var(--text-on-accent)" },
  ".cm-completionMatchedText": { textDecoration: "none", fontWeight: "var(--weight-semibold)" },
  ".cm-panels": { backgroundColor: "var(--surface-chrome)", color: "var(--text)" },
  ".cm-panels.cm-panels-top": { borderBottom: "1px solid var(--line)" },
  ".cm-panel.cm-search": { fontFamily: "var(--font-ui)", fontSize: "var(--text-sm)", padding: "var(--space-3)" },
  ".cm-textfield": {
    border: "1px solid var(--line-strong)",
    borderRadius: "var(--radius-sm)",
    backgroundColor: "var(--surface-raised)",
    color: "var(--text-strong)",
  },
  ".cm-button": {
    backgroundImage: "none",
    backgroundColor: "var(--surface-raised)",
    border: "1px solid var(--line-strong)",
    borderRadius: "var(--radius-sm)",
    color: "var(--text)",
  },
});

const highlight = HighlightStyle.define([
  { tag: [tags.keyword, tags.operatorKeyword, tags.modifier], color: "var(--syntax-keyword)", fontWeight: "600" },
  { tag: [tags.string, tags.special(tags.string)], color: "var(--syntax-string)" },
  { tag: [tags.number, tags.bool, tags.null], color: "var(--syntax-number)" },
  { tag: [tags.lineComment, tags.blockComment, tags.comment], color: "var(--syntax-comment)", fontStyle: "italic" },
  { tag: [tags.typeName, tags.standard(tags.name)], color: "var(--syntax-type)" },
  { tag: [tags.function(tags.name), tags.function(tags.variableName)], color: "var(--syntax-function)" },
  { tag: [tags.operator, tags.punctuation, tags.bracket], color: "var(--syntax-operator)" },
  { tag: [tags.special(tags.name), tags.labelName], color: "var(--syntax-variable)" },
  { tag: tags.quote, color: "var(--text-strong)" },
]);

export const editorTheme = [theme, syntaxHighlighting(highlight)];
