import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { CircleAlert, Copy } from "lucide-react";
import { useEffect, useState } from "react";
import { getBackend } from "../../lib/api";
import type { TableDesign } from "../../lib/designTypes";
import { errorMessage, type TableRef } from "../../lib/types";
import { Button } from "../ui/Button";
import { DdlStatements, modeDescription } from "./DdlStatements";

interface SqlPreviewPaneProps {
  connectionId: string;
  original: TableRef | null;
  draft: TableDesign;
}

const DEBOUNCE_MS = 250;

/** 停手一会儿再生成,打字时不会每个键都去后端跑一遍。 */
function useDebounced<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const timer = window.setTimeout(() => setSettled(value), ms);
    return () => window.clearTimeout(timer);
  }, [value, ms]);
  return settled;
}

/** SQL 预览页:按当前草稿实时生成将要执行的 DDL,只看不执行。 */
export function SqlPreviewPane({ connectionId, original, draft }: SqlPreviewPaneProps) {
  const settled = useDebounced(draft, DEBOUNCE_MS);
  const query = useQuery({
    queryKey: ["ddlPreview", connectionId, original, settled],
    queryFn: async () => (await getBackend()).previewTableDesign(connectionId, original, settled),
    staleTime: 0,
    gcTime: 0,
    placeholderData: keepPreviousData,
  });
  const preview = query.data;
  const script = preview?.statements.map((s) => `${s};`).join("\n\n") ?? "";
  const [copied, setCopied] = useState<string | null>(null);

  return (
    <div className="designer-pane sql-pane">
      {query.isPending && !preview && (
        <p className="preview-loading">
          <span className="spinner" aria-hidden /> 正在生成…
        </p>
      )}
      {query.error && !query.isFetching && (
        <div className="preview-error" role="alert">
          <CircleAlert aria-hidden />
          <div>
            <p className="preview-error-title">现在还生成不了 SQL</p>
            <p className="preview-error-message selectable">{errorMessage(query.error)}</p>
          </div>
        </div>
      )}
      {preview && !query.error && preview.statements.length === 0 && (
        <p className="designer-empty">和库里的结构一样，没有要执行的修改。</p>
      )}
      {preview && !query.error && preview.statements.length > 0 && (
        <>
          <div className="sql-pane-head">
            <p className="sql-pane-mode">{modeDescription(preview.mode, preview.statements.length)}</p>
            <Button
              size="sm"
              variant="ghost"
              icon={<Copy />}
              onClick={() => {
                void navigator.clipboard
                  ?.writeText(script)
                  .then(() => setCopied(script))
                  .catch((error: unknown) => console.warn("复制 SQL 失败", error));
              }}
            >
              {copied === script ? "已复制" : "复制"}
            </Button>
          </div>
          <DdlStatements statements={preview.statements} />
        </>
      )}
    </div>
  );
}
