/** 文件前几行的预览:行号、列头(可以带一行小字说明来源),NULL 显示成淡色的 NULL。 */
export function PreviewTable({
  label,
  headers,
  subheaders,
  rows,
  rowNumbers,
}: {
  label: string;
  headers: string[];
  subheaders?: string[];
  rows: (string | null)[][];
  rowNumbers: number[];
}) {
  if (headers.length === 0) return <p className="transfer-placeholder">没有可以预览的列。</p>;
  return (
    <div className="transfer-preview-scroll">
      <table className="transfer-preview" aria-label={label}>
        <thead>
          <tr>
            <th className="transfer-preview-gutter" scope="col">
              行
            </th>
            {headers.map((header, index) => (
              <th key={index} scope="col">
                <span className="transfer-preview-head">{header}</span>
                {subheaders?.[index] && <span className="transfer-preview-sub">← {subheaders[index]}</span>}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row, r) => (
            <tr key={r}>
              <td className="transfer-preview-gutter mono">{rowNumbers[r]}</td>
              {headers.map((_, c) => {
                const value = row[c] ?? null;
                return (
                  <td key={c} className={value === null ? "is-null" : undefined} title={value ?? undefined}>
                    {value ?? "NULL"}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
