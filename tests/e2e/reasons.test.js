// How a connection ended -> what the person is told (run: node --test reasons.test.js).
const test = require('node:test');
const assert = require('node:assert');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

const load = () => import(pathToFileURL(path.join(__dirname, '../../web/js/session/reasons.js')).href);
const ended = (failure, extra = {}) => ({ wasConnected: false, userInitiated: false, failure, ...extra });

test('a user disconnect is silent', async () => {
  const { classify } = await load();
  assert.strictEqual(classify(ended(null, { userInitiated: true })).reason, 'user');
});

test('authentication failures are recognised (status 769 and 771)', async () => {
  const { classify } = await load();
  for (const code of [769, 771]) {
    const result = classify(ended({ code, message: 'Aborted. See logs.' }));
    assert.strictEqual(result.reason, 'auth');
    assert.strictEqual(result.canEdit, true);
    assert.strictEqual(result.tone, 'error');
  }
});

test('timeouts, missing hosts and unreachable upstreams each get their own explanation', async () => {
  const { classify } = await load();
  assert.strictEqual(classify(ended({ code: 514, message: '' })).reason, 'timeout');
  assert.strictEqual(classify(ended({ code: 519, message: '' })).reason, 'not-found');
  assert.strictEqual(classify(ended({ code: 515, message: '' })).reason, 'upstream');
  assert.strictEqual(classify(ended({ code: 520, message: '' })).reason, 'upstream');
  assert.strictEqual(classify(ended({ code: 797, message: '' })).reason, 'busy');
});

test('the generic server text is not shown as a "technical detail"', async () => {
  const { classify } = await load();
  assert.ok(!classify(ended({ code: 519, message: 'Aborted. See logs.' })).detail.includes('技術細節'));
  assert.ok(classify(ended({ code: 519, message: 'something specific' })).detail.includes('something specific'));
});

test('ending after a good connection is a normal remote exit; ending before one is a network problem', async () => {
  const { classify } = await load();
  const exit = classify(ended(null, { wasConnected: true }));
  assert.strictEqual(exit.reason, 'remote-exit');
  assert.strictEqual(exit.status, '連線已結束');
  assert.strictEqual(exit.tone, 'neutral');
  assert.strictEqual(classify(ended(null)).reason, 'network');
});
