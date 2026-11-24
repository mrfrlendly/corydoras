import type { ColumnConfig, ColumnId, InventoryItem } from '../inventoryApi';

// Only types come from inventoryApi: it imports Tauri, which the node tests cannot load.

export type ColumnKey = 'name' | ColumnId;
export type SortValue = string | number | null;
export type ColumnScope = 'all' | 'category';

export interface ColumnDef {
  id: ColumnKey;
  label: string;
  /** Right-aligned number columns. */
  numeric: boolean;
  sortable: boolean;
  /** The value that the list sorts by. null means missing, which sorts last. */
  sortValue: (item: InventoryItem) => SortValue;
  /**
   * Items with a higher group sort after items with a lower one, in both directions, before the values are compared.
   * Missing values still come last of all. Without it every item is in group 0.
   */
  sortGroup?: (item: InventoryItem) => number;
}

const text = (value: string | null | undefined): string | null => value?.trim() || null;

/** The fields that make up a measured pack. An item or a draft has them all. */
export type Measured = Pick<InventoryItem, 'quantity' | 'measure_unit' | 'measure_full' | 'measure_left'>;

/** The full pack of a measured item, or null when the item counts pieces or has no full amount. */
function measuredFull(item: Partial<Measured>): number | null {
  const full = item.measure_full ?? null;
  return item.measure_unit && full !== null && full >= 1 ? full : null;
}

/**
 * The amount on hand in the measure unit: whole packs, the opened one included, with the opened pack counted at what is
 * left in it. null when the item is not measured or the count is unknown.
 */
export function totalAmount(item: Partial<Measured>): number | null {
  const full = measuredFull(item);
  const quantity = item.quantity ?? null;
  if (full === null || quantity === null) return null;
  const left = item.measure_left ?? null;
  if (quantity < 1) return 0;
  const total = left === null ? quantity * full : (quantity - 1) * full + left;
  // Beyond 2^53 - 1 the sum is no longer exact. The record keeps no such total, so neither does the list.
  return Number.isSafeInteger(total) ? total : null;
}

/** The share of the opened pack that is left, 0 to 1. null when the item has no full pack or no recorded amount left. */
export function fractionLeft(item: Partial<Measured>): number | null {
  const full = measuredFull(item);
  const left = item.measure_left ?? null;
  return full === null || left === null ? null : left / full;
}

/** An amount with its unit, for example 1,100 g. */
export function formatAmount(amount: number, unit: string): string {
  return `${amount.toLocaleString()} ${unit}`.trim();
}

/** The open pack as the list shows it, for example 100 of 1000 g. null when no amount left is recorded. */
export function openPackText(item: Partial<Measured>): string | null {
  const full = measuredFull(item);
  const left = item.measure_left ?? null;
  return full === null || left === null ? null : `${left} of ${full} ${item.measure_unit}`;
}

/** True when the item is priced per unit of its measure (g, mL or m) rather than per piece. */
export function isMeasured(item: Partial<Measured>): boolean {
  return measuredFull(item) !== null;
}

/**
 * Price of one piece: the pack price divided by the pack count. A fraction of a cent is kept.
 * A measured item is priced per unit of its measure instead: the pack price divided by the full pack, for example per gram.
 */
export function unitPriceCents(item: InventoryItem): number | null {
  const price = item.price_cents ?? null;
  if (price === null) return null;
  const full = measuredFull(item);
  if (full !== null) return price / full;
  const pack = item.pack_quantity ?? null;
  return pack !== null && pack >= 1 ? price / pack : price;
}

/**
 * The stock on hand times the unit price, in whole cents. null when the price or the count is unknown.
 * A measured item uses its total amount (the opened pack at what is left), not its count of packs.
 */
export function stockValueCents(item: InventoryItem): number | null {
  const price = item.price_cents ?? null;
  const quantity = item.quantity ?? null;
  if (price === null || quantity === null) return null;
  const full = measuredFull(item);
  if (full !== null) {
    const total = totalAmount(item);
    return total === null ? null : Math.round((total * price) / full);
  }
  const pack = item.pack_quantity ?? null;
  return Math.round(pack !== null && pack >= 1 ? (quantity * price) / pack : quantity * price);
}

/** True when the item has a price that is an estimate from a supplier listing. The designer's own edit clears the mark. */
export function isEstimatedPrice(item: Pick<InventoryItem, 'price_cents' | 'price_source'>): boolean {
  return item.price_cents != null && !!item.price_source?.trim();
}

const def = (id: ColumnKey, label: string, numeric: boolean, sortValue: ColumnDef['sortValue'],
  sortGroup?: ColumnDef['sortGroup']): ColumnDef => ({ id, label, numeric, sortable: true, sortValue, sortGroup });

