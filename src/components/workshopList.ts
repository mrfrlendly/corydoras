import type { InventoryItem } from '../inventoryApi';
import { columnDefs, isColumnKey, type ColumnKey } from './workshopColumns.ts';
import { colourSwatches } from './workshopColours.ts';

/** The five columns of the fixed list layout. Each column's label comes from the column definitions. */
export const listColumns = (['name', 'category', 'colour', 'quantity', 'pack_quantity'] as const)
  .map(field => ({ field, label: columnDefs[field].label }));

export type SortField = ColumnKey;
export type ListSort = { field: SortField; direction: 'asc' | 'desc' };
export const defaultListSort: ListSort = { field: 'name', direction: 'asc' };
const natural = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

export function parseListSort(value: string | null): ListSort {
  const parts = value?.split(':') ?? [];
  const direction = parts[1];
  if (parts.length !== 2 || !isColumnKey(parts[0]) || !columnDefs[parts[0]].sortable
    || (direction !== 'desc' && direction !== 'asc')) return defaultListSort;
  return { field: parts[0], direction };
}

export function nextListSort(current: ListSort, field: SortField): ListSort {
  return { field, direction: current.field === field && current.direction === 'asc' ? 'desc' : 'asc' };
}

/** The sort to display. A sort on a hidden column falls back to name ascending; the stored preference is not touched. */
export function effectiveSort(sort: ListSort, visibleIds: readonly string[]): ListSort {
  return sort.field === 'name' || visibleIds.includes(sort.field) ? sort : defaultListSort;
}

function sortableValue(item: InventoryItem, field: SortField): string | number | null {
  return columnDefs[field].sortValue(item);
}

export function sortItems(items: readonly InventoryItem[], sort: ListSort): InventoryItem[] {
  return [...items].sort((a, b) => {
    const left = sortableValue(a, sort.field);
    const right = sortableValue(b, sort.field);
    if ((left === null) !== (right === null)) return left === null ? 1 : -1;
    const group = columnDefs[sort.field].sortGroup;
    if (group && left !== null) {
      const byGroup = group(a) - group(b);
      if (byGroup) return byGroup;
    }
    let order = 0;
    if (typeof left === 'number' && typeof right === 'number') order = left - right;
    if (typeof left === 'string' && typeof right === 'string') order = natural.compare(left, right);
    if (order) return sort.direction === 'asc' ? order : -order;
    const byName = natural.compare(a.name, b.name);
    return byName || a.id.localeCompare(b.id);
  });
}

// The single words that earlier versions drew with a CSS colour name. They keep that look; every other colour text,
// including every phrase ("Dark Grey", "Titan Grey", "Olive Green"), comes from the shared colour dictionary.
const namedColours = new Set(('black white blue navy royalblue skyblue lightblue darkblue teal turquoise cyan aqua ' +
  'red darkred orange darkorange yellow gold brown saddlebrown green darkgreen lime olive purple violet ' +
  'magenta pink hotpink gray lightgray darkgray slategray silver ivory beige tan coral maroon indigo').split(' '));

/** The swatch colours of a colour text, one for each "/" part, or [] when any part is not a known colour. */
export function swatchColours(label: string): string[] {
  const names = label.trim().toLowerCase().split(/\s*\/\s*/);
  if (names.length > 3) return [];
  const colours = names.map(text => {
    if (text === 'clear' || text === 'transparent') return 'transparent';
    const name = text.replace(/grey$/, 'gray');
    if (!/\s/.test(text) && namedColours.has(name) || /^#(?:[a-f\d]{3}|[a-f\d]{6})$/.test(text)) return name;
    return colourSwatches(text)[0] ?? '';
  });
  return colours.every(Boolean) ? colours : [];
}
