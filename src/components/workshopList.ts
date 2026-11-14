import type { InventoryItem } from '../inventoryApi';

export const listColumns = [
  { field: 'name', label: 'Item' },
  { field: 'category', label: 'Category' },
  { field: 'colour', label: 'Colour' },
  { field: 'quantity', label: 'On hand' },
  { field: 'pack_quantity', label: 'Per pack' },
] as const;

export type SortField = (typeof listColumns)[number]['field'];
export type ListSort = { field: SortField; direction: 'asc' | 'desc' };
export const defaultListSort: ListSort = { field: 'name', direction: 'asc' };
const natural = new Intl.Collator(undefined, { numeric: true, sensitivity: 'base' });

export function parseListSort(value: string | null): ListSort {
  const parts = value?.split(':') ?? [];
  const column = listColumns.find(({ field }) => field === parts[0]);
  const direction = parts[1];
  if (parts.length !== 2 || !column || (direction !== 'desc' && direction !== 'asc')) return defaultListSort;
  return { field: column.field, direction };
}

export function nextListSort(current: ListSort, field: SortField): ListSort {
  return { field, direction: current.field === field && current.direction === 'asc' ? 'desc' : 'asc' };
}

function sortableValue(item: InventoryItem, field: SortField): string | number | null {
  const value = item[field];
  return typeof value === 'string' ? value.trim() || null : value ?? null;
}

export function sortItems(items: readonly InventoryItem[], sort: ListSort): InventoryItem[] {
  return [...items].sort((a, b) => {
    const left = sortableValue(a, sort.field);
    const right = sortableValue(b, sort.field);
    if ((left === null) !== (right === null)) return left === null ? 1 : -1;
    let order = 0;
    if (typeof left === 'number' && typeof right === 'number') order = left - right;
    if (typeof left === 'string' && typeof right === 'string') order = natural.compare(left, right);
    if (order) return sort.direction === 'asc' ? order : -order;
    const byName = natural.compare(a.name, b.name);
    return byName || a.id.localeCompare(b.id);
  });
}

// Only assign a swatch when the stored text identifies a known colour.
const namedColours = new Set(('black white blue navy royalblue skyblue lightblue darkblue teal turquoise cyan aqua ' +
  'red darkred orange darkorange yellow gold brown saddlebrown green darkgreen lime olive purple violet ' +
  'magenta pink hotpink gray lightgray darkgray slategray silver ivory beige tan coral maroon indigo').split(' '));

export function swatchColours(label: string): string[] {
  const names = label.trim().toLowerCase().split(/\s*\/\s*/);
  if (names.length > 3) return [];
  const colours = names.map(text => {
    if (text === 'clear' || text === 'transparent') return 'transparent';
    const name = text.replaceAll(' ', '').replace(/grey$/, 'gray');
    if (namedColours.has(name) || /^#(?:[a-f\d]{3}|[a-f\d]{6})$/.test(text)) return name;
    return '';
  });
  return colours.every(Boolean) ? colours : [];
}
