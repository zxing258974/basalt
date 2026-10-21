import { FolderOpen } from "lucide-react";
import type { ImportPreview, SourceFormat, SourceSpec, TextEncoding } from "../../lib/transferTypes";
import { Button } from "../ui/Button";
import { Checkbox } from "../ui/Field";
import { Choice } from "./ExportOptionsForm";
import { fileName } from "./exportModel";
import { AUTO, DELIMITER_CHOICES, ENCODING_CHOICES, describeFormat, needsSheetChoice } from "./importModel";
import { PreviewTable } from "./PreviewTable";

interface ImportFileStepProps {
  spec: SourceSpec | null;
  preview: ImportPreview | null;
  loading: boolean;
  /** 有多个工作表时,用户已经亲手选了一个。 */
  sheetChosen: boolean;
  onPick(): void;
  /** 改了编码、分隔符、工作表或「第一行是列名」:重新读预览。 */
  onSpecChange(spec: SourceSpec): void;
  onSheetChosen(): void;
}

export function ImportFileStep({
  spec,
  preview,
  loading,
  sheetChosen,
  onPick,
  onSpecChange,
  onSheetChosen,
}: ImportFileStepProps) {
  const format = preview?.source.format ?? null;
  const setFormat = (next: SourceFormat) => spec && onSpecChange({ ...spec, format: next });
  const choosing = preview !== null && needsSheetChoice(preview.source);
  const waitingForSheet = choosing && !sheetChosen;
  // 和「已经请求的」工作表比,不和已读完的预览比:读 B 的途中点回 A,也要重新读 A。
  const requested = spec?.format?.kind === "excel" ? spec.format.sheet : null;
  const chooseSheet = (sheet: string) => {
    onSheetChosen();
    if (requested !== sheet) setFormat({ kind: "excel", sheet });
  };
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

      {choosing && format?.kind === "excel" && (
        <section className="transfer-sheets" aria-label="工作表">
          <h3 className="transfer-section-title">
            这个文件有 {preview.source.sheets.length} 个工作表{waitingForSheet ? "，先选要导入的那一个" : ""}
          </h3>
          <div className="transfer-sheet-list" role="radiogroup" aria-label="工作表">
            {preview.source.sheets.map((sheet) => {
              const checked = sheetChosen && format.sheet === sheet;
              return (
                <button
                  key={sheet}
                  type="button"
                  role="radio"
                  aria-checked={checked}
                  className={checked ? "transfer-sheet is-checked" : "transfer-sheet"}
                  onClick={() => chooseSheet(sheet)}
                >
                  {sheet}
                </button>
              );
            })}
          </div>
        </section>
      )}

      {spec && format && !waitingForSheet && (
        <div className="transfer-options is-three">
          {format.kind === "csv" && (
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

      {preview && !waitingForSheet && (
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
