import { CircleCheck, KeyRound, TriangleAlert } from "lucide-react";
import type { ImportedConnection, ImportReport, ImportSource } from "../../lib/importTypes";
import { reportHeadline } from "./importModel";

interface ImportReportViewProps {
  report: ImportReport;
  source: ImportSource;
}

function passwordNote(item: ImportedConnection): string {
  if (item.driver === "sqlite") return "无需密码";
  return item.passwordSaved ? "密码已存入钥匙串" : "首次打开时输入密码";
}

export function ImportReportView({ report, source }: ImportReportViewProps) {
  return (
    <div className="import-report">
      <p className="import-report-headline">
        <CircleCheck aria-hidden />
        {reportHeadline(report)}
      </p>
      {report.imported.length > 0 && (
        <ul className="import-report-list">
          {report.imported.map((item) => (
            <li key={item.id}>
              <span className="import-report-name">{item.name}</span>
              <span className={item.passwordSaved ? "import-report-meta is-saved" : "import-report-meta"}>
                {item.passwordSaved && <KeyRound aria-hidden />}
                {passwordNote(item)}
              </span>
            </li>
          ))}
        </ul>
      )}
      {report.passwordFilled.length > 0 && (
        <>
          <h3 className="import-report-subtitle">补上密码</h3>
          <ul className="import-report-list">
            {report.passwordFilled.map((item) => (
              <li key={item.id}>
                <span className="import-report-name">{item.name}</span>
                <span className="import-report-meta is-saved">
                  <KeyRound aria-hidden />
                  密码已存入钥匙串
                </span>
              </li>
            ))}
          </ul>
        </>
      )}
      {report.skipped.length > 0 && (
        <>
          <h3 className="import-report-subtitle">跳过</h3>
          <ul className="import-report-list">
            {report.skipped.map((item, index) => (
              <li key={`${index}-${item.name}`}>
                <span className="import-report-name">{item.name}</span>
                <span className="import-report-meta">{item.reason}</span>
              </li>
            ))}
          </ul>
        </>
      )}
      {source.kind === "ncxFile" && (
        <div className="import-reminder" role="note">
          <TriangleAlert aria-hidden />
          <p>
            .ncx 文件里的密码可以被还原，导入完成后请删除它：
            <code>{source.path}</code>
          </p>
        </div>
      )}
    </div>
  );
}
