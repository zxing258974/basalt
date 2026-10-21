import type { ImportMode, UniqueKey } from "../../lib/transferTypes";
import { IMPORT_MODES, describeKey } from "./importModel";

interface ImportModeStepProps {
  mode: ImportMode;
  uniqueKeys: UniqueKey[];
  onChange(mode: ImportMode): void;
}

/** 第二步:文件里的行怎么写进表。没有主键和唯一索引的表做不了「追加或更新」。 */
export function ImportModeStep({ mode, uniqueKeys, onChange }: ImportModeStepProps) {
  return (
    <div className="transfer-body">
      <section className="transfer-section" aria-label="导入模式">
        <h3 className="transfer-section-title">导入模式</h3>
        <div className="transfer-range" role="radiogroup" aria-label="导入模式">
          {IMPORT_MODES.map((choice) => {
            const unavailable = choice.value === "upsert" && uniqueKeys.length === 0;
            const checked = mode === choice.value;
            return (
              <button
                key={choice.value}
                type="button"
                role="radio"
                aria-checked={checked}
                disabled={unavailable}
                className={checked ? "transfer-range-option is-checked" : "transfer-range-option"}
                onClick={() => onChange(choice.value)}
              >
                <span className="transfer-radio" aria-hidden />
                <span className="transfer-range-label">{choice.label}</span>
                <span className="transfer-range-text">
                  {unavailable ? "这张表没有主键，也没有唯一索引，判断不了哪些行已经有了。" : choice.detail}
                </span>
              </button>
            );
          })}
        </div>
        {uniqueKeys.length > 0 && (
          <p className="transfer-note">这张表上能用来匹配的键：{uniqueKeys.map(describeKey).join("；")}。</p>
        )}
      </section>
    </div>
  );
}
