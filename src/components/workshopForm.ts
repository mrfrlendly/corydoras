import type { InventoryDraft, InventoryItem, MeasureUnit, Spec, SpecSource } from '../inventoryApi';
import { totalAmount } from './workshopColumns.ts';

export type WorkshopForm = Omit<InventoryDraft, 'quantity' | 'pack_quantity' | 'price_cents' | 'measure_full' | 'measure_left'> & {
  quantity: string;
  pack_quantity: string;
  /** Amount in one full pack, in measure_unit, as typed. */
  measure_full: string;
  /** Amount left in the opened pack, as typed. */
  measure_left: string;
  /** Dollars as typed, for example 9.99. */
  price: string;
};

/** Where a spec came from, in the words the Mac and the phone show. */
export const sourceLabel: Record<SpecSource, string> = {
  label: 'Printed',
  notation: 'Printed, role read from notation',
  standard: 'Standard, not measured',
  entered: 'Entered',
};

/** The units of a measured pack for the Unit select. '' counts pieces. Keep in step with measureUnits in inventoryApi. */
export const measureUnitOptions: ReadonlyArray<{ value: MeasureUnit; label: string }> = [
  { value: '', label: 'Pieces' }, { value: 'g', label: 'g' }, { value: 'mL', label: 'mL' }, { value: 'm', label: 'm' },
];

export const LABEL_CATEGORIES = [
  'Fasteners', 'Hardware', 'Tools', 'Electronics', 'Materials', 'Consumables',
  'Adhesives', 'Finishes', 'Safety equipment', 'Other',
];

export function blankForm(): WorkshopForm {
  return {
    name: '', category: '', brand: '', part_number: '', colour: '', dimensions: '',
    quantity: '', pack_quantity: '', price: '', measure_unit: '', measure_full: '', measure_left: '',
    notes: '', specs: [], supplier: null,
  };
}

export function editForm(item: InventoryDraft): WorkshopForm {
  return {
    name: item.name, category: item.category, brand: item.brand, part_number: item.part_number,
    colour: item.colour ?? '', dimensions: item.dimensions, quantity: item.quantity?.toString() ?? '',
    pack_quantity: item.pack_quantity?.toString() ?? '', price: priceText(item.price_cents),
    // Older cards and drafts have no measure fields.
    measure_unit: item.measure_unit ?? '', measure_full: item.measure_full?.toString() ?? '',
    measure_left: item.measure_left?.toString() ?? '',
    notes: item.notes, specs: item.specs ?? [], supplier: item.supplier,
  };
}

/** A copy of an item for Duplicate item: the same fields with the count and the opened pack cleared. The photo is not copied. */
export function duplicateForm(item: InventoryDraft): WorkshopForm {
  const form = editForm(item);
  return { ...form, quantity: '', measure_left: '', specs: form.specs.map(spec => ({ ...spec })), supplier: form.supplier && { ...form.supplier } };
}

const roleWords: Record<string, string> = {
  thread: 'thread', pitch: 'pitch', length: 'length', outer_diameter: 'outer diameter', hole_diameter: 'hole diameter',
  clearance_hole: 'clearance hole', tap_drill: 'tap drill', diameter: 'diameter', net_weight: 'net weight',
  material: 'material', nozzle_temp: 'nozzle temperature', bed_temp: 'bed temperature', drying: 'drying',
};

/** The Dimensions line. The database builds the same text when it saves; this one is for display. */
export function specSummary(specs: Spec[]): string {
  return specs.map(spec => {
    if (spec.role === 'thread') return `${spec.value} thread`;
    const words = spec.role === 'other' ? spec.name : roleWords[spec.role] ?? spec.role;
    return `${words} ${spec.value}${spec.tolerance ? ` ±${spec.tolerance}` : ''}${spec.unit ? ` ${spec.unit}` : ''}${spec.source === 'standard' ? ' (standard)' : ''}`;
  }).join('; ');
}

