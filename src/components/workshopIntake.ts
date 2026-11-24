// Plain logic of the listing intake in the inventory screens: when a suggestion may fill a field, which listing price
// may fill an empty price, and what the Review suggestions screen applies. No React and no Tauri here, so that node
// can test it.

import type { FieldInput, FieldSuggestions, ItemSuggestion, SuggestionChange, SupplierListing } from '../inventoryApi';
import { priceText } from './workshopForm.ts';

/** The categories that a keyword rule may replace, besides an empty one and one that the library does not have. */
const genericCategories = new Set(['other', 'materials', 'hardware', 'consumables']);
const key = (text: string) => text.trim().toLowerCase();

/** True when the category is empty, one of the broad ones (Other, Materials, Hardware, Consumables) or not in the library's list. */
export function isGenericCategory(name: string, categories: readonly string[]): boolean {
  const k = key(name);
  if (!k || genericCategories.has(k)) return true;
  return !categories.some(category => key(category) === k);
}

export type AutoField = 'category' | 'colour';

/**
 * Whether a suggestion may set this field now. Colour: only when it is empty. Category: only when it is generic.
 * Never after the designer typed in the field or undid an automatic fill (`touched`).
 */
export function shouldAutoFill(field: AutoField, current: string, touched: boolean, categories: readonly string[]): boolean {
  if (touched) return false;
  return field === 'colour' ? current.trim() === '' : isGenericCategory(current, categories);
}

/** The words of a form that intake reads. */
export interface IntakeText {
  name: string;
  brand: string;
  notes: string;
  dimensions: string;
  category: string;
  colour: string;
}

/** What the last automatic fill put in each field, while the designer has not changed it. */
export interface AutoFilled {
  /** `previous` is what the field held before the first automatic fill: Undo and a later "no suggestion" restore it. */
  category: { value: string; previous: string } | null;
  colour: string | null;
}
export const noAuto: AutoFilled = { category: null, colour: null };

/** True while the field still holds the value that an automatic fill put there. */
export function ownedFields(form: Pick<IntakeText, 'category' | 'colour'>, auto: AutoFilled, touched: Record<AutoField, boolean>): Record<AutoField, boolean> {
  return {
    category: !touched.category && auto.category !== null && key(form.category) === key(auto.category.value),
    colour: !touched.colour && auto.colour !== null && key(form.colour) === key(auto.colour),
  };
}

/**
 * The category and colour that a request is built on. An automatic fill that the designer has not touched counts as if
 * it were not there (its previous category, an empty colour), so that the next words can give a different answer.
 */
export function intakeBase<T extends IntakeText>(form: T, auto: AutoFilled, touched: Record<AutoField, boolean>): T {
  const own = ownedFields(form, auto, touched);
  return { ...form, category: own.category ? auto.category!.previous : form.category, colour: own.colour ? '' : form.colour };
}

const maxTitleChars = 512;

/**
 * What suggest_fields is given: the name, the brand and the notes and dimensions as the description. A chosen listing
 * sends only its title (as the name) and the item's brand; the supplier's description text is not sent.
 */
export function fieldInput(form: IntakeText, listing?: Pick<SupplierListing, 'title'> | null): FieldInput {
  if (listing) {
    return {
      name: listing.title.trim().slice(0, maxTitleChars), brand: form.brand.trim(), description: '',
      category: form.category.trim(), colour: form.colour.trim(),
    };
  }
  const description = [form.notes, form.dimensions].map(part => part.trim()).filter(Boolean).join('\n');
  return { name: form.name.trim(), brand: form.brand.trim(), description, category: form.category.trim(), colour: form.colour.trim() };
}

export interface Note { previous: string; text: string }

export interface FillResult {
  /** The fields to set. A field that must stay as it is is absent. */
  patch: { category?: string; colour?: string };
  /** The line under Category: a note when a fill was made, null to remove it, undefined to leave it as it is. */
  categoryNote?: Note | null;
  colourNote?: Note | null;
  /** What the fields hold from an automatic fill after this result is applied. */
  auto: AutoFilled;
}

/**
 * Fill-only-empty: sets an empty colour and a generic category, and leaves every other field alone. A value that an
 * earlier automatic fill put there and the designer has not touched may be replaced by a later suggestion, and goes
 * back to what it was when there is none. A field the designer typed in or undid (`touched`) is never changed.
 */
