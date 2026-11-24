// Plain logic for the quick actions of the inventory list: the stepper, the keyboard shortcuts, the category manager
// and the currency field. No React and no Tauri here, so that node can test it.

import type { CategoryInfo, InventoryItem } from '../inventoryApi';
import { formatUnitPrice } from './workshopColumns.ts';
import { categoryKey, parsePrice } from './workshopForm.ts';

/**
 * The count after pressing - or + on On hand. null means nothing changes: the count is already 0 and cannot go lower.
 * An unknown count counts as 0 for the buttons: + sets 1 and - sets 0.
 */
export function stepQuantity(current: number | null | undefined, delta: number): number | null {
  const base = current ?? 0;
  const next = base + delta;
  if (!Number.isSafeInteger(next)) return null;
  if (next < 0) return current == null ? 0 : null;
  return next;
}

/**
 * The item as the library saves it after - or +. A lower count (or a count of 0) assumes the opened pack was the one used
 * up, so the amount left is cleared; a higher or equal count keeps it, and so does an unknown count going down to more
 * than 0. null means nothing changes, as in stepQuantity.
 */
export function stepQuantityItem(item: InventoryItem, delta: number): InventoryItem | null {
  const before = item.quantity ?? null;
  const next = stepQuantity(before, delta);
  if (next === null) return null;
  const used = next === 0 || (before !== null && next < before);
  return { ...item, quantity: next, measure_left: used ? null : (item.measure_left ?? null) };
}

/** The minus button is off only at a count of 0. An unknown count can still be stepped. */
export function canDecrease(current: number | null | undefined): boolean {
  return current !== 0;
}

export type Shortcut = 'search' | 'add';

export interface ShortcutKey {
  key: string;
  ctrlKey?: boolean;
  metaKey?: boolean;
  altKey?: boolean;
  isComposing?: boolean;
  defaultPrevented?: boolean;
  target?: {
    tagName?: string;
    isContentEditable?: boolean;
    /** The nearest ancestor (or the element itself) that matches the selector. Absent on plain test objects. */
    closest?: (selector: string) => unknown;
  } | null;
}

/** Elements where a typed letter belongs to the element. */
const typingTags = new Set(['INPUT', 'TEXTAREA', 'SELECT']);
/** Controls that act on Enter or Space; inside the column picker they keep every key to themselves. */
const pressTags = new Set(['BUTTON', 'A', 'SUMMARY']);
/** An open dialog or popover: the shortcuts of the list stay out of it. */
export const overlaySelector = '[role="dialog"], [role="alertdialog"], [aria-modal="true"], dialog[open], .workshop-picker-panel';
/** The column picker, its button included. */
export const pickerSelector = '.workshop-picker';

/**
 * `/` focuses the search box and `n` opens Add item. Both are ignored while typing in a field, with Ctrl, Cmd or Alt held,
 * inside an open dialog or popover, on a button, link or summary of the column picker, and while the editor, the category
 * manager or the photo review is open (`blocked`).
 */
export function shortcutFor(event: ShortcutKey, blocked: boolean): Shortcut | null {
  if (blocked || event.defaultPrevented || event.isComposing) return null;
  if (event.ctrlKey || event.metaKey || event.altKey) return null;
  const target = event.target;
  if (target && (typingTags.has((target.tagName ?? '').toUpperCase()) || target.isContentEditable)) return null;
  if (target?.closest?.(overlaySelector)) return null;
  if (target && pressTags.has((target.tagName ?? '').toUpperCase()) && target.closest?.(pickerSelector)) return null;
  if (event.key === '/') return 'search';
  if (event.key === 'n') return 'add';
  return null;
}

export type CurrencyCheck = { ok: true; code: string } | { ok: false; error: string };

/** A currency code is three letters. Lower case is accepted and stored in upper case. */
export function checkCurrency(text: string): CurrencyCheck {
  const code = text.trim().toUpperCase();
  if (!/^[A-Z]{3}$/.test(code)) return { ok: false, error: 'Enter a three-letter currency code, such as CAD or USD.' };
  return { ok: true, code };
}

const same = (a: string, b: string) => categoryKey(a) === categoryKey(b);

/** The category with this name in any letter case. */
export function findCategory<T extends { name: string }>(categories: readonly T[], name: string): T | undefined {
  return categories.find(category => same(category.name, name));
}

