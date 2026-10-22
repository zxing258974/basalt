import { CircleStop, FastForward, History, Paintbrush, Play, Save, Sparkles } from "lucide-react";
import type { QueryScope } from "../../lib/queryTypes";
import type { ConnectionView } from "../../lib/types";
import { DriverBadge } from "../connection/DriverBadge";
import { Button } from "../ui/Button";
import { ScopePicker } from "./ScopePicker";
import { shortcut } from "../../lib/platform";

interface QueryHeaderProps {
  connection: ConnectionView | undefined;
  scope: QueryScope;
  title: string;
  dirty: boolean;
  running: boolean;
  cancelling: boolean;
  historyOpen: boolean;
  assistantOpen: boolean;
  onScopeChange(scope: QueryScope): void;
  onRun(): void;
  onRunAll(): void;
  onCancel(): void;
  onSave(): void;
  onFormat(): void;
  onToggleHistory(): void;
  onToggleAssistant(): void;
}

/** 查询标签页的顶栏:在哪个连接、哪个库,执行、取消、保存、美化、历史、助手。 */
export function QueryHeader(props: QueryHeaderProps) {
  const { connection, running } = props;
  const ready = Boolean(connection?.isOpen);
  return (
    <header className="query-header">
      <div className="query-heading">
        {connection && <DriverBadge driver={connection.driver} />}
        <span className="data-crumb">{connection?.name ?? "已关闭的连接"}</span>
        <span className="data-crumb-sep">/</span>
        {connection ? (
          <ScopePicker
            connection={connection}
            scope={props.scope}
            disabled={!ready || running}
            onChange={props.onScopeChange}
          />
        ) : null}
        <span className="data-crumb-sep">/</span>
        <h1 className="data-title selectable">
          {props.title}
          {props.dirty && (
            <span className="query-dirty" title="有没保存的改动">
              <span className="visually-hidden">（未保存）</span>
            </span>
          )}
        </h1>
      </div>
      <div className="query-actions">
        <Button
          variant="primary"
          size="sm"
          icon={<Play />}
          disabled={!ready || running}
          title={`执行光标所在的语句；有选中文字时执行选中的部分（${shortcut("Enter")}）`}
          onClick={props.onRun}
        >
          执行
        </Button>
        <Button
          size="sm"
          icon={<FastForward />}
          disabled={!ready || running}
          title={`按顺序执行全部语句，出错就停（${shortcut("Enter", { shift: true })}）`}
          onClick={props.onRunAll}
        >
          全部执行
        </Button>
        <Button
          variant="danger"
          size="sm"
          icon={<CircleStop />}
          disabled={!running || props.cancelling}
          busy={props.cancelling && running}
          title={`中止正在执行的语句（${shortcut(".")}）`}
          onClick={props.onCancel}
        >
          取消
        </Button>
        <span className="toolbar-divider" aria-hidden />
        <Button
          className="query-tool"
          variant="ghost"
          size="sm"
          icon={<Save />}
          title={`保存查询（${shortcut("S")}）`}
          onClick={props.onSave}
        >
          保存
        </Button>
        <Button
          className="query-tool"
          variant="ghost"
          size="sm"
          icon={<Paintbrush />}
          title={`美化 SQL：有选中的文字时只美化选中的部分（${shortcut("F", { shift: true })}）`}
          onClick={props.onFormat}
        >
          美化
        </Button>
        <Button
          className="query-tool"
          variant="ghost"
          size="sm"
          icon={<History />}
          aria-pressed={props.historyOpen}
          title="执行历史"
          onClick={props.onToggleHistory}
        >
          历史
        </Button>
        <Button
          className="query-tool"
          variant="ghost"
          size="sm"
          icon={<Sparkles />}
          aria-pressed={props.assistantOpen}
          title="SQL 助手：按问题生成 SQL、优化当前 SQL（不会执行）"
          onClick={props.onToggleAssistant}
        >
          助手
        </Button>
      </div>
    </header>
  );
}