/** A designer's edit to a value makes it theirs. The printed evidence stays and cannot be edited. */
export function editSpec(specs: Spec[], index: number, patch: Partial<Omit<Spec, 'source' | 'evidence'>>): Spec[] {
  const current = specs[index];
  const changed = (['role', 'name', 'value', 'unit', 'tolerance'] as const).some(key => patch[key] !== undefined && patch[key] !== current[key]);
  if (!changed) return specs;
  const next: Spec = { ...current, source: 'entered' };
  for (const key of ['role', 'name', 'value', 'unit', 'tolerance'] as const) if (patch[key] !== undefined) next[key] = patch[key];
  if (next.role !== 'other') next.name = '';
  return specs.map((spec, i) => i === index ? next : spec);
}

export function addSpec(specs: Spec[]): Spec[] {
  return [...specs, { role: 'other', name: '', value: '', unit: '', tolerance: '', source: 'entered', evidence: '' }];
}

/**
 * Set the specs of a form. The first spec of an item that has typed Dimensions text moves that text into the notes,
 * because the saved Dimensions text is built from the specs and would otherwise replace it.
 */
export function withSpecs(form: WorkshopForm, specs: Spec[]): WorkshopForm {
  const typed = form.dimensions.trim();
  const keepTyped = form.specs.length === 0 && specs.length > 0 && typed !== '';
  const earlier = `Earlier dimensions: ${typed}`;
  const notes = keepTyped && !form.notes.includes(earlier) ? (form.notes.trim() ? `${form.notes}\n${earlier}` : earlier) : form.notes;
  return { ...form, specs, notes, dimensions: specs.length ? specSummary(specs) : form.dimensions };
}

export function removeSpec(specs: Spec[], index: number): Spec[] {
  return specs.filter((_, i) => i !== index);
}

/** Removes one spec on the phone. When the last spec goes, its generated Dimensions text goes too, so a removed
 *  wrong value is not saved as text; text the designer typed stays. */
export function removeSpecRow(form: WorkshopForm, index: number): WorkshopForm {
  const generated = form.specs.length > 0 && form.dimensions === specSummary(form.specs);
  const next = withSpecs(form, removeSpec(form.specs, index));
  return next.specs.length === 0 && generated ? { ...next, dimensions: '' } : next;
}

/** A measured pack the designer already set up stays as it is; otherwise the label's one is taken whole. */
function reviewMeasure(form: WorkshopForm, label: InventoryDraft): Pick<WorkshopForm, 'measure_unit' | 'measure_full' | 'measure_left'> {
  if (form.measure_unit) return { measure_unit: form.measure_unit, measure_full: form.measure_full ?? '', measure_left: form.measure_left ?? '' };
  const unit = label.measure_unit ?? '';
  return { measure_unit: unit, measure_full: unit ? label.measure_full?.toString() ?? '' : '', measure_left: unit ? label.measure_left?.toString() ?? '' : '' };
}

export function reviewLabel(form: WorkshopForm, label: InventoryDraft): WorkshopForm {
  const keep = (current: string, suggested: string) => current.trim() ? current : suggested;
  const specs = form.specs.length ? form.specs : label.specs ?? [];
  return {
    ...form,
    specs,
    name: keep(form.name, label.name),
    category: keep(form.category, label.category),
    brand: keep(form.brand, label.brand),
    part_number: keep(form.part_number, label.part_number),
    colour: keep(form.colour, label.colour ?? ''),
    dimensions: specs.length ? specSummary(specs) : keep(form.dimensions, label.dimensions),
    pack_quantity: keep(form.pack_quantity, label.pack_quantity?.toString() ?? ''),
    price: keep(form.price ?? '', priceText(label.price_cents)),
    ...reviewMeasure(form, label),
    notes: keep(form.notes, label.notes),
  };
}

function quantity(text: string): number | null {
  const value = text.trim();
  if (value === '') return null;
  if (!/^\d+$/.test(value) || !Number.isSafeInteger(Number(value))) {
    throw new Error('Enter a whole number from 0 to 9,007,199,254,740,991.');
  }
  return Number(value);
}

const maxAmount = Number.MAX_SAFE_INTEGER;

/** A whole amount of a measured pack. `what` names it in the message, for example "the full pack". */
function amount(text: string | undefined, what: string, min: number): number | null {
  const value = (text ?? '').trim();
  if (value === '') return null;
  const number = Number(value);
  if (!/^\d+$/.test(value) || !Number.isSafeInteger(number) || number < min) {
    throw new Error(`Enter ${what} as a whole number from ${min} to 9,007,199,254,740,991.`);
  }
  return number;
}

