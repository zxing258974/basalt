import type { ReactNode } from "react";

/** 内容区没有可显示内容时的占位:加载中、出错、为空、连接已关闭。 */
export function EmptyState({
  title,
  detail,
  tone,
  action,
}: {
  title: string;
  detail?: string;
  tone?: "error";
  action?: ReactNode;
}) {
  return (
    <div className={tone === "error" ? "empty-state is-error" : "empty-state"}>
      <p className="empty-state-title">{title}</p>
      {detail && <p className="empty-state-detail selectable">{detail}</p>}
      {action}
    </div>
  );
}
