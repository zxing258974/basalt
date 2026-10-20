import { FolderOpen } from "lucide-react";
import type { ImportPreview, SourceFormat, SourceSpec, TextEncoding } from "../../lib/transferTypes";
import { Button } from "../ui/Button";
import { Checkbox } from "../ui/Field";
import { Choice } from "./ExportOptionsForm";
import { fileName } from "./exportModel";
import { AUTO, DELIMITER_CHOICES, ENCODING_CHOICES, describeFormat } from "./importModel";
import { PreviewTable } from "./PreviewTable";

interface ImportFileStepProps {
  spec: SourceSpec | null;
  preview: ImportPreview | null;
  loading: boolean;
  onPick(): void;
  /** 改了编码、分隔符、工作表或「第一行是列名」:重新读预览。 */
  onSpecChange(spec: SourceSpec): void;
}

export function ImportFileStep({ spec, preview, loading, onPick, onSpecChange }: ImportFileStepProps) {
  const format = preview?.source.format ?? null;
  const setFormat = (next: SourceFormat) => spec && onSpecChange({ ...spec, format: next });
  return (
    <div className="transfer-body">
      <div className="transfer-file">
        <Button variant="secondary" icon={<FolderOpen />} onClick={onPick}>
          {spec ? "换一个文件…" : "选择文件…"}
        </Button>
        {spec ? (
          <div className="transfer-file-text">
            <span className="transfer-file-name">{fileName(spec.path)}</span>
            <span className="transfer-file-meta" title={spec.path}>
              {format ? describeFormat(format) : loading ? "正在读取…" : spec.path}
            </span>
          </div>
        ) : (
          <span className="transfer-note">CSV（.csv、.tsv、.txt）或 Excel（.xlsx、.xls）。</span>
        )}
      </div>

      {spec && format && (
        <div className="transfer-options is-three">
          {format.kind === "csv" ? (
            <>
              <Choice<TextEncoding | typeof AUTO>
                label="编码"
                value={format.encoding ?? AUTO}
                options={ENCODING_CHOICES}
                onChange={(value) => setFormat({ ...format, encoding: value === AUTO ? null : value })}
              />
              <Choice
                label="分隔符"
                value={format.delimiter ?? AUTO}
                options={DELIMITER_CHOICES}
                onChange={(value) => setFormat({ ...format, delimiter: value === AUTO ? null : value })}
              />
            </>
          ) : (
            <Choice
              label="工作表"
              value={format.sheet ?? ""}
              options={(preview?.source.sheets ?? []).map((sheet) => ({ value: sheet, label: sheet }))}
              onChange={(sheet) => setFormat({ kind: "excel", sheet })}
            />
          )}
          <div className="transfer-field is-check">
            <Checkbox
              label="第一行是列名"
              checked={spec.hasHeader}
              onCheckedChange={(hasHeader) => onSpecChange({ ...spec, hasHeader })}
            />
          </div>
        </div>
      )}

      {preview && (
        <PreviewTable
          label="文件预览"
          headers={preview.source.headers}
          rows={preview.source.rows}
          rowNumbers={preview.source.rowNumbers}
        />
      )}
      {spec && !preview && loading && <p className="transfer-placeholder">正在读取文件…</p>}
    </div>
  );
}