/** The unit with its amounts: the full pack and what is left in the opened pack. */
function measure(form: WorkshopForm): Pick<InventoryDraft, 'measure_unit' | 'measure_full' | 'measure_left'> {
  const unit = form.measure_unit ?? '';
  if (!measureUnitOptions.some(option => option.value === unit)) throw new Error('Choose Pieces, g, mL or m as the unit.');
  const full = amount(form.measure_full, 'the full pack', 1);
  const left = amount(form.measure_left, 'the amount left in the opened pack', 0);
  if (!unit && (full !== null || left !== null)) {
    throw new Error('Choose a unit for the measured pack, or clear the full pack and the amount left.');
  }
  if (left !== null && full === null) throw new Error('Enter the full pack amount before the amount left in the opened pack.');
  if (left !== null && full !== null && left > full) {
    throw new Error(`The amount left in the opened pack cannot be more than the full pack of ${full} ${unit}.`);
  }
  return { measure_unit: unit as MeasureUnit, measure_full: full, measure_left: left };
}

/** The amount on hand of a form, in its unit. null when the unit, the full pack or the count is missing or does not parse. */
export function formTotalAmount(form: WorkshopForm): number | null {
  try {
    return totalAmount({ quantity: quantity(form.quantity), ...measure(form) });
  } catch { return null; }
}

