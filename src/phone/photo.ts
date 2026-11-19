// Phone cameras hand over HEIC, 48 MP or files over 12 MiB. The label reader uses 3072 px, so send a JPEG of that size.

const LONG_SIDE = 3072;
const UNREADABLE = 'This photo cannot be read. Take it again or choose a JPEG.';

export function fitWithin(width: number, height: number, max: number): { width: number; height: number } {
  const scale = Math.min(1, max / Math.max(width, height));
  return { width: Math.round(width * scale), height: Math.round(height * scale) };
}

async function decode(file: Blob): Promise<{ source: CanvasImageSource; width: number; height: number; done: () => void }> {
  try {
    const bitmap = await createImageBitmap(file, { imageOrientation: 'from-image' });
    return { source: bitmap, width: bitmap.width, height: bitmap.height, done: () => bitmap.close() };
  } catch {
    // Older Safari: an image element applies the orientation when it is drawn.
    const url = URL.createObjectURL(file);
    const image = new Image();
    image.src = url;
    try {
      await image.decode();
    } catch {
      URL.revokeObjectURL(url);
      throw new Error(UNREADABLE);
    }
    return { source: image, width: image.naturalWidth, height: image.naturalHeight, done: () => URL.revokeObjectURL(url) };
  }
}

/** Decodes the photo, applies its orientation, scales it to 3072 px on the long side and encodes a JPEG at 0.9. */
export async function preparePhoto(file: Blob): Promise<Blob> {
  const image = await decode(file);
  try {
    if (!image.width || !image.height) throw new Error(UNREADABLE);
    const size = fitWithin(image.width, image.height, LONG_SIDE);
    const canvas = document.createElement('canvas');
    canvas.width = size.width;
    canvas.height = size.height;
    const context = canvas.getContext('2d');
    if (!context) throw new Error(UNREADABLE);
    context.drawImage(image.source, 0, 0, size.width, size.height);
    const blob = await new Promise<Blob | null>(resolve => canvas.toBlob(resolve, 'image/jpeg', 0.9));
    if (!blob) throw new Error(UNREADABLE);
    return blob;
  } finally {
    image.done();
  }
}
