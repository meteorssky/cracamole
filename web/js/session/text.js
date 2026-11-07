// Native Unicode conversion avoids the vendor StringReader/Writer's surrogate-pair defects.
const LIMIT = 256 * 1024;

export function encodeClipboard(text) {
  for (const character of text) {
    const code = character.codePointAt(0);
    if (code >= 0xd800 && code <= 0xdfff) throw new Error('剪貼簿含有不完整的 Unicode 字元，未傳送。');
  }
  const bytes = new TextEncoder().encode(text);
  if (bytes.byteLength > LIMIT) throw new Error('剪貼簿超過 256 KiB，未傳送。請縮短內容。');
  return bytes;
}

export function receiveClipboard(stream, onText, onError) {
  const reader = new Guacamole.ArrayBufferReader(stream);
  const decoder = new TextDecoder('utf-8', { fatal: true, ignoreBOM: true });
  let parts = [];
  let bytes = 0;
  let failed = false;
  const reject = message => {
    if (failed) return;
    failed = true;
    parts = [];
    stream.sendAck(message, 0x030f);
    onError(message);
  };
  reader.ondata = buffer => {
    if (failed) return;
    bytes += buffer.byteLength;
    if (bytes > LIMIT) {
      reject('遠端剪貼簿超過 256 KiB，未接收。請縮短內容。');
      return;
    }
    try {
      parts.push(decoder.decode(buffer, { stream: true }));
      stream.sendAck('OK', 0);
    } catch {
      reject('遠端剪貼簿不是有效的 UTF-8，未接收。');
    }
  };
  reader.onend = () => {
    if (failed) return;
    try {
      parts.push(decoder.decode());
      onText(parts.join(''));
      parts = [];
    } catch {
      reject('遠端剪貼簿含有不完整的 UTF-8 字元，未接收。');
    }
  };
}

export function sendClipboard(client, bytes) {
  const writer = new Guacamole.ArrayBufferWriter(client.createClipboardStream('text/plain'));
  writer.blobLength = 3072; // Base64 chunks are unpadded except the final one.
  writer.sendData(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength));
  writer.sendEnd();
}