/** The weight or volume that a label prints as `net_weight`, as a whole amount in `unit`; kg gives grams and L gives mL. */
export function fullFromSpecs(specs: readonly Spec[] | undefined, unit: string): number | null {
  const factors: Record<string, Record<string, number>> = { g: { g: 1, kg: 1000 }, mL: { ml: 1, l: 1000 }, m: { m: 1 } };
  const table = factors[unit];
  if (!table) return null;
  for (const spec of specs ?? []) {
    if (spec.role !== 'net_weight') continue;
    // The value and unit are separate fields; a value that still carries its unit ("1 kg") is read too.
    const joined = spec.unit?.trim() ? `${spec.value} ${spec.unit}` : spec.value;
    const found = /^\s*(\d{1,3}(?:,\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?)\s*([a-zA-Z]+)\s*$/.exec(joined);
    if (!found) continue;
    const factor = table[found[2].toLowerCase()];
    if (factor === undefined) continue;
    const scaled = Number(found[1].replaceAll(',', '')) * factor;
    const whole = Math.round(scaled);
    if (Math.abs(scaled - whole) < 1e-6 && whole >= 1 && whole <= maxAmount) return whole;
  }
  return null;
}

/**
 * Choose the unit of the measured pack. Back to Pieces clears the amounts, which then have no unit.
 * A first unit prefills the full pack from the printed net weight when the full pack is empty.
 */
export function chooseUnit(form: WorkshopForm, unit: MeasureUnit): WorkshopForm {
  if (!unit) return { ...form, measure_unit: '', measure_full: '', measure_left: '' };
  const full = form.measure_full?.trim() ? form.measure_full : String(fullFromSpecs(form.specs, unit) ?? '');
  return { ...form, measure_unit: unit, measure_full: full };
}

/** One bought pack adds the pack's count to the stock on hand. */
export function addPack(form: WorkshopForm): WorkshopForm {
  const pack = quantity(form.pack_quantity);
  if (pack === null) throw new Error('Enter the quantity per pack first.');
  const total = (quantity(form.quantity) ?? 0) + pack;
  if (!Number.isSafeInteger(total)) throw new Error('Enter a whole number from 0 to 9,007,199,254,740,991.');
  return { ...form, quantity: String(total) };
}

const maxPriceCents = 1_000_000_000_000;
const priceShape = /^(?:\d+|\d{1,3}(?:,\d{3})+)?(?:\.\d*)?$/;

/** Dollars as typed to whole cents, without float error. Empty means no price. */
export function parsePrice(text: string): number | null {
  if (text.trim() === '') return null;
  const typed = text.trim().replace(/^\$\s*/, '');
  if (typed.startsWith('-')) throw new Error('Enter a price of 0 or more.');
  if (!priceShape.test(typed) || !/\d/.test(typed) || typed.endsWith('.')) {
    throw new Error('Enter a price in dollars, such as 9.99.');
  }
  const [whole, fraction = ''] = typed.replaceAll(',', '').split('.');
  if (fraction.length > 2) throw new Error('Enter a price with at most two decimals, such as 9.99.');
  const cents = Number(whole || '0') * 100 + Number(fraction.padEnd(2, '0'));
  if (!Number.isSafeInteger(cents) || cents > maxPriceCents) throw new Error('Enter a price of $10,000,000,000 or less.');
  return cents;
}

/** Whole cents as dollars for the price field, for example 999 gives 9.99. */
export function priceText(cents: number | null | undefined): string {
  if (cents == null) return '';
  return `${Math.floor(cents / 100)}.${String(cents % 100).padStart(2, '0')}`;
}

export function saveDraft(form: WorkshopForm): InventoryDraft {
  if (!form.name.trim()) throw new Error('Enter an item name.');
  const { price, ...fields } = form;
  return {
    ...fields, ...measure(form), name: form.name.trim(), category: form.category.trim() || 'Other', colour: form.colour.trim(),
    quantity: quantity(form.quantity), pack_quantity: quantity(form.pack_quantity),
    price_cents: parsePrice(price ?? ''),
  };
}

/** Categories are one category in any letter case, as in the library. */
export function categoryKey(name: string): string {
  return name.trim().toLowerCase();
}

export function matchingItems(items: InventoryItem[], query: string, category: string,
  flags?: { noPrice?: boolean; noCount?: boolean }): InventoryItem[] {
  const words = query.toLowerCase().trim().split(/\s+/).filter(Boolean);
  return items.filter((item) => {
    if (category && categoryKey(item.category) !== categoryKey(category)) return false;
    if (flags?.noPrice && item.price_cents != null) return false;
    if (flags?.noCount && item.quantity != null) return false;
    const fields = [item.name, item.category, item.brand, item.part_number, item.colour, item.dimensions, item.notes];
    const text = fields.join(' ').toLowerCase();
    return words.every(word => text.includes(word));
  });
}

const localSuffixes = new Set(['localhost', 'local', 'internal', 'lan', 'invalid', 'test', 'home', 'example', 'arpa', 'onion']);

export function publicListingUrl(value: string): URL | null {
  try {
    if (value.length > 4096 || /[\s\\\u0000-\u001f\u007f]/u.test(value)) return null;
    const url = new URL(value);
    const host = url.hostname.replace(/\.$/, '');
    const labels = host.split('.');
    const suffix = labels[labels.length - 1];
    const dnsLabel = /^[a-z0-9](?:[a-z0-9-]{0,61}[a-z0-9])?$/i;
    if (url.protocol !== 'https:' || url.username || url.password || url.href.length > 4096) return null;
    if (labels.length < 2 || host.length > 253 || !labels.every(label => dnsLabel.test(label))) return null;
    if (/^\d+$/.test(suffix) || localSuffixes.has(suffix)) return null;
    return url;
  } catch { return null; }
}

/** A quick signature check before showing or submitting a local photo. The native decoder verifies the entire image. */
export async function validateItemPhoto(file: File): Promise<void> {
  if (!file.size || file.size > 12 * 1024 * 1024) throw new Error('Choose a nonempty photo of 12 MiB or less.');
  const header = new Uint8Array(await file.slice(0, 12).arrayBuffer());
  const isJpeg = header[0] === 0xff && header[1] === 0xd8 && header[2] === 0xff;
  const isPng = [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a].every((b, i) => header[i] === b);
  const isWebP = String.fromCharCode(...header.slice(0, 4)) === 'RIFF'
    && String.fromCharCode(...header.slice(8, 12)) === 'WEBP';
  if (!isJpeg && !isPng && !isWebP) throw new Error('Choose a JPEG, PNG, or WebP photo.');
}

/** Drafts for one batch save. A draft that cannot be saved is named by its place in the list. */
export function batchDrafts(forms: WorkshopForm[]): InventoryDraft[] {
  return forms.map((form, i) => {
    try { return saveDraft(form); }
    catch (e) { throw new Error(`Item ${i + 1}: ${e instanceof Error ? e.message : String(e)}`); }
  });
}
