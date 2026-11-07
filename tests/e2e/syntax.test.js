// Every front-end module must parse as an ES module (run: node --test syntax.test.js).
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');
const { spawnSync } = require('node:child_process');

const root = path.join(__dirname, '../../web/js');
function modules(dir) {
  return fs.readdirSync(dir, { withFileTypes: true }).flatMap(entry =>
    entry.isDirectory() ? modules(path.join(dir, entry.name)) : entry.name.endsWith('.js') ? [path.join(dir, entry.name)] : []);
}

for (const file of modules(root)) {
  test(`parses: ${path.relative(root, file)}`, () => {
    const result = spawnSync(process.execPath, ['--input-type=module', '--check'], { input: fs.readFileSync(file) });
    assert.strictEqual(result.status, 0, result.stderr.toString());
  });
}
