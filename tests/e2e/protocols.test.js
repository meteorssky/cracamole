// The front end learns protocols from the server catalogue (run: node --test protocols.test.js).
const test = require('node:test');
const assert = require('node:assert');
const path = require('node:path');
const { pathToFileURL } = require('node:url');

const load = () => import(pathToFileURL(path.join(__dirname, '../../web/js/util/protocols.js')).href);

const CATALOGUE = [
  { name: 'ssh', label: 'SSH', default_port: 22, identity: 'host-key', args: ['hostname', 'port', 'host-key'] },
  { name: 'telnet', label: 'Telnet', default_port: 23, identity: null, args: ['hostname', 'port'] },
  { name: 'rdp', label: 'RDP', default_port: 3389, identity: 'certificate', args: ['hostname', 'domain'] },
];

test('catalogue drives labels, ports, accepted parameters and identity kind', async () => {
  const requests = [];
  globalThis.fetch = async (url, options) => {
    requests.push([url, options.method]);
    return { ok: true, status: 200, json: async () => CATALOGUE };
  };
  const p = await load();
  assert.deepStrictEqual(p.allProtocols(), []);
  await p.loadProtocols();
  assert.deepStrictEqual(requests, [['/api/protocols', 'GET']]);
  assert.deepStrictEqual(p.allProtocols().map(x => x.name), ['ssh', 'telnet', 'rdp']);
  assert.strictEqual(p.protocolLabel('rdp'), 'RDP');
  assert.strictEqual(p.defaultPort('ssh'), '22');
  assert.ok(p.protocolArgs('rdp').has('domain'));
  assert.ok(!p.protocolArgs('telnet').has('domain'));
  assert.strictEqual(p.identityKind('ssh'), 'host-key');
  assert.strictEqual(p.identityKind('rdp'), 'certificate');
  assert.strictEqual(p.identityKind('telnet'), null);
  assert.strictEqual(p.endpoint({ protocol: 'telnet', params: { hostname: 'h' } }), 'h:23');
  assert.strictEqual(p.endpoint({ protocol: 'ssh', params: { hostname: 'h', port: '2222', username: 'u' } }), 'u@h:2222');
});

test('an unknown protocol degrades to an upper-case label and no port, args or identity', async () => {
  const p = await load();
  assert.strictEqual(p.protocolLabel('future'), 'FUTURE');
  assert.strictEqual(p.defaultPort('future'), '');
  assert.strictEqual(p.protocolArgs('future').size, 0);
  assert.strictEqual(p.identityKind('future'), null);
  assert.strictEqual(p.endpoint({ protocol: 'future', params: { hostname: 'h' } }), 'h');
});