export function fillFromSuggestions(
  form: Pick<IntakeText, 'category' | 'colour'>, suggestions: FieldSuggestions,
  touched: Record<AutoField, boolean>, categories: readonly string[], auto: AutoFilled = noAuto,
): FillResult {
  const own = ownedFields(form, auto, touched);
  const patch: FillResult['patch'] = {};
  const result: FillResult = { patch, auto: { ...auto } };

  const colour = suggestions.colour?.trim() ?? '';
  if (own.colour) {
    if (!colour) {
      patch.colour = '';
      result.auto.colour = null;
      result.colourNote = null;
    } else if (key(colour) !== key(form.colour)) {
      patch.colour = colour;
      result.auto.colour = colour;
      result.colourNote = { previous: '', text: `Colour set to ${colour}: from the name` };
    }
  } else if (colour && shouldAutoFill('colour', form.colour, touched.colour, categories)) {
    patch.colour = colour;
    result.auto.colour = colour;
    result.colourNote = { previous: '', text: `Colour set to ${colour}: from the name` };
  }

  const category = suggestions.category;
  const name = category?.name.trim() ?? '';
  if (own.category) {
    const previous = auto.category!.previous;
    if (!category || !name) {
      patch.category = previous;
      result.auto.category = null;
      result.categoryNote = null;
    } else if (key(name) !== key(form.category)) {
      patch.category = name;
      result.auto.category = { value: name, previous };
      result.categoryNote = categoryNote(previous, name, category.reason);
    }
  } else if (category && name && key(name) !== key(form.category) && shouldAutoFill('category', form.category, touched.category, categories)) {
    patch.category = name;
    result.auto.category = { value: name, previous: form.category };
    result.categoryNote = categoryNote(form.category, name, category.reason);
  }
  return result;
}

function categoryNote(previous: string, name: string, reason: string): Note {
  const why = reason.trim();
  return { previous, text: why ? `Category set to ${name}: ${why}` : `Category set to ${name}` };
}

/**
 * The category to save. The library assigns a category to a new item whose category is empty, so an empty one that the
 * designer decided on (typed in the field, or clicked Undo on an automatic fill) goes as "Other" and stays empty of
 * suggestions. A field the designer never touched stays empty so that the library's intake can assign.
 */
export function categoryForSave(form: Pick<IntakeText, 'category'>, lockedByDesigner: boolean): string {
  const category = form.category.trim();
  return category === '' && lockedByDesigner ? 'Other' : category;
}

/** The host of a URL for "listing:<host>" and "Estimated from <host>". '' when the text is not a URL. */
export function hostOf(url: string): string {
  try { return new URL(url.trim()).hostname.toLowerCase(); } catch { return ''; }
}

/** One product of a supplier, whatever its variant: the address without query and fragment. */
export function productKey(url: string): string {
  try {
    const parsed = new URL(url.trim());
    return `${parsed.origin}${parsed.pathname}`;
  } catch { return url.trim(); }
}

const sameCurrency = (listingCurrency: string | undefined, libraryCurrency: string) => {
  const a = (listingCurrency ?? '').trim().toUpperCase();
  return a !== '' && a === libraryCurrency.trim().toUpperCase();
};

/** True when the listing has a price but not in the library's currency, or in no known currency. */
export function hasOtherCurrency(listing: Pick<SupplierListing, 'price_cents' | 'currency'>, libraryCurrency: string): boolean {
  return listing.price_cents != null && !sameCurrency(listing.currency, libraryCurrency);
}

export interface ListingPrice { price: string; price_source: string }

/** The price and its source for one listing, or null when its price cannot be an estimate in the library's currency. */
export function listingPrice(listing: SupplierListing, libraryCurrency: string): ListingPrice | null {
  const cents = listing.price_cents;
  if (cents == null || !Number.isSafeInteger(cents) || cents <= 0 || !sameCurrency(listing.currency, libraryCurrency)) return null;
  const host = hostOf(listing.url);
  return host ? { price: priceText(cents), price_source: `listing:${host}` } : null;
}

/**
 * The estimate that a finished search may fill without asking: among the listings of the first product (its variants
 * share one address), those in the library's currency must all have the same price. Anything else, including a first
 * product in another currency, gives null; the cards still show their prices.
 */
export function unambiguousListingPrice(listings: readonly SupplierListing[], libraryCurrency: string): ListingPrice | null {
  const first = listings[0];
  if (!first) return null;
  const product = productKey(first.url);
  const prices = listings.filter(listing => productKey(listing.url) === product)
    .map(listing => listingPrice(listing, libraryCurrency)).filter((price): price is ListingPrice => price !== null);
  if (prices.length === 0) return null;
  const distinct = new Set(prices.map(price => price.price));
  return distinct.size === 1 ? prices[0] : null;
}

/** The price fields to set when the form's price is empty, otherwise null. An existing price is never replaced. */
export function priceFill(form: { price: string }, price: ListingPrice | null): ListingPrice | null {
  return price && form.price.trim() === '' ? price : null;
}

// Review suggestions

