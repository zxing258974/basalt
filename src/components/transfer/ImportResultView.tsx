import { CircleAlert, CircleCheck, TriangleAlert } from "lucide-react";
import type { ImportReport } from "../../lib/transferTypes";
import { formatCount } from "./exportModel";
import { reportTitle, reportTone } from "./importModel";

/** 明细表里最多列这么多条,其余导出成 CSV 看。 */
export const SHOWN_FAILURES = 200;

const ICONS = { success: CircleCheck, warning: TriangleAlert, error: CircleAlert };

export function ImportResultView({ report }: { report: ImportReport }) {
  const tone = reportTone(report);
  const Icon = ICONS[tone];
  const shown = report.failures.slice(0, SHOWN_FAILURES);
  return (
    <div className="transfer-body">
      <div className={`transfer-result is-${tone}`}>
        <Icon className="transfer-result-icon" aria-hidden />
        <div>
          <p className="transfer-result-title">{reportTitle(report)}</p>
          <p className="transfer-result-summary selectable">{report.summary}</p>
          <p className="transfer-result-stats">
            <span>
              生效 <strong className="mono">{formatCount(report.inserted)}</strong> 行
            </span>
            <span>
              失败 <strong className="mono">{formatCount(report.failed)}</strong> 行
            </span>
          </p>
        </div>
      </div>
      {shown.length > 0 && (
        <>
          <div className="transfer-preview-scroll">
            <table className="transfer-preview transfer-failures" aria-label="失败的行">
              <thead>
                <tr>
                  <th scope="col">行号</th>
                  <th scope="col">列</th>
                  <th scope="col">值</th>
                  <th scope="col">原因</th>
                </tr>
              </thead>
              <tbody>
                {shown.map((failure, index) => (
                  <tr key={index}>
                    <td className="mono">{failure.row}</td>
                    <td>{failure.column ?? "（整行）"}</td>
                    <td className={failure.value === null ? "is-null" : "mono"} title={failure.value ?? undefined}>
                      {failure.value ?? "—"}
                    </td>
                    <td className="transfer-failure-reason selectable">{failure.reason}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {report.failed > shown.length && (
            <p className="transfer-note">
              这里只列出前 {formatCount(shown.length)} 条，全部 {formatCount(report.failed)} 条请导出成 CSV 查看
              {report.failed > report.failures.length
                ? `（报告里带回了前 ${formatCount(report.failures.length)} 条）`
                : ""}
              。
            </p>
          )}
        </>
      )}
    </div>
  );
}
