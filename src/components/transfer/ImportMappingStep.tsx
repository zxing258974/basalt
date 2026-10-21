import { RotateCcw, TriangleAlert } from "lucide-react";
import type { ColumnMapping, ImportMode, ImportPreview } from "../../lib/transferTypes";
import type { DriverKind } from "../../lib/types";
import { Button } from "../ui/Button";
import {
  describeKey,
  keyProblem,
  mappedCount,
  mappedPreview,
  missingRequired,
  modeLabel,
  mysqlUpsertNote,
  setSource,
  usesKeys,
} from "./importModel";
import { PreviewTable } from "./PreviewTable";

const SKIP = "skip";

interface ImportMappingStepProps {
  preview: ImportPreview;
  mapping: ColumnMapping[];
  driver: DriverKind;
  mode: ImportMode;
  /** 匹配键(列名),更新、追加或更新、删除用。 */
  keys: string[];
  onChange(mapping: ColumnMapping[]): void;
  onKeysChange(keys: string[]): void;
}

export function ImportMappingStep({
  preview,
  mapping,
  driver,
  mode,
  keys,
  onChange,
  onKeysChange,
}: ImportMappingStepProps) {
  const { columns, source, uniqueKeys } = preview;
  const keyed = usesKeys(mode);
  // 删除只用匹配键,别的列空着也不会失败;必填提示只对插入、更新有意义。
  const missing = mode === "delete" || mode === "update" ? [] : missingRequired(columns, mapping);
  const problem = keyProblem(mode, keys, uniqueKeys, mapping);
  const mysqlNote = mysqlUpsertNote(driver, mode, uniqueKeys);
  const toggleKey = (column: string, on: boolean) =>
    onKeysChange(on ? [...keys, column] : keys.filter((k) => k !== column));
  const shown = mappedPreview(source, mapping);
  const typeOf = (name: string) => columns.find((c) => c.name === name);
  return (
    <div className="transfer-body">
      <div className="transfer-mapping-head">
        <p className="transfer-note">
          按列名自动匹配了 {mappedCount(preview.mapping)} 列，可以逐列改；选「不导入」的列用它的默认值。
        </p>
        <Button variant="ghost" size="sm" icon={<RotateCcw />} onClick={() => onChange(preview.mapping)}>
          恢复自动匹配
        </Button>
      </div>
      <div className="transfer-mapping-scroll">
        <table className="transfer-mapping" aria-label="列映射">
          <thead>
            <tr>
              <th scope="col">目标列</th>
              <th scope="col">类型</th>
              <th scope="col">取文件里的</th>
              {keyed && <th scope="col">匹配键</th>}
            </tr>
          </thead>
          <tbody>
            {mapping.map((item) => {
              const column = typeOf(item.column);
              const required = column ? !column.nullable && !column.hasDefault : false;
              return (
                <tr key={item.column} className={item.source === null ? "is-skipped" : undefined}>
                  <th scope="row">
                    <span className="transfer-mapping-name">{item.column}</span>
                    {column?.isPrimaryKey && <span className="transfer-tag">主键</span>}
                    {required && <span className="transfer-tag is-required">必填</span>}
                  </th>
                  <td className="mono transfer-mapping-type">{column?.dataType}</td>
                  <td>
                    <select
                      className="transfer-select"
                      aria-label={`${item.column} 的来源`}
                      value={item.source === null ? SKIP : String(item.source)}
                      onChange={(event) => {
                        const value = event.target.value;
                        onChange(setSource(mapping, item.column, value === SKIP ? null : Number(value)));
                      }}
                    >
                      <option value={SKIP}>不导入</option>
                      {source.headers.map((header, index) => (
                        <option key={index} value={index}>
                          {header}
                        </option>
                      ))}
                    </select>
                  </td>
                  {keyed && (
                    <td className="transfer-mapping-key">
                      <input
                        type="checkbox"
                        aria-label={`按 ${item.column} 匹配`}
                        checked={keys.includes(item.column)}
                        onChange={(event) => toggleKey(item.column, event.target.checked)}
                      />
                    </td>
                  )}
                </tr>
              );
            })}
          </tbody>
        </table>
      </div>
      {keyed && (
        <div className="transfer-keys">
          <p className="transfer-note">
            「{modeLabel(mode)}」按勾选的匹配键找到表里对应的行
            {mode === "delete" ? "，其余列的映射不影响结果" : ""}。
          </p>
          {mode === "upsert" && (
            <div className="transfer-key-choices" role="group" aria-label="可用的键">
              {uniqueKeys.map((key) => (
                <Button
                  key={key.name ?? "primary"}
                  variant="secondary"
                  size="sm"
                  onClick={() => onKeysChange(key.columns)}
                >
                  按{describeKey(key)}
                </Button>
              ))}
            </div>
          )}
          {mysqlNote && <p className="transfer-note">{mysqlNote}</p>}
          {problem && (
            <p className="transfer-warning" role="note">
              <TriangleAlert aria-hidden />
              {problem}
            </p>
          )}
        </div>
      )}
      {missing.length > 0 && (
        <p className="transfer-warning" role="note">
          <TriangleAlert aria-hidden />
          {missing.map((c) => `「${c.name}」`).join("")}不能为空、也没有默认值，不选来源的话每一行都会失败。
        </p>
      )}
      <h3 className="transfer-section-title">前 {source.rows.length} 行按映射的样子</h3>
      <PreviewTable
        label="按映射预览"
        headers={shown.headers}
        subheaders={shown.sources}
        rows={shown.rows}
        rowNumbers={source.rowNumbers}
      />
    </div>
  );
}
