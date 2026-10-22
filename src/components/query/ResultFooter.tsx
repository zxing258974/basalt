import { Lock } from "lucide-react";
import { formatCount } from "../../lib/format";
import { shortcut } from "../../lib/platform";
import { EditActions, type EditActionsProps } from "../data/EditActions";
import { Button } from "../ui/Button";

interface ResultFooterProps extends EditActionsProps {
  loaded: number;
  total: number;
  /** 结果太大,后端只存了前 total 行。 */
  truncated: boolean;
  loading: boolean;
  error: string | null;
  onRetry(): void;
  /** 不能改的原因;能改时为 null。 */
  readOnlyReason: string | null;
  /** 最近一次提交的结果。 */
  message: string | null;
}

/** 结果表格底下:取到了多少行、能不能改,能改时放增删行、撤销、提交。 */
export function ResultFooter(props: ResultFooterProps) {
  const { loaded, total, pending } = props;
  const count =
    loaded >= total ? `${formatCount(total)} 行` : `已显示 ${formatCount(loaded)} / 共 ${formatCount(total)} 行`;
  return (
    <footer className="result-footer">
      <span className="result-footer-count mono">
        {count}
        {props.truncated && "（结果太大，只保留了这些）"}
      </span>
      {props.loading && <span className="result-footer-loading">正在读取…</span>}
      {props.error && (
        <span className="result-footer-error" role="alert">
          读取后面的行失败：{props.error}
          <Button variant="ghost" size="sm" onClick={props.onRetry}>
            重试
          </Button>
        </span>
      )}
      <span className="result-footer-status">
        {props.readOnlyReason !== null ? (
          <span className="result-footer-readonly" title={props.readOnlyReason}>
            <Lock aria-hidden />
            {props.readOnlyReason}
          </span>
        ) : pending > 0 ? (
          <span className="data-footer-pending">
            {pending} 行有未提交的修改 · <kbd>{shortcut("S")}</kbd> 预览并提交
          </span>
        ) : props.message ? (
          <span className="data-footer-message" role="status">
            {props.message}
          </span>
        ) : null}
      </span>
      {props.readOnlyReason === null && (
        <div className="data-actions">
          <EditActions {...props} />
        </div>
      )}
    </footer>
  );
}
