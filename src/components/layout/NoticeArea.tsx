import { CircleAlert, X } from "lucide-react";
import { useEffect } from "react";
import { useUi } from "../../stores/ui";

const NOTICE_LIFETIME_MS = 6000;

/** 状态栏里最近一条操作失败提示,几秒后自动消失。 */
export function NoticeArea() {
  const notice = useUi((s) => s.notice);
  const dismiss = useUi((s) => s.dismissNotice);

  useEffect(() => {
    if (!notice) return;
    const timer = window.setTimeout(() => dismiss(notice.id), NOTICE_LIFETIME_MS);
    return () => window.clearTimeout(timer);
  }, [notice, dismiss]);

  if (!notice) return null;
  return (
    <div className="notice" role="alert">
      <CircleAlert aria-hidden />
      <span className="notice-message selectable" title={notice.message}>
        {notice.message}
      </span>
      <button className="notice-close" aria-label="关闭提示" onClick={() => dismiss(notice.id)}>
        <X />
      </button>
    </div>
  );
}
