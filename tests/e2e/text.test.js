const test = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const { pathToFileURL } = require('node:url');
const load = () => import(pathToFileURL(path.join(__dirname, '../../web/js/session/text.js')).href);

global.Guacamole = {
  ArrayBufferReader: class { constructor(stream) { stream.reader = this; } },
  ArrayBufferWriter: class {
    constructor(stream) { this.stream = stream; }
    sendData(bytes) { this.stream.bytes = new Uint8Array(bytes); }
    sendEnd() { this.stream.ended = true; }
  },
};

test('clipboard UTF-8 preserves CJK, emoji and decomposed combining characters at every byte split', async () => {
  const { encodeClipboard, receiveClipboard, sendClipboard } = await load();
  const text = '中文 日本語 한국어 😀𠮷 e\u0301';
  const bytes = encodeClipboard(text);
  for (let split = 0; split <= bytes.length; split++) {
    const stream = { sendAck: (_, code) => assert.equal(code, 0) };
    let result = null;
    receiveClipboard(stream, value => { result = value; }, message => assert.fail(message));
    stream.reader.ondata(bytes.slice(0, split).buffer);
    stream.reader.ondata(bytes.slice(split).buffer);
    stream.reader.onend();
    assert.equal(result, text, 'split ' + split);
  }
  const stream = {};
  sendClipboard({ createClipboardStream: mime => { assert.equal(mime, 'text/plain'); return stream; } }, bytes);
  assert.deepEqual(stream.bytes, bytes);
  assert.equal(stream.ended, true);
});

test('invalid Unicode and oversized clipboard text are rejected without returning a prefix', async () => {
  const { encodeClipboard, receiveClipboard } = await load();
  assert.throws(() => encodeClipboard('\ud800'));
  assert.throws(() => encodeClipboard('😀'.repeat(65537)));
  for (const bytes of [new Uint8Array([0xed, 0xa0, 0xbd, 0xed, 0xb8, 0x80]),
    new Uint8Array([0xf0, 0x9f]), new Uint8Array(262145)]) {
    let errors = 0;
    const stream = { sendAck: () => {} };
    receiveClipboard(stream, () => assert.fail('invalid clipboard was accepted'), () => { errors++; });
    stream.reader.ondata(bytes.buffer);
    stream.reader.onend();
    assert.equal(errors, 1);
  }
});
