import { autocompletion, closeBrackets, closeBracketsKeymap, completionKeymap } from "@codemirror/autocomplete";
import { defaultKeymap, history, historyKeymap, indentWithTab } from "@codemirror/commands";
import {
  MariaSQL,
  MySQL,
  PostgreSQL,
  SQLite,
  keywordCompletionSource,
  sql,
  type SQLDialect,
} from "@codemirror/lang-sql";
import { bracketMatching, indentOnInput } from "@codemirror/language";
import { highlightSelectionMatches, searchKeymap } from "@codemirror/search";
import { Compartment, EditorState, Prec } from "@codemirror/state";
import {
  EditorView,
  drawSelection,
  highlightActiveLine,
  highlightActiveLineGutter,
  keymap,
  lineNumbers,
  placeholder,
} from "@codemirror/view";
import { useEffect, useImperativeHandle, useRef, type Ref } from "react";
import type { DriverKind } from "../../lib/types";
import { editorTheme } from "./editorTheme";
import { schemaCompletion, type CompletionProvider } from "./sqlCompletion";
import type { SqlDialect } from "./splitStatements";

export interface SqlEditorHandle {
  getText(): string;
  /** 选中的范围;没有选中时 from === to。 */
  getSelection(): { from: number; to: number };
  /** 替换选中的文字(没选中就在光标处插入),并把光标放到插入内容之后。 */
  insert(text: string): void;
  focus(): void;
}

interface SqlEditorProps {
  ref?: Ref<SqlEditorHandle>;
  driver: DriverKind | null;
  initialText: string;
  label: string;
  completion: CompletionProvider;
  onChange(text: string): void;
  onRun(): void;
  onRunAll(): void;
  onCancel(): void;
}

export function editorDialect(driver: DriverKind | null): SqlDialect {
  if (driver === "postgres") return "postgres";
  if (driver === "sqlite") return "sqlite";
  return "mysql";
}

function codeMirrorDialect(driver: DriverKind | null): SQLDialect {
  switch (driver) {
    case "postgres":
      return PostgreSQL;
    case "sqlite":
      return SQLite;
    case "mariadb":
      return MariaSQL;
    default:
      return MySQL;
  }
}

function language(driver: DriverKind | null, provider: CompletionProvider) {
  const dialect = codeMirrorDialect(driver);
  return [
    sql({ dialect, upperCaseKeywords: true }),
    autocompletion({
      override: [schemaCompletion(provider, editorDialect(driver)), keywordCompletionSource(dialect, true)],
      icons: false,
    }),
  ];
}

const contentAttributes = (label: string) =>
  EditorView.contentAttributes.of({
    "aria-label": label,
    spellcheck: "false",
    autocorrect: "off",
    autocapitalize: "off",
  });

/** CodeMirror 6 的 SQL 编辑器。EditorView 只建一次;回调走 ref,换方言走 Compartment。 */
export function SqlEditor({ ref, driver, initialText, label, completion, ...callbacks }: SqlEditorProps) {
  const host = useRef<HTMLDivElement>(null);
  const view = useRef<EditorView | null>(null);
  const languageSlot = useRef(new Compartment());
  const labelSlot = useRef(new Compartment());
  const latest = useRef(callbacks);
  useEffect(() => {
    latest.current = callbacks;
  });
  const initial = useRef({ initialText, driver, completion, label });

  useEffect(() => {
    if (!host.current) return;
    const { initialText: text, driver: firstDriver, completion: provider, label: name } = initial.current;
    const shortcuts = Prec.highest(
      keymap.of([
        { key: "Mod-Enter", run: () => (latest.current.onRun(), true) },
        { key: "Shift-Mod-Enter", run: () => (latest.current.onRunAll(), true) },
        { key: "Mod-.", run: () => (latest.current.onCancel(), true) },
      ]),
    );
    const editor = new EditorView({
      parent: host.current,
      state: EditorState.create({
        doc: text,
        extensions: [
          shortcuts,
          lineNumbers(),
          highlightActiveLineGutter(),
          history(),
          drawSelection(),
          indentOnInput(),
          bracketMatching(),
          closeBrackets(),
          highlightActiveLine(),
          highlightSelectionMatches(),
          placeholder("在这里写 SQL，⌘↩ 执行光标所在的语句"),
          keymap.of([
            ...closeBracketsKeymap,
            ...defaultKeymap,
            ...searchKeymap,
            ...historyKeymap,
            ...completionKeymap,
            indentWithTab,
          ]),
          languageSlot.current.of(language(firstDriver, provider)),
          editorTheme,
          labelSlot.current.of(contentAttributes(name)),
          EditorView.updateListener.of((update) => {
            if (update.docChanged) latest.current.onChange(update.state.doc.toString());
          }),
        ],
      }),
    });
    view.current = editor;
    return () => {
      editor.destroy();
      view.current = null;
    };
  }, []);

  // 保存后标签页改了名,读屏念的名字跟着改。
  useEffect(() => {
    view.current?.dispatch({ effects: labelSlot.current.reconfigure(contentAttributes(label)) });
  }, [label]);

  // 连接列表加载完才知道是哪种库:到时换一下方言。
  useEffect(() => {
    view.current?.dispatch({ effects: languageSlot.current.reconfigure(language(driver, completion)) });
  }, [driver, completion]);

  useImperativeHandle(
    ref,
    () => ({
      getText: () => view.current?.state.doc.toString() ?? "",
      getSelection: () => {
        const main = view.current?.state.selection.main;
        return { from: main?.from ?? 0, to: main?.to ?? 0 };
      },
      insert: (text) => {
        const editor = view.current;
        if (!editor) return;
        const { from, to } = editor.state.selection.main;
        editor.dispatch({ changes: { from, to, insert: text }, selection: { anchor: from + text.length } });
        editor.focus();
      },
      focus: () => view.current?.focus(),
    }),
    [],
  );

  return <div ref={host} className="sql-editor" />;
}