export type SuggestionField = 'category' | 'colour';
export type SuggestionFilter = 'all' | SuggestionField;

/** The key of one tick box in the set of ticked boxes. */
export const tickKey = (field: SuggestionField, id: string) => `${field}:${id}`;

const hasField = (row: ItemSuggestion, field: SuggestionField) => field === 'category' ? row.category_to !== null : row.colour_to !== null;

/** Rows that the chip shows: All, only those with a category change, or only those with a colour. */
export function filterRows(rows: readonly ItemSuggestion[], filter: SuggestionFilter): ItemSuggestion[] {
  return filter === 'all' ? [...rows] : rows.filter(row => hasField(row, filter));
}

/** What is ticked when the screen opens: every category change, no colour. A colour comes from words in free text, so it is the designer's call. */
export function defaultTicks(rows: readonly ItemSuggestion[]): Set<string> {
  return new Set(rows.filter(row => hasField(row, 'category')).map(row => tickKey('category', row.id)));
}

/**
 * The changes to apply: exactly the ticked fields of the rows in view. A row with nothing ticked is left out, and a
 * field that is not ticked is absent from its change rather than null.
 */
export function suggestionChanges(rows: readonly ItemSuggestion[], ticked: ReadonlySet<string>, filter: SuggestionFilter = 'all'): SuggestionChange[] {
  const changes: SuggestionChange[] = [];
  for (const row of filterRows(rows, filter)) {
    const change: SuggestionChange = { id: row.id };
    if (filter !== 'colour' && row.category_to !== null && ticked.has(tickKey('category', row.id))) change.category = row.category_to;
    if (filter !== 'category' && row.colour_to !== null && ticked.has(tickKey('colour', row.id))) change.colour = row.colour_to;
    if (change.category !== undefined || change.colour !== undefined) changes.push(change);
  }
  return changes;
}

/** The number of fields that Apply changes. */
export function changeCount(changes: readonly SuggestionChange[]): number {
  return changes.reduce((sum, change) => sum + (change.category !== undefined ? 1 : 0) + (change.colour !== undefined ? 1 : 0), 0);
}

export type ColumnTick = 'all' | 'none' | 'some';

/** The state of a column's Select all/none box over the rows in view that have the field. */
export function columnTick(rows: readonly ItemSuggestion[], ticked: ReadonlySet<string>, field: SuggestionField): ColumnTick {
  const mine = rows.filter(row => hasField(row, field));
  const on = mine.filter(row => ticked.has(tickKey(field, row.id))).length;
  return on === 0 ? 'none' : on === mine.length ? 'all' : 'some';
}

/** The ticked boxes after Select all (`on`) or Select none for one column of the rows in view. Other boxes stay as they are. */
export function setColumnTicks(rows: readonly ItemSuggestion[], ticked: ReadonlySet<string>, field: SuggestionField, on: boolean): Set<string> {
  const next = new Set(ticked);
  for (const row of rows) {
    if (!hasField(row, field)) continue;
    if (on) next.add(tickKey(field, row.id));
    else next.delete(tickKey(field, row.id));
  }
  return next;
}

/** The ticks that still apply after a reload: boxes of rows that are gone are dropped, and new rows start as default. */
export function ticksAfterReload(previousRows: readonly ItemSuggestion[], rows: readonly ItemSuggestion[], ticked: ReadonlySet<string>): Set<string> {
  const known = new Set(previousRows.map(row => row.id));
  const next = new Set<string>();
  for (const row of rows) {
    for (const field of ['category', 'colour'] as const) {
      if (!hasField(row, field)) continue;
      const k = tickKey(field, row.id);
      if (known.has(row.id) ? ticked.has(k) : field === 'category') next.add(k);
    }
  }
  return next;
}

// Category words

/** Words typed in the Words editor: split at commas and line breaks, trimmed, one space between words, empty ones dropped, and each word once whatever its letter case. */
export function keywordsFromText(text: string): string[] {
  const seen = new Set<string>();
  const words: string[] = [];
  for (const part of text.split(/[,\n]/)) {
    const word = part.trim().replace(/\s+/g, ' ');
    if (!word || seen.has(word.toLowerCase())) continue;
    seen.add(word.toLowerCase());
    words.push(word);
  }
  return words;
}

/** The words as the field shows them. */
export function keywordsText(keywords: readonly string[]): string {
  return keywords.join(', ');
}

/** The muted summary on a category row: the first two words, and how many more. '' when there are none. */
export function keywordsSummary(keywords: readonly string[]): string {
  if (keywords.length === 0) return '';
  const rest = keywords.length - 2;
  return `${keywords.slice(0, 2).join(', ')}${rest > 0 ? ` +${rest} more` : ''}`;
}
