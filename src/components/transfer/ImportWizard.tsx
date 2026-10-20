import { useQueryClient } from "@tanstack/react-query";
import { Download } from "lucide-react";
import { useRef, useState } from "react";
import { catalogKeys } from "../../hooks/useCatalog";
import { tableDataKeys } from "../../hooks/useTableData";
import { getBackend } from "../../lib/api";
import type { ColumnMapping, ImportOptions, ImportPreview, ImportReport, SourceSpec } from "../../lib/transferTypes";
import { errorMessage } from "../../lib/types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { ImportFileStep } from "./ImportFileStep";
import { ImportMappingStep } from "./ImportMappingStep";
import { ImportOptionsStep } from "./ImportOptionsStep";
import { ImportResultView } from "./ImportResultView";
import {
  DEFAULT_IMPORT_OPTIONS,
  STEPS,
  commitNote,
  failuresFileName,
  mappedCount,
  type ImportTarget,
  type WizardStep,
} from "./importModel";
import { TransferProgressView } from "./TransferProgressView";
import { useTransferJob } from "./useTransferJob";

export function ImportWizard({ target, onClose }: { target: ImportTarget; onClose(): void }) {
  const queryClient = useQueryClient();
  const [step, setStep] = useState<WizardStep>("file");
  const [spec, setSpec] = useState<SourceSpec | null>(null);
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [mapping, setMapping] = useState<ColumnMapping[]>([]);
  const [options, setOptions] = useState<ImportOptions>(DEFAULT_IMPORT_OPTIONS);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [confirmClear, setConfirmClear] = useState(false);
  const [report, setReport] = useState<ImportReport | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const { job, run, cancel } = useTransferJob();
  const running = job.status === "running";
  // 每读一次预览加一:用户连着改选项时,迟到的旧结果不能盖掉新的。
  const token = useRef(0);

  const inspect = async (next: SourceSpec) => {
    const mine = ++token.current;
    setSpec(next);
    setLoading(true);
    setError(null);
    try {
      const result = await (await getBackend()).previewImport(target.connectionId, target.table, next);
      if (mine !== token.current) return;
      setPreview(result);
      setMapping(result.mapping);
      setSpec({ ...next, format: result.source.format });
    } catch (e) {
      if (mine === token.current) {
        setPreview(null);
        setError(errorMessage(e));
      }
    } finally {
      if (mine === token.current) setLoading(false);
    }
  };

  const pick = async () => {
    try {
      const path = await (await getBackend()).pickImportFile();
      if (path) await inspect({ path, format: null, hasHeader: spec?.hasHeader ?? true });
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const start = async () => {
    if (!spec || !preview) return;
    setConfirmClear(false);
    setError(null);
    const request = { connectionId: target.connectionId, table: target.table, source: spec, mapping, options };
    try {
      setReport(await run((backend, jobId) => backend.importData(jobId, request)));
      // 表里的数据变了:开着的表数据页、对象列表的行数都要重读。
      void queryClient.invalidateQueries({ queryKey: tableDataKeys.table(target.connectionId, target.table) });
      void queryClient.invalidateQueries({ queryKey: catalogKeys.connection(target.connectionId) });
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const saveFailures = async () => {
    if (!report) return;
    try {
      const backend = await getBackend();
      const path = await backend.pickSavePath(failuresFileName(target.table), { name: "CSV", extensions: ["csv"] });
      if (!path) return;
      await backend.saveImportFailures(path, report.failures);
      setSaved(path);
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  if (report) {
    return (
      <Dialog
        open
        title="导入数据"
        subtitle={target.title}
        width={760}
        onClose={onClose}
        footer={
          <>
            {report.failures.length > 0 && (
              <Button variant="secondary" icon={<Download />} onClick={() => void saveFailures()}>
                导出失败明细…
              </Button>
            )}
            <span className="dialog-footer-hint">{saved ? `已保存到 ${saved}` : (error ?? "")}</span>
            <span className="footer-spacer" />
            <Button variant="primary" autoFocus onClick={onClose}>
              完成
            </Button>
          </>
        }
      >
        <ImportResultView report={report} />
      </Dialog>
    );
  }

  const index = STEPS.findIndex((s) => s.step === step);
  const canNext = step === "file" ? Boolean(preview) && !loading : mappedCount(mapping) > 0;
  return (
    <Dialog
      open
      title="导入数据"
      subtitle={target.title}
      width={760}
      onClose={running ? () => undefined : onClose}
      footer={
        running ? undefined : (
          <>
            <ol className="transfer-steps" aria-label="步骤">
              {STEPS.map((s, i) => (
                <li key={s.step} className={i === index ? "is-current" : i < index ? "is-done" : undefined}>
                  <span className="transfer-step-number mono">{i + 1}</span>
                  {s.label}
                </li>
              ))}
            </ol>
            <span className="footer-spacer" />
            {index === 0 ? (
              <Button variant="ghost" onClick={onClose}>
                取消
              </Button>
            ) : (
              <Button variant="ghost" onClick={() => setStep(STEPS[index - 1]!.step)}>
                上一步
              </Button>
            )}
            {step === "options" ? (
              <Button
                variant={options.clearTable ? "danger" : "primary"}
                onClick={() => (options.clearTable ? setConfirmClear(true) : void start())}
              >
                开始导入
              </Button>
            ) : (
              <Button variant="primary" disabled={!canNext} onClick={() => setStep(STEPS[index + 1]!.step)}>
                下一步
              </Button>
            )}
          </>
        )
      }
    >
      {running ? (
        <TransferProgressView job={job} verb="已处理" onCancel={() => void cancel()} />
      ) : (
        <>
          {step === "file" && (
            <ImportFileStep
              spec={spec}
              preview={preview}
              loading={loading}
              onPick={() => void pick()}
              onSpecChange={(next) => void inspect(next)}
            />
          )}
          {step === "mapping" && preview && (
            <ImportMappingStep preview={preview} mapping={mapping} onChange={setMapping} />
          )}
          {step === "options" && (
            <ImportOptionsStep
              options={options}
              driver={target.driver}
              tableTitle={target.title}
              onChange={setOptions}
            />
          )}
          {error && (
            <p className="transfer-error" role="alert">
              {error}
            </p>
          )}
        </>
      )}
      {confirmClear && (
        <Dialog
          open
          title="清空表再导入？"
          width={440}
          onClose={() => setConfirmClear(false)}
          footer={
            <>
              <span className="footer-spacer" />
              <Button variant="ghost" autoFocus onClick={() => setConfirmClear(false)}>
                取消
              </Button>
              <Button variant="danger" onClick={() => void start()}>
                清空并导入
              </Button>
            </>
          }
        >
          <p className="transfer-confirm">
            会先删掉「{target.title}」里现有的全部行，再导入文件里的数据。{commitNote(target.driver)}
          </p>
        </Dialog>
      )}
    </Dialog>
  );
}
