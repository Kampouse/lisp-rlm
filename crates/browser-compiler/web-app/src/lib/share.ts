// Share-link codec: deflate-raw + base64url via the browser CompressionStream
// API (baseline since 2023). Cuts snippet URLs ~3-5x vs the legacy
// btoa(encodeURIComponent(src)) `code=` param, so shared snippets survive
// chat apps that truncate long URLs.

export async function compressToBase64Url(text: string): Promise<string> {
  const bytes = new TextEncoder().encode(text);
  const cs = new CompressionStream('deflate-raw');
  const stream = new Blob([toArrayBuffer(bytes)]).stream().pipeThrough(cs);
  const buf = await new Response(stream).arrayBuffer();
  return bytesToBase64Url(new Uint8Array(buf));
}

export async function decompressFromBase64Url(data: string): Promise<string> {
  const bytes = base64UrlToBytes(data);
  const ds = new DecompressionStream('deflate-raw');
  const stream = new Blob([toArrayBuffer(bytes)]).stream().pipeThrough(ds);
  return new Response(stream).text();
}

// BlobPart typing under TS 5.9 Uint8Array<ArrayBufferLike> generics — an
// explicit ArrayBuffer copy is unambiguous under any lib version.
function toArrayBuffer(bytes: Uint8Array): ArrayBuffer {
  const out = new ArrayBuffer(bytes.byteLength);
  new Uint8Array(out).set(bytes);
  return out;
}

function bytesToBase64Url(bytes: Uint8Array): string {
  let bin = '';
  for (const b of bytes) bin += String.fromCharCode(b);
  return btoa(bin).replace(/\+/g, '-').replace(/\//g, '_').replace(/=+$/, '');
}

function base64UrlToBytes(s: string): Uint8Array {
  const b64 = s.replace(/-/g, '+').replace(/_/g, '/');
  const bin = atob(b64);
  const bytes = new Uint8Array(bin.length);
  for (let i = 0; i < bin.length; i++) bytes[i] = bin.charCodeAt(i);
  return bytes;
}
