import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useState } from "react";
import { catalogKeys } from "../../hooks/useCatalog";
import { getBackend } from "../../lib/api";
import type { ImportReport, ImportSource } from "../../lib/importTypes";
import { errorMessage } from "../../lib/types";
import { Button } from "../ui/Button";
import { Dialog } from "../ui/Dialog";
import { ImportPreview } from "./ImportPreview";
import { ImportReportView } from "./ImportReportView";
import { ImportSourcePicker } from "./ImportSourcePicker";
import { defaultSelection, selectionSummary } from "./importModel";

const LOCAL: ImportSource = { kind: "localNavicat" };

/** 预览不缓存:导入后再打开要重新判定「已存在」。 */
function scanQuery(source: ImportSource | null) {
  return {
    queryKey: ["import", source?.kind ?? "none", source?.kind === "ncxFile" ? source.path : ""] as const,
    queryFn: async () => (await getBackend()).scanImportSource(source ?? LOCAL),
    enabled: source !== null,
    staleTime: 0,
    gcTime: 0,
  };
}

interface ImportDialogProps {
  onClose(): void;
}

export function ImportDialog({ onClose }: ImportDialogProps) {
  const queryClient = useQueryClient();
  const local = useQuery(scanQuery(LOCAL));
  const [chosen, setChosen] = useState<ImportSource | null>(null);
  // 没选过来源时,检测到本机 Navicat 就默认用它。
  const source = chosen ?? (local.data?.length ? LOCAL : null);
  const preview = useQuery(scanQuery(source));
  const candidates = preview.data ?? [];
  const [picked, setPicked] = useState<Set<string> | null>(null);
  const selection = picked ?? defaultSelection(candidates);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string>();
  const [result, setResult] = useState<{ report: ImportReport; source: ImportSource } | null>(null);

  const choose = (next: ImportSource) => {
    setChosen(next);
    setPicked(null);
    setError(undefined);
  };
  const pickNcx = async () => {
    try {
      const path = await (await getBackend()).pickNcxFile();
      if (path) choose({ kind: "ncxFile", path });
    } catch (e) {
      setError(errorMessage(e));
    }
  };

  const submit = async () => {
    if (!source) return;
    setBusy(true);
    setError(undefined);
    try {
      const ids = candidates.filter((c) => selection.has(c.id)).map((c) => c.id);
      const report = await (await getBackend()).importConnections(source, ids);
      await queryClient.invalidateQueries({ queryKey: catalogKeys.connections });
      setResult({ report, source });
    } catch (e) {
      setError(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  // 导入进行中不让关:后端照样会写完,关掉就看不到汇报了。
  const close = busy ? () => {} : onClose;

  if (result) {
    return (
      <Dialog
        open
        title="导入连接"
        width={560}
        onClose={onClose}
        footer={
          <>
            <span className="footer-spacer" />
            <Button variant="primary" onClick={onClose} autoFocus>
              完成
            </Button>
          </>
        }
      >
        <ImportReportView report={result.report} source={result.source} />
      </Dialog>
    );
  }

  const scanError = preview.error ? errorMessage(preview.error) : undefined;
  return (
    <Dialog
      open
      title="导入连接"
      subtitle="从 Navicat 迁移连接配置；只新增连接，不改动已有的。"
      width={820}
      onClose={close}
      footer={
        <>
          <span className="import-footer-summary">
            {source && preview.data ? selectionSummary(candidates, selection) : ""}
          </span>
          <span className="footer-spacer" />
          <Button variant="ghost" disabled={busy} onClick={onClose}>
            取消
          </Button>
          {/* 预览就绪时重新挂载并取得焦点:否则打开对话框时焦点落在来源卡片上,看着像被选中了 */}
          <Button
            key={preview.data ? "ready" : "loading"}
            variant="primary"
            busy={busy}
            disabled={!preview.data || selection.size === 0}
            autoFocus={Boolean(preview.data)}
            onClick={() => void submit()}
          >
            {`导入 ${selection.size} 个连接`}
          </Button>
        </>
      }
    >
      <div className="import-body">
        <ImportSourcePicker
          source={source}
          local={{ pending: local.isPending, count: local.data?.length, error: local.error }}
          onChooseLocal={() => choose(LOCAL)}
          onPickNcx={() => void pickNcx()}
        />
        {source?.kind === "localNavicat" && (
          <p className="import-hint">
            本机 Navicat 的密码存在它自己的钥匙串里，Basalt 读不到，导入后第一次打开时再输入。 想连密码一起迁移，请在
            Navicat 里「文件 → 导出连接」并勾选「导出密码」，再选择导出的 .ncx 文件。
          </p>
        )}
        {scanError && <p className="test-result is-error">{scanError}</p>}
        {preview.isFetching && !preview.data && <p className="import-placeholder">正在读取连接…</p>}
        {preview.data && <ImportPreview candidates={candidates} selection={selection} onSelectionChange={setPicked} />}
        {!source && !local.isPending && (
          <p className="import-placeholder">
            {local.error ? `${errorMessage(local.error)}。` : ""}选择一个来源，预览要导入的连接。
          </p>
        )}
        {error && <p className="test-result is-error">{error}</p>}
      </div>
    </Dialog>
  );
}