export const columnDefs: Record<ColumnKey, ColumnDef> = {
  name: def('name', 'Item', false, item => text(item.name)),
  category: def('category', 'Category', false, item => text(item.category)),
  brand: def('brand', 'Brand', false, item => text(item.brand)),
  part_number: def('part_number', 'Part number', false, item => text(item.part_number)),
  colour: def('colour', 'Colour', false, item => text(item.colour)),
  dimensions: def('dimensions', 'Dimensions', false, item => text(item.dimensions)),
  quantity: def('quantity', 'On hand', true, item => item.quantity ?? null),
  pack_quantity: def('pack_quantity', 'Per pack', true, item => item.pack_quantity ?? null),
  open_amount: def('open_amount', 'Open pack', true, fractionLeft),
  price: def('price', 'Price', true, item => item.price_cents ?? null),
  // A price per gram is not comparable with a price per piece, so measured items sort after the others.
  unit_price: def('unit_price', 'Unit price', true, unitPriceCents, item => (isMeasured(item) ? 1 : 0)),
  stock_value: def('stock_value', 'Stock value', true, stockValueCents),
  notes: def('notes', 'Notes', false, item => text(item.notes)),
  supplier: def('supplier', 'Supplier', false, item => text(item.supplier?.title)),
};

export function isColumnKey(value: string): value is ColumnKey {
  return Object.prototype.hasOwnProperty.call(columnDefs, value);
}

export function isColumnId(value: string): value is ColumnId {
  return value !== 'name' && isColumnKey(value);
}

function moneyFormat(currency: string, fraction: [number, number]): Intl.NumberFormat | null {
  try {
    return new Intl.NumberFormat(undefined, {
      style: 'currency', currency, currencyDisplay: 'narrowSymbol', minimumFractionDigits: fraction[0], maximumFractionDigits: fraction[1],
    });
  } catch { return null; }
}

/** Whole cents as money in the library currency, for example $9.99. The narrow symbol keeps the library's own CAD as $ rather than CA$. */
export function formatMoney(cents: number, currency: string): string {
  const format = moneyFormat(currency, [2, 2]);
  return format ? format.format(cents / 100) : `${(cents / 100).toFixed(2)} ${currency}`;
}

/** A unit price: two decimals from one dollar up, and up to four below it, such as $0.222. */
export function formatUnitPrice(cents: number, currency: string): string {
  if (cents >= 100) return formatMoney(cents, currency);
  const format = moneyFormat(currency, [2, 4]);
  return format ? format.format(cents / 100) : `${(cents / 100).toFixed(4)} ${currency}`;
}

const hasText = (value: string | null | undefined) => !!value?.trim();

/**
 * The columns of an Auto view: only those with a value for at least one item. Item is implicit.
 * Brand, part number, notes and supplier are never automatic because the caption shows brand and part number.
 */
export function autoColumns(items: readonly InventoryItem[], scope: ColumnScope): ColumnId[] {
  const columns: ColumnId[] = [];
  if (scope === 'all') columns.push('category');
  if (items.some(item => hasText(item.colour))) columns.push('colour');
  if (items.some(item => hasText(item.dimensions))) columns.push('dimensions');
  if (items.some(item => item.pack_quantity != null)) columns.push('pack_quantity');
  columns.push('quantity');
  if (items.some(item => (item.measure_left ?? null) !== null)) columns.push('open_amount');
  if (items.some(item => item.price_cents != null)) columns.push('price');
  if (items.some(item => stockValueCents(item) !== null)) columns.push('stock_value');
  return columns;
}

/** The columns to show: the stored choice in its order, or Auto when the config is null. Unknown ids are dropped. */
export function visibleColumns(items: readonly InventoryItem[], config: ColumnConfig | readonly string[],
  scope: ColumnScope): ColumnId[] {
  if (config === null || config === undefined) return autoColumns(items, scope);
  const seen = new Set<string>();
  const columns: ColumnId[] = [];
  for (const id of config) {
    if (!isColumnId(id) || seen.has(id)) continue;
    seen.add(id);
    columns.push(id);
  }
  return columns;
}

export interface ViewTotals {
  count: number;
  /** Items whose count on hand is unknown. */
  withoutCount: number;
  /** Items without a price. */
  unpriced: number;
  /** Stock value of the items that have both a price and a count. Items missing either add nothing. */
  stockValueCents: number;
  /** Items whose price is an estimate from a supplier listing. */
  estimatedPrices: number;
}

export function viewTotals(items: readonly InventoryItem[]): ViewTotals {
  let withoutCount = 0;
  let unpriced = 0;
  let value = 0;
  let estimatedPrices = 0;
  for (const item of items) {
    if (isEstimatedPrice(item)) estimatedPrices += 1;
    if (item.quantity == null) withoutCount += 1;
    if (item.price_cents == null) unpriced += 1;
    value += stockValueCents(item) ?? 0;
  }
  return { count: items.length, withoutCount, unpriced, stockValueCents: value, estimatedPrices };
}

/** The item caption still shows the brand and part number that are not their own columns. */
export function captionFields(columns: readonly string[]): Array<'brand' | 'part_number'> {
  return (['brand', 'part_number'] as const).filter(field => !columns.includes(field));
}
