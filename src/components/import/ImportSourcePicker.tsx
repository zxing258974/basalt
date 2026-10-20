import { FileCode2, FileUp, HardDrive } from "lucide-react";
import type { ReactNode } from "react";
import type { ImportSource } from "../../lib/importTypes";

interface LocalDetection {
  pending: boolean;
  count: number | undefined;
  error: unknown;
}

interface ImportSourcePickerProps {
  /** 选的是连接来源;选了「保存的查询」时为 null。 */
  source: ImportSource | null;
  local: LocalDetection;
  queries: LocalDetection;
  queriesChosen: boolean;
  onChooseLocal(): void;
  onPickNcx(): void;
  onChooseQueries(): void;
}

function detected({ pending, count, error }: LocalDetection, unit: string): string {
  if (pending) return "正在检测…";
  if (error || count === undefined) return "未检测到";
  return `检测到 ${count} ${unit}`;
}

const isDetected = (detection: LocalDetection) => !detection.pending && !detection.error && (detection.count ?? 0) > 0;

function fileName(path: string): string {
  return path.split("/").pop() ?? path;
}

/** 来源三选一:两种连接来源、Navicat 保存的查询。选 .ncx 时每次点击都重新打开文件选择框。 */
export function ImportSourcePicker(props: ImportSourcePickerProps) {
  const { source, local, queries, queriesChosen } = props;
  const ncxPath = !queriesChosen && source?.kind === "ncxFile" ? source.path : null;
  return (
    <div className="import-sources" role="radiogroup" aria-label="导入来源">
      <SourceOption
        icon={<HardDrive />}
        label="本机 Navicat 的连接"
        meta={detected(local, "个连接")}
        checked={!queriesChosen && source?.kind === "localNavicat"}
        disabled={!isDetected(local)}
        onClick={props.onChooseLocal}
      />
      <SourceOption
        icon={<FileUp />}
        label="Navicat 导出文件（.ncx）"
        meta={ncxPath ? fileName(ncxPath) : "选择 .ncx 文件…"}
        title={ncxPath ?? "在 Navicat 里「文件 → 导出连接」生成，勾选「导出密码」可连密码一起迁移"}
        checked={ncxPath !== null}
        onClick={props.onPickNcx}
      />
      <SourceOption
        icon={<FileCode2 />}
        label="Navicat 保存的查询"
        meta={detected(queries, "条查询")}
        title="本机 Navicat 里保存的 .sql 查询，导入到对应连接的库下"
        checked={queriesChosen}
        disabled={!isDetected(queries)}
        onClick={props.onChooseQueries}
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
