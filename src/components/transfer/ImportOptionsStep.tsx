import type { ImportOptions, OnError } from "../../lib/transferTypes";
import type { DriverKind } from "../../lib/types";
import { Checkbox } from "../ui/Field";
import { Choice } from "./ExportOptionsForm";
import { BINARY_CHOICES } from "./exportModel";
import { MAX_BATCH_SIZE, commitNote } from "./importModel";
import { NumberField } from "./NumberField";

const ON_ERROR: { value: OnError; label: string; detail: string }[] = [
  {
    value: "stop",
    label: "停止",
    detail: "先把整份文件按目标列检查一遍，有解析不了的值就什么都不写；写入时被数据库拒绝的行会停下。",
  },
  {
    value: "skip",
    label: "跳过并记录",
    detail: "出错的行跳过、记进报告，其余照常导入。报告可以导出成 CSV。",
  },
];

interface ImportOptionsStepProps {
  options: ImportOptions;
  driver: DriverKind;
  tableTitle: string;
  onChange(options: ImportOptions): void;
}

export function ImportOptionsStep({ options, driver, tableTitle, onChange }: ImportOptionsStepProps) {
  return (
    <div className="transfer-body">
      <section className="transfer-section" aria-label="遇到错误时">
        <h3 className="transfer-section-title">遇到错误时</h3>
        <div className="transfer-range" role="radiogroup" aria-label="遇到错误时">
          {ON_ERROR.map((choice) => (
            <button
              key={choice.value}
              type="button"
              role="radio"
              aria-checked={options.onError === choice.value}
              className={
                options.onError === choice.value ? "transfer-range-option is-checked" : "transfer-range-option"
              }
              onClick={() => onChange({ ...options, onError: choice.value })}
            >
              <span className="transfer-radio" aria-hidden />
              <span className="transfer-range-label">{choice.label}</span>
              <span className="transfer-range-text">{choice.detail}</span>
            </button>
          ))}
        </div>
      </section>
      <section className="transfer-section" aria-label="写入">
        <h3 className="transfer-section-title">写入</h3>
        <div className="transfer-options">
          <NumberField
            label="每批"
            ariaLabel="每批的行数"
            unit="行"
            value={options.batchSize}
            min={1}
            max={MAX_BATCH_SIZE}
            onChange={(batchSize) => onChange({ ...options, batchSize })}
          />
          <Choice
            label="二进制列的内容是"
            value={options.binary}
            options={BINARY_CHOICES}
            onChange={(binary) => onChange({ ...options, binary })}
          />
          <div className="transfer-field is-check is-danger">
            <Checkbox
              label={`导入前清空「${tableTitle}」`}
              checked={options.clearTable}
              onCheckedChange={(clearTable) => onChange({ ...options, clearTable })}
            />
          </div>
        </div>
        <p className="transfer-note">一律参数化执行。{commitNote(driver)}</p>
      </section>
    </div>
  );
}