/** The reason a new category name cannot be used, or an empty string. */
export function newCategoryError(categories: readonly CategoryInfo[] | readonly { name: string }[], name: string): string {
  const typed = name.trim();
  if (!typed) return 'Enter a category name.';
  if (typed.length > 120) return 'Use a category name of 120 characters or fewer.';
  if (findCategory(categories, typed)) return `${findCategory(categories, typed)!.name} already exists.`;
  return '';
}

/** The reason a category cannot be renamed to `to`, or an empty string. Same name in other letters is allowed. */
export function renameError(from: string, to: string): string {
  const typed = to.trim();
  if (!typed) return 'Enter a category name.';
  if (typed.length > 120) return 'Use a category name of 120 characters or fewer.';
  if (typed === from) return 'Enter a different name.';
  return '';
}

/** Whether renaming `from` to `to` merges into another category that exists, and that category. */
export function mergeTarget<T extends { name: string }>(categories: readonly T[], from: string, to: string): T | undefined {
  if (same(from, to)) return undefined;
  return findCategory(categories, to);
}

const plural = (count: number) => `${count} ${count === 1 ? 'item' : 'items'}`;

/** The confirm text for Merge into. */
export function mergeText(from: string, count: number, into: string): string {
  const moves = count === 0 ? `${from} has no items to move.` : `${plural(count)} will move from ${from} to ${into}.`;
  return `${moves} ${into} keeps its own column choice. If ${into} is on Auto, it takes the column choice of ${from}. ${from} is removed.`;
}

/** The toast after a rename or a merge. */
export function movedText(count: number, to: string): string {
  return count === 0 ? `Renamed to ${to}.` : `${plural(count)} moved to ${to}.`;
}

/**
 * The line under the price field: the price of one piece, or of one unit of a measured pack (per g). Empty when the price
 * or the pack count (or the full pack) is missing or wrong.
 */
export function pieceHint(price: string, packQuantity: string, currency: string,
  measure: { unit: string; full: string } = { unit: '', full: '' }): string {
  let cents: number | null;
  try { cents = parsePrice(price); } catch { return ''; }
  if (measure.unit) {
    const full = measure.full.trim() === '' ? NaN : Number(measure.full);
    if (cents === null || !Number.isSafeInteger(full) || full < 1) return '';
    return `About ${formatUnitPrice(cents / full, currency)} per ${measure.unit}`;
  }
  const pack = packQuantity.trim() === '' ? NaN : Number(packQuantity);
  if (cents === null || !Number.isSafeInteger(pack) || pack < 1) return '';
  return `About ${formatUnitPrice(cents / pack, currency)} per piece`;
}

export type OpenAmountCheck = { ok: true; left: number | null } | { ok: false; error: string };

/** What was typed in the open pack field of the list: a blank clears the amount, otherwise a whole number from 0 to the full pack. */
export function checkOpenAmount(text: string, full: number, unit: string): OpenAmountCheck {
  const typed = text.trim();
  if (typed === '') return { ok: true, left: null };
  const value = Number(typed);
  if (!/^\d+$/.test(typed) || !Number.isSafeInteger(value) || value > full) {
    return { ok: false, error: `Enter a whole number from 0 to ${full} ${unit}, or leave it empty.` };
  }
  return { ok: true, left: value };
}

export interface CategoryChip {
  name: string;
  count: number;
}

/**
 * The category chips: stored categories (empty ones too) and any that items use, by name in any letter case, as the library
 * treats them. Counts merge the spellings. The name shown is the stored one, else the first spelling seen. Sorted by name.
 */
export function categoryChips(items: readonly Pick<InventoryItem, 'category'>[], infos: readonly { name: string }[]): CategoryChip[] {
  const chips = new Map<string, CategoryChip>();
  for (const info of infos) {
    const key = categoryKey(info.name);
    if (!chips.has(key)) chips.set(key, { name: info.name, count: 0 });
  }
  for (const item of items) {
    const key = categoryKey(item.category);
    const chip = chips.get(key);
    if (chip) chip.count += 1;
    else chips.set(key, { name: item.category, count: 1 });
  }
  return [...chips.values()].sort((a, b) => a.name.localeCompare(b.name));
}
