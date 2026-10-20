import { FileUp, HardDrive } from "lucide-react";
import type { ReactNode } from "react";
import type { ImportSource } from "../../lib/importTypes";

interface LocalDetection {
  pending: boolean;
  count: number | undefined;
  error: unknown;
}

interface ImportSourcePickerProps {
  source: ImportSource | null;
  local: LocalDetection;
  onChooseLocal(): void;
  onPickNcx(): void;
}

function localMeta({ pending, count, error }: LocalDetection): string {
  if (pending) return "正在检测…";
  if (error || count === undefined) return "未检测到";
  return `检测到 ${count} 个连接`;
}

function fileName(path: string): string {
  return path.split("/").pop() ?? path;
}

/** 来源二选一。选 .ncx 时每次点击都重新打开文件选择框。 */
export function ImportSourcePicker({ source, local, onChooseLocal, onPickNcx }: ImportSourcePickerProps) {
  const detected = !local.pending && !local.error && (local.count ?? 0) > 0;
  const ncxPath = source?.kind === "ncxFile" ? source.path : null;
  return (
    <div className="import-sources" role="radiogroup" aria-label="导入来源">
      <SourceOption
        icon={<HardDrive />}
        label="本机 Navicat"
        meta={localMeta(local)}
        checked={source?.kind === "localNavicat"}
        disabled={!detected}
        onClick={onChooseLocal}
      />
      <SourceOption
        icon={<FileUp />}
        label="Navicat 导出文件（.ncx）"
        meta={ncxPath ? fileName(ncxPath) : "选择 .ncx 文件…"}
        title={ncxPath ?? "在 Navicat 里「文件 → 导出连接」生成，勾选「导出密码」可连密码一起迁移"}
        checked={ncxPath !== null}
        onClick={onPickNcx}
      />
    </div>
  );
}

interface SourceOptionProps {
  icon: ReactNode;
  label: string;
  meta: string;
  title?: string;
  checked: boolean;
  disabled?: boolean;
  onClick(): void;
}

function SourceOption({ icon, label, meta, title, checked, disabled, onClick }: SourceOptionProps) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={checked}
      className={["import-source", checked && "is-checked"].filter(Boolean).join(" ")}
      disabled={disabled}
      title={title}
      onClick={onClick}
    >
      <span className="import-source-icon" aria-hidden>
        {icon}
      </span>
      <span className="import-source-text">
        <span className="import-source-label">{label}</span>
        <span className="import-source-meta">{meta}</span>
      </span>
    </button>
  );
}
