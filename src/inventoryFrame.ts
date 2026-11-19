import type { InventoryDraft } from './inventoryApi';

/** One request carries the reviewed drafts and their shared photo: a 4-byte big-endian length, the JSON, then the photo. */
export async function frameDrafts(drafts: InventoryDraft[], photo: Blob): Promise<Uint8Array<ArrayBuffer>> {
  const meta = new TextEncoder().encode(JSON.stringify({ drafts }));
  const frame = new Uint8Array(4 + meta.length + photo.size);
  new DataView(frame.buffer).setUint32(0, meta.length, false);
  frame.set(meta, 4);
  frame.set(new Uint8Array(await photo.arrayBuffer()), 4 + meta.length);
  return frame;
}
