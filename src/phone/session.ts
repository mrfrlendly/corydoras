// The photo and the cards in progress survive a reload or a locked phone. IndexedDB works over plain HTTP.
import type { WorkshopForm } from '../components/workshopForm';

import { newBatchId } from './phoneApi.ts';

export interface PhoneSession {
  photo: Blob;
  /** null while the read is pending. */
  cards: WorkshopForm[] | null;
  /** Names the save of this photo, so a repeated save adds nothing. */
  batch: string;
  /** The Mac's answer to a failed read, kept so a reload does not read (and pay) again. */
  failure: string | null;
}

// The photo and the cards are kept apart, so typing on a card does not rewrite the photo.
// Safari has stored Blobs badly in IndexedDB; an ArrayBuffer is safe.
interface StoredPhoto { bytes: ArrayBuffer; type: string; batch?: string }

const DB = 'corydoras-phone';
const STORE = 'session';

function open(): Promise<IDBDatabase> {
  return new Promise((resolve, reject) => {
    const request = indexedDB.open(DB, 1);
    request.onupgradeneeded = () => { request.result.createObjectStore(STORE); };
    request.onsuccess = () => resolve(request.result);
    request.onerror = () => reject(request.error);
  });
}

async function run<T>(mode: IDBTransactionMode, work: (store: IDBObjectStore) => IDBRequest<T>): Promise<T> {
  const db = await open();
  try {
    return await new Promise<T>((resolve, reject) => {
      const request = work(db.transaction(STORE, mode).objectStore(STORE));
      request.onsuccess = () => resolve(request.result);
      request.onerror = () => reject(request.error);
    });
  } finally {
    db.close();
  }
}

export async function loadSession(): Promise<PhoneSession | null> {
  try {
    const photo = await run<StoredPhoto | undefined>('readonly', store => store.get('photo'));
    if (!photo) return null;
    const cards = await run<WorkshopForm[] | null | undefined>('readonly', store => store.get('cards'));
    const failure = await run<string | null | undefined>('readonly', store => store.get('failure'));
    return {
      photo: new Blob([photo.bytes], { type: photo.type }),
      // Cards stored by an older phone page have no price source: the designer entered the price.
      cards: cards?.map(card => ({ ...card, price_source: card.price_source ?? '' })) ?? null,
      batch: photo.batch ?? newBatchId(),
      failure: failure ?? null,
    };
  } catch {
    return null;
  }
}

/** Keeps a new photo, its batch id and its cards (null while the read is pending). */
export async function saveSession(session: Omit<PhoneSession, 'failure'>): Promise<void> {
  try {
    const photo: StoredPhoto = { bytes: await session.photo.arrayBuffer(), type: session.photo.type, batch: session.batch };
    await run('readwrite', store => store.clear());
    await run('readwrite', store => store.put(photo, 'photo'));
    await run('readwrite', store => store.put(session.cards, 'cards'));
  } catch { /* the review still works; it just will not survive a reload */ }
}

/** null marks the read as pending again. */
export async function saveCards(cards: WorkshopForm[] | null): Promise<void> {
  try { await run('readwrite', store => store.put(cards, 'cards')); } catch { /* as above */ }
}

export async function saveFailure(text: string | null): Promise<void> {
  try { await run('readwrite', store => store.put(text, 'failure')); } catch { /* as above */ }
}

export async function clearSession(): Promise<void> {
  try { await run('readwrite', store => store.clear()); } catch { /* nothing kept */ }
}
