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
}

const maxPhotoBytes = 12 * 1024 * 1024;

export const inventoryApi = {
  setEnabled: (enabled: boolean) => invoke<AppInfo>('set_inventory_enabled', { enabled }),
  list: () => invoke<InventoryItem[]>('list_inventory'),
  save: (id: string | null, draft: InventoryDraft) => invoke<InventoryItem>('save_inventory', { id, draft }),
  remove: (id: string) => invoke<void>('delete_inventory', { id }),
  search: (name: string, brand: string, partNumber: string) =>
    invoke<SupplierListing[]>('search_inventory_listings', { name, brand, partNumber }),
  readLabel: async (file: File): Promise<InventoryDraft> => {
    if (!file.size || file.size > maxPhotoBytes) throw new Error('Select a nonempty photo of 12 MiB or less.');
    return invoke<InventoryDraft>('read_inventory_label', new Uint8Array(await file.arrayBuffer()));
  },
};
