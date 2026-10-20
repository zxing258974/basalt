import { Braces, FileCode, FileSpreadsheet, FileText } from "lucide-react";
import type { ReactNode } from "react";
import type { BinaryEncoding, CsvOptions, ExportFormatKind, TextEncoding } from "../../lib/transferTypes";
import { Checkbox } from "../ui/Field";
import { BINARY_CHOICES, DELIMITERS, ENCODINGS, FORMATS, type FormatChoices } from "./exportModel";
import { NumberField } from "./NumberField";

const ICONS: Record<ExportFormatKind, ReactNode> = {
  csv: <FileText />,
  xlsx: <FileSpreadsheet />,
  json: <Braces />,
  sql: <FileCode />,
};

/** 带标签的下拉框。 */
export function Choice<T extends string>({
  label,
  value,
  options,
  onChange,
}: {
  label: string;
  value: T;
  options: readonly { value: T; label: string }[];
  onChange(value: T): void;
}) {
  return (
    <label className="transfer-field">
      <span className="transfer-field-label">{label}</span>
      <select className="transfer-select" value={value} onChange={(event) => onChange(event.target.value as T)}>
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.label}
          </option>
        ))}
      </select>
    </label>
  );
}

interface ExportOptionsFormProps {
  choices: FormatChoices;
  onChange(next: FormatChoices): void;
  binary: BinaryEncoding;
  onBinaryChange(binary: BinaryEncoding): void;
}

export function ExportOptionsForm({ choices, onChange, binary, onBinaryChange }: ExportOptionsFormProps) {
  const setCsv = (patch: Partial<CsvOptions>) => onChange({ ...choices, csv: { ...choices.csv, ...patch } });
  return (
    <>
      <div className="transfer-formats" role="radiogroup" aria-label="格式">
        {FORMATS.map((format) => (
          <button
            key={format.kind}
            type="button"
            role="radio"
            aria-checked={choices.kind === format.kind}
            className={choices.kind === format.kind ? "transfer-format is-checked" : "transfer-format"}
            onClick={() => onChange({ ...choices, kind: format.kind })}
          >
            <span className="transfer-format-icon" aria-hidden>
              {ICONS[format.kind]}
            </span>
            <span className="transfer-format-label">{format.label}</span>
            <span className="transfer-format-hint">{format.hint}</span>
          </button>
        ))}
      </div>

      <div className="transfer-options">
        {choices.kind === "csv" && (
          <>
            <Choice
              label="分隔符"
              value={choices.csv.delimiter}
              options={DELIMITERS}
              onChange={(delimiter) => setCsv({ delimiter })}
            />
            <Choice
              label="引号"
              value={choices.csv.quote}
              options={[
                { value: '"', label: '双引号 "' },
                { value: "'", label: "单引号 '" },
              ]}
              onChange={(quote) => setCsv({ quote })}
            />
            <Choice<TextEncoding>
              label="编码"
              value={choices.csv.encoding}
              options={ENCODINGS}
              onChange={(encoding) => setCsv({ encoding })}
            />
            <div className="transfer-field is-check">
              <Checkbox
                label="第一行写列名"
                checked={choices.csv.header}
                onCheckedChange={(header) => setCsv({ header })}
              />
            </div>
          </>
        )}
        {choices.kind === "json" && (
          <Choice
            label="写法"
            value={choices.jsonLines ? "lines" : "array"}
            options={[
              { value: "array", label: "一个数组" },
              { value: "lines", label: "每行一个对象（JSON Lines）" },
            ]}
            onChange={(value) => onChange({ ...choices, jsonLines: value === "lines" })}
          />
        )}
        {choices.kind === "sql" && (
          <>
            <label className="transfer-field">
              <span className="transfer-field-label">表名</span>
              <input
                className="transfer-input"
                value={choices.sqlTable}
                spellCheck={false}
                onChange={(event) => onChange({ ...choices, sqlTable: event.target.value })}
              />
            </label>
            <NumberField
              label="每条 INSERT 合并"
              ariaLabel="每条 INSERT 合并的行数"
              unit="行"
              value={choices.rowsPerStatement}
              min={1}
              max={10000}
              onChange={(rowsPerStatement) => onChange({ ...choices, rowsPerStatement })}
            />
          </>
        )}
        {choices.kind !== "sql" && (
          <Choice label="二进制写成" value={binary} options={BINARY_CHOICES} onChange={onBinaryChange} />
        )}
      </div>
      <p className="transfer-note">{FORMAT_NOTES[choices.kind]}</p>
    </>
  );
}

const FORMAT_NOTES: Record<ExportFormatKind, string> = {
  csv: "NULL 写成空字段，空字符串写成一对引号，导回时分得开。数字保留全部位数。",
  xlsx: "Excel 只有 15 位有效数字：更长的整数、定点数写成文本；日期时间能精确表示时写成 Excel 日期，否则写成文本。",
  json: "整数写成 JSON 数字、位数原样；定点数写成字符串，保留全部位数；JSON 列原样嵌入。",
  sql: "按这个连接的方言给表名、列名加引号；生成列跳过。文件是 UTF-8。",
};
