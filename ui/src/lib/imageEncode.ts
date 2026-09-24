// Images a user attaches to a chat message, read as base64 for the send.

export function readFileAsBase64(file: File | Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => {
      const result = reader.result as string;
      resolve(result.split(',')[1] || result);
    };
    reader.onerror = reject;
    reader.readAsDataURL(file);
  });
}

const MAX_EDGE = 1568;
const PASSTHROUGH_BYTES = 256 * 1024;

/**
 * Read an image for sending, downscaled.
 *
 * Vision models resample anything larger than ~1568px on the long edge
 * anyway, so the extra pixels buy no accuracy — they only cost upload
 * size and tokens. A phone photo drops from megabytes to a couple of
 * hundred KB here.
 *
 * PNG screenshots re-encode to JPEG; an image that is already small
 * enough is passed through untouched, so a deliberately-attached small
 * PNG keeps its exact bytes and its alpha.
 */
export async function readImageForSend(file: File): Promise<string> {
  let bitmap: ImageBitmap;
  try {
    bitmap = await createImageBitmap(file);
  } catch {
    // Not decodable here (exotic format, or createImageBitmap missing) —
    // send the original rather than dropping the user's attachment.
    return readFileAsBase64(file);
  }

  const longEdge = Math.max(bitmap.width, bitmap.height);
  if (longEdge <= MAX_EDGE && file.size <= PASSTHROUGH_BYTES) {
    bitmap.close();
    return readFileAsBase64(file);
  }

  const scale = Math.min(1, MAX_EDGE / longEdge);
  const canvas = document.createElement('canvas');
  canvas.width = Math.max(1, Math.round(bitmap.width * scale));
  canvas.height = Math.max(1, Math.round(bitmap.height * scale));

  const ctx = canvas.getContext('2d');
  if (!ctx) {
    bitmap.close();
    return readFileAsBase64(file);
  }
  // JPEG has no alpha; paint white so transparent PNGs don't come out black.
  ctx.fillStyle = '#ffffff';
  ctx.fillRect(0, 0, canvas.width, canvas.height);
  ctx.drawImage(bitmap, 0, 0, canvas.width, canvas.height);
  bitmap.close();

  const blob = await new Promise<Blob | null>((res) => canvas.toBlob(res, 'image/jpeg', 0.85));
  // Keep whichever is actually smaller — re-encoding a small graphic can grow it.
  if (!blob || blob.size >= file.size) return readFileAsBase64(file);
  return readFileAsBase64(blob);
}
