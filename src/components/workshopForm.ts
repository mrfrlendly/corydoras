import type { InventoryDraft, InventoryItem } from '../inventoryApi';

export type WorkshopForm = Omit<InventoryDraft, 'quantity' | 'pack_quantity'> & {
  quantity: string;
  pack_quantity: string;
};

export const LABEL_CATEGORIES = [
  'Fasteners', 'Hardware', 'Tools', 'Electronics', 'Materials', 'Consumables',
  'Adhesives', 'Finishes', 'Safety equipment', 'Other',
];

export function blankForm(): WorkshopForm {
  return {
    name: '', category: '', brand: '', part_number: '', colour: '', dimensions: '',
    quantity: '', pack_quantity: '', notes: '', supplier: null,
  };
}

export function editForm(item: InventoryDraft): WorkshopForm {
  return {
    name: item.name, category: item.category, brand: item.brand, part_number: item.part_number,
    colour: item.colour ?? '', dimensions: item.dimensions, quantity: item.quantity?.toString() ?? '',
    pack_quantity: item.pack_quantity?.toString() ?? '', notes: item.notes, supplier: item.supplier,
  };
}

export function reviewLabel(form: WorkshopForm, label: InventoryDraft): WorkshopForm {
  const keep = (current: string, suggested: string) => current.trim() ? current : suggested;
  return {
    ...form,
    name: keep(form.name, label.name),
    category: keep(form.category, label.category),
    brand: keep(form.brand, label.brand),
    part_number: keep(form.part_number, label.part_number),
    colour: keep(form.colour, label.colour ?? ''),
    dimensions: keep(form.dimensions, label.dimensions),
    pack_quantity: keep(form.pack_quantity, label.pack_quantity?.toString() ?? ''),
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

export function saveDraft(form: WorkshopForm): InventoryDraft {
  if (!form.name.trim()) throw new Error('Enter an item name.');
  return {
    ...form, name: form.name.trim(), category: form.category.trim() || 'Other', colour: form.colour.trim(),
    quantity: quantity(form.quantity), pack_quantity: quantity(form.pack_quantity),
  };
}

export function matchingItems(items: InventoryItem[], query: string, category: string): InventoryItem[] {
  const words = query.toLowerCase().trim().split(/\s+/).filter(Boolean);
  return items.filter((item) => {
    if (category && item.category !== category) return false;
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
