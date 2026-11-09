// Verifies every foreground/background token pair in web/css/tokens.css (run: node --test contrast.test.js).
const test = require('node:test');
const assert = require('node:assert');
const fs = require('node:fs');
const path = require('node:path');

const css = fs.readFileSync(path.join(__dirname, '../../web/css/tokens.css'), 'utf8');

function blocks() {
  // dark = first :root block; light = the two light blocks merged over dark
  const rule = /(:root[^{]*)\{([^}]*)\}/g;
  const out = [];
  let m;
  while ((m = rule.exec(css))) out.push({ selector: m[1].trim(), body: m[2] });
  return out;
}
function vars(body) {
  const v = {};
  for (const m of body.matchAll(/--([\w-]+):\s*(#[0-9a-fA-F]{6})\b/g)) v[m[1]] = m[2];
  return v;
}
const all = blocks();
const dark = vars(all[0].body);
const lightMedia = vars(css.slice(css.indexOf('@media (prefers-color-scheme: light)'), css.indexOf(':root[data-theme="light"]')));
const lightAttr = vars(all.find(b => b.selector === ':root[data-theme="light"]').body);
const themes = { dark, 'light (media)': { ...dark, ...lightMedia }, 'light (attribute)': { ...dark, ...lightAttr } };

const lum = hex => {
  const c = [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16) / 255)
    .map(v => (v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4));
  return 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
};
const ratio = (a, b) => { const [x, y] = [lum(a), lum(b)].sort((p, q) => q - p); return (x + 0.05) / (y + 0.05); };

// [foreground, background, minimum ratio, why]
const PAIRS = [
  ['text', 'surface', 4.5], ['text', 'surface-raised', 4.5], ['text', 'surface-sunken', 4.5],
  ['text-muted', 'surface', 4.5], ['text-muted', 'surface-raised', 4.5], ['text-muted', 'surface-sunken', 4.5],
  ['text-danger', 'surface', 4.5], ['text-danger', 'surface-raised', 4.5],
  ['text-danger', 'surface-sunken', 4.5],
  ['text-warning', 'surface', 4.5], ['text-warning', 'surface-raised', 4.5],
  ['text-warning', 'surface-sunken', 4.5],
  ['text-success', 'surface', 4.5], ['text-success', 'surface-raised', 4.5],
  ['on-accent', 'accent', 4.5], ['on-accent', 'accent-hover', 4.5],
  ['on-danger', 'danger', 4.5], ['on-danger', 'danger-hover', 4.5],
  ['border-strong', 'surface', 3], ['border-strong', 'surface-raised', 3],
  ['focus', 'surface', 3], ['focus', 'surface-raised', 3], ['focus', 'surface-sunken', 3],
  ['accent', 'surface', 3],
  ['on-terminal', 'terminal-bg', 4.5], ['on-terminal-muted', 'terminal-bg', 4.5],
];

for (const [name, t] of Object.entries(themes)) {
  for (const [fg, bg, min] of PAIRS) {
    test(`${name}: ${fg} on ${bg} >= ${min}:1`, () => {
      assert.ok(t[fg] && t[bg], `token missing: ${fg} or ${bg}`);
      const r = ratio(t[fg], t[bg]);
      assert.ok(r >= min, `${fg} ${t[fg]} on ${bg} ${t[bg]} = ${r.toFixed(2)}:1 (< ${min})`);
    });
  }
}
