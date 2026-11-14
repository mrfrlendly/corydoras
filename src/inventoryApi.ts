import { invoke } from '@tauri-apps/api/core';
import type { AppInfo } from './api';

export interface SupplierListing {
  title: string;
  url: string;
  details: string;
}

export interface InventoryDraft {
  name: string;
  category: string;
  colour: string;
  brand: string;
  part_number: string;
  dimensions: string;
  quantity: number | null;
  pack_quantity: number | null;
  notes: string;
  supplier: SupplierListing | null;
}

export interface InventoryItem extends InventoryDraft {
  id: string;
  photo_key: string | null;
}

const maxPhotoBytes = 12 * 1024 * 1024;

// Only thumbnails share a cache. Full-size previews last for one editor view.
const thumbnails = new Map<string, Promise<string>>();
const maxThumbnails = 180;

function photo(key: string, thumbnail: boolean): Promise<string> {
  if (!thumbnail) return invoke<string>('get_inventory_photo', { key, thumbnail });
  const cached = thumbnails.get(key);
  if (cached) return cached;
  const request = invoke<string>('get_inventory_photo', { key, thumbnail }).catch(error => {
    if (thumbnails.get(key) === request) thumbnails.delete(key);
    throw error;
  });
  if (thumbnails.size >= maxThumbnails) thumbnails.delete(thumbnails.keys().next().value!);
  thumbnails.set(key, request);
  return request;
}

async function save(id: string | null, draft: InventoryDraft, change?: File | 'remove'): Promise<InventoryItem> {
  if (change === undefined) return invoke<InventoryItem>('save_inventory', { id, draft });
  if (change === 'remove') return invoke<InventoryItem>('save_inventory', { id, draft, removePhoto: true });
  if (!change.size || change.size > maxPhotoBytes) throw new Error('Select a nonempty photo of 12 MiB or less.');
  // Frame the metadata and file in one binary request so the database can save both atomically.
  const meta = new TextEncoder().encode(JSON.stringify({ id, draft }));
  const frame = new Uint8Array(4 + meta.length + change.size);
  new DataView(frame.buffer).setUint32(0, meta.length, false);
  frame.set(meta, 4);
  frame.set(new Uint8Array(await change.arrayBuffer()), 4 + meta.length);
  return invoke<InventoryItem>('save_inventory_photo', frame);
}

export const inventoryApi = {
  setEnabled: (enabled: boolean) => invoke<AppInfo>('set_inventory_enabled', { enabled }),
  list: () => {
    // A new inventory view can belong to another library. Drop its predecessor's thumbnails.
    thumbnails.clear();
    return invoke<InventoryItem[]>('list_inventory');
  },
  save,
  photo,
  remove: (id: string) => invoke<void>('delete_inventory', { id }),
  search: (name: string, brand: string, partNumber: string) =>
    invoke<SupplierListing[]>('search_inventory_listings', { name, brand, partNumber }),
  readLabel: async (file: File): Promise<InventoryDraft> => {
    if (!file.size || file.size > maxPhotoBytes) throw new Error('Select a nonempty photo of 12 MiB or less.');
    return invoke<InventoryDraft>('read_inventory_label', new Uint8Array(await file.arrayBuffer()));
  },
};
