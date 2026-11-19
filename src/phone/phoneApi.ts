// The phone page talks to Corydoras on the Mac over the Wi-Fi. Nothing here imports Tauri.
import type { InventoryDraft, InventoryItem } from '../inventoryApi';
import { frameDrafts } from '../inventoryFrame.ts';

export const TOKEN_KEY = 'corydoras.phone.token';

export const OFFLINE = 'Cannot reach Corydoras. Check that the Mac is awake, on the same Wi-Fi, and that Phone is on in Settings.';
export const DISCONNECTED = 'This phone is no longer connected. Open Corydoras on your Mac, go to Settings › Phone, and scan the code.';

export interface PhoneStatus {
  app: string;
  version: string;
  library: string;
  inventory_enabled: boolean;
  provider_ready: boolean;
  categories: string[];
}

export class PhoneError extends Error {
  readonly status: number | 'offline';
  constructor(message: string, status: number | 'offline') {
    super(message);
    this.status = status;
  }
}

/** The token that a scanned code carries: `#t=<token>`. */
export function tokenFromHash(hash: string): string | null {
  const match = /^#t=([^&]+)$/.exec(hash);
  return match ? match[1] : null;
}

export function messageFor(status: number | 'offline', serverText?: string): string {
  if (status === 'offline') return OFFLINE;
  if (status === 401) return DISCONNECTED;
  return serverText || `Corydoras answered with an error (${status}).`;
}

/** A read that never reached the Mac (offline, or cancelled by a reload) stays pending, so a reload asks again for
 *  free: the Mac keeps results. A read the Mac answered with an error may have been paid; it is not repeated by itself. */
export function keepsReadPending(e: unknown): boolean {
  return !(e instanceof PhoneError) || e.status === 'offline';
}

/** Names one save, so a save repeated after a lost answer adds nothing. crypto.randomUUID needs HTTPS; this does not. */
export function newBatchId(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(16));
  return Array.from(bytes, b => b.toString(16).padStart(2, '0')).join('');
}

// Kept in memory too, for a browser that refuses storage (a private tab).
let current: string | null = null;

function storedToken(): string | null {
  if (current) return current;
  try { current = localStorage.getItem(TOKEN_KEY); } catch { /* no storage: memory only */ }
  return current;
}

export function forgetToken(): void {
  current = null;
  try { localStorage.removeItem(TOKEN_KEY); } catch { /* no storage */ }
}

/** Takes a token from the address (and removes it there), or the one stored before. */
export function connect(): string | null {
  const fresh = tokenFromHash(location.hash);
  if (fresh) {
    current = fresh;
    try { localStorage.setItem(TOKEN_KEY, fresh); } catch { /* memory only */ }
    history.replaceState(null, '', location.pathname + location.search);
  }
  return storedToken();
}

async function request<T>(path: string, init: RequestInit = {}): Promise<T> {
  const token = storedToken();
  if (!token) throw new PhoneError(DISCONNECTED, 401);
  let response: Response;
  try {
    response = await fetch(path, { ...init, cache: 'no-store', headers: { ...init.headers, Authorization: `Bearer ${token}` } });
  } catch {
    throw new PhoneError(OFFLINE, 'offline');
  }
  let body: unknown = null;
  try { body = await response.json(); } catch { /* not JSON */ }
  if (!response.ok) {
    if (response.status === 401) forgetToken();
    const text = body && typeof body === 'object' && 'error' in body && typeof body.error === 'string' ? body.error : undefined;
    throw new PhoneError(messageFor(response.status, text), response.status);
  }
  return body as T;
}

const binary = { 'Content-Type': 'application/octet-stream' };

export const phoneApi = {
  status: () => request<PhoneStatus>('/api/status'),
  /** One draft per item that the photo shows. Nothing is saved. */
  read: async (photo: Blob) =>
    (await request<{ drafts: InventoryDraft[] }>('/api/read', { method: 'POST', headers: binary, body: photo })).drafts,
  /** Saves the reviewed drafts with their photo. Either all are saved or none; the same batch saves once. */
  save: async (drafts: InventoryDraft[], photo: Blob, batch: string) =>
    (await request<{ items: InventoryItem[] }>('/api/save', {
      method: 'POST', headers: { ...binary, 'X-Batch-Id': batch }, body: await frameDrafts(drafts, photo),
    })).items,
};
