import { flexRender } from '@tanstack/react-table';
import {
  getCoreRowModel,
  getSortedRowModel,
  useLegacyTable as useReactTable,
  type LegacyColumnDef,
} from '@tanstack/react-table/legacy';
import { type RowData, type SortingState } from '@tanstack/table-core';
import { useState } from 'react';

export interface PaginationState {
  pageIndex: number;
  pageSize: number;
  /** Pages on the server. Rows passed in are already the current page. */
  pageCount: number;
}

interface Props<T extends RowData> {
  data: T[];
  columns: LegacyColumnDef<T, any>[];
  initialSorting?: SortingState;
  /** Client-side sorting. Turn it off when the server orders the rows. Defaults to on. */
  sortable?: boolean;
  /** Server-side pagination: `data` is one page and the table does not slice it. */
  pagination?: PaginationState;
  compact?: boolean;
  emptyText: string;
  label: string;
}

/** TanStack Table rendered as a plain HTML table, optionally with manual (server-side) pagination. */
export function DataTable<T extends RowData>({
  data,
  columns,
  initialSorting = [],
  sortable = true,
  pagination,
  compact = false,
  emptyText,
  label,
}: Props<T>) {
  const [sorting, setSorting] = useState<SortingState>(initialSorting);

  const table = useReactTable({
    data,
    columns,
    state: {
      sorting,
      ...(pagination ? { pagination: { pageIndex: pagination.pageIndex, pageSize: pagination.pageSize } } : {}),
    },
    onSortingChange: setSorting,
    enableSorting: sortable,
    getCoreRowModel: getCoreRowModel(),
    ...(sortable ? { getSortedRowModel: getSortedRowModel() as any } : {}),
    ...(pagination ? { manualPagination: true, pageCount: pagination.pageCount } : {}),
  });

  const rows = table.getRowModel().rows;

  return (
    <div className={`table-wrap stack ${compact ? 'compact' : ''}`}>
      <table aria-label={label}>
        <thead>
          {table.getHeaderGroups().map((group) => (
            <tr key={group.id}>
              {group.headers.map((header) => {
                const sortableColumn = header.column.getCanSort();
                const dir = header.column.getIsSorted();
                return (
                  <th
                    key={header.id}
                    scope="col"
                    aria-sort={
                      !sortableColumn ? undefined : dir === 'asc' ? 'ascending' : dir === 'desc' ? 'descending' : 'none'
                    }
                    className={`col-${header.column.id}`}
                  >
                    {header.isPlaceholder ? null : sortableColumn ? (
                      <button type="button" className="sort-btn" onClick={header.column.getToggleSortingHandler()}>
                        {flexRender(header.column.columnDef.header, header.getContext())}
                        <span aria-hidden="true" className="sort-ind">
                          {dir === 'asc' ? '▲' : dir === 'desc' ? '▼' : ''}
                        </span>
                      </button>
                    ) : (
                      flexRender(header.column.columnDef.header, header.getContext())
                    )}
                  </th>
                );
              })}
            </tr>
          ))}
        </thead>
        <tbody>
          {rows.map((row) => (
            <tr key={row.id}>
              {row.getVisibleCells().map((cell) => (
                <td
                  key={cell.id}
                  className={`col-${cell.column.id}`}
                  data-label={typeof cell.column.columnDef.header === 'string' ? cell.column.columnDef.header : undefined}
                >
                  {flexRender(cell.column.columnDef.cell, cell.getContext())}
                </td>
              ))}
            </tr>
          ))}
        </tbody>
      </table>
      {rows.length === 0 && <div className="state state-empty">{emptyText}</div>}
    </div>
  );
}
