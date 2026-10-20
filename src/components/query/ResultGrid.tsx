import { useMemo, useState } from "react";
import type { ResultSet } from "../../lib/queryTypes";
import type { SortSpec } from "../../lib/types";
import { DataGrid, type GridActions } from "../data/DataGrid";
import { EMPTY_EDITS } from "../data/editState";
import { nextSort } from "../data/GridHeader";
import { fitSelection, type GridSelection } from "../data/gridModel";
import { EmptyState } from "../ui/EmptyState";
import { resultPage, sortKey } from "./resultModel";

/** 只读的结果表格不会调用这些。 */
const NO_ACTIONS: GridActions = {
  setCell: () => {},
  revertCell: () => {},
  deleteRow: () => {},
  restoreRow: () => {},
};

/** 一个结果集:复用表数据的 DataGrid,只读,点列头在前端排序。 */
export function ResultGrid({ set, label }: { set: ResultSet; label: string }) {
  const [sort, setSort] = useState<SortSpec | null>(null);
  const [selection, setSelection] = useState<GridSelection | null>(null);
  const page = useMemo(() => resultPage(set, sort), [set, sort]);
  const fitted = fitSelection(selection, page.rows.length, page.columns.length);
  if (fitted !== selection) setSelection(fitted);

  if (set.columns.length === 0) return <EmptyState title="这个结果集没有列" />;
  return (
    <DataGrid
      label={label}
      page={page}
      edits={EMPTY_EDITS}
      rowOffset={0}
      editable={false}
      selection={fitted}
      onSelectionChange={setSelection}
      sort={sort}
      sortDisabled={false}
      sortKeyOf={(column, index) => sortKey(index, column.name)}
      onSort={(key) => setSort((current) => nextSort(current, key))}
      copyOnlyMenu
      actions={NO_ACTIONS}
      focusRequest={0}
      scrollKey={sort ? `${sort.column}:${sort.descending}` : ""}
    />
  );
}
