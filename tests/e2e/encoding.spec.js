const { test, expect } = require('@playwright/test');
const { execFileSync } = require('node:child_process');
const path = require('node:path');
const { compose, ready, login, remoteFile, visibleInk, shellQuote, selectProtocol } = require('./helpers');

// Fixed wire vectors, independent of the product's encoding library.
const cases = [
  ['UTF-8', '中文', [0xe4, 0xb8, 0xad, 0xe6, 0x96, 0x87]],
  ['Big5', '中文', [0xa4, 0xa4, 0xa4, 0xe5]],
  ['GBK', '中文', [0xd6, 0xd0, 0xce, 0xc4]],
  ['GB18030', '中文', [0xd6, 0xd0, 0xce, 0xc4]],
  ['Shift_JIS', '日本', [0x93, 0xfa, 0x96, 0x7b]],
  ['EUC-JP', '日本', [0xc6, 0xfc, 0xcb, 0xdc]],
  ['EUC-KR', '가나', [0xb0, 0xa1, 0xb3, 0xaa]],
];
const octal = bytes => bytes.map(byte => '\\' + byte.toString(8).padStart(3, '0')).join('');
function backend(script) {
  const container = process.env.CRACAMOLE_E2E_BACKEND_CONTAINER;
  try {
    if (!container) return compose(['exec', '-T', 'backend', 'sh', '-c', script]).trim();
    return execFileSync('ssh', ['-o', 'BatchMode=yes', '-o', 'ConnectTimeout=5',
      process.env.CRACAMOLE_E2E_SSH,
      `docker exec ${shellQuote(container)} sh -c ${shellQuote(script)}`],
    { encoding: 'utf8', timeout: 30000 }).trim();
  } catch (error) {
    // Keep the infrastructure failure visible in the assertion while allowing polling to retry.
    return `fixture command failed: ${error.code || error.status}`;
  }
}
test.beforeAll(async ({ request }) => ready(request));
test.beforeEach(async ({ page }) => {
  // Exercise the current static modules even when another Windows process locks the binary.
  // The API and application lifecycle still use the real Rust server.
  if (process.env.CRACAMOLE_TEXT_SKIP_BUILD !== '1') return;
  for (const module of ['util/encodings.js', 'views/options.js', 'views/option-sections.js']) {
    await page.route(`**/assets/js/${module}`, route => route.fulfill({
      path: path.join(__dirname, '../../web/js', module), contentType: 'text/javascript',
    }));
  }
});

test('encoding: editing a saved API alias keeps its encoding', async ({ page }) => {
  await login(page);
  const headers = { Origin: new URL(page.url()).origin };
  for (const label of ['CP950', 'utf8', 'shift_jis']) {
    const name = `Alias ${label}`;
    const response = await page.request.post('api/connections', {
      headers, data: { name, protocol: 'ssh',
        params: { hostname: 'example.invalid', 'character-encoding': label } },
    });
    expect(response.ok()).toBe(true);
    const saved = await response.json();
    await page.reload();
    await page.getByRole('button', { name: `連線 ${name}`, exact: true }).focus();
    await page.getByRole('button', { name: `編輯 ${name}`, exact: true }).click();
    const form = page.locator('#connection-form');
    await expect(form.getByLabel('文字編碼', { exact: true })).toHaveValue(label);
    await form.getByRole('button', { name: '儲存連線' }).click();
    await expect(page.locator('#connection-editor')).not.toBeVisible();
    const reread = await page.request.get(`api/connections/${saved.id}`);
    expect((await reread.json()).params['character-encoding']).toBe(label);
    await page.request.delete(`api/connections/${saved.id}`, { headers });
  }
});

test('encoding: editor offers terminal encodings and removes them for RDP', async ({ page }) => {
  await login(page);
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  const form = page.locator('#connection-form');
  await selectProtocol(form, 'ssh');
  await form.getByText('進階設定', { exact: true }).click();
  await form.getByText('終端機外觀與輸入', { exact: true }).click();
  const encoding = form.getByLabel('文字編碼', { exact: true });
  await expect(encoding).toHaveValue('');
  for (const [value] of cases.slice(1)) await encoding.selectOption(value);
  await encoding.selectOption('Big5');
  await selectProtocol(form, 'telnet');
  await expect(encoding).toHaveValue('Big5');
  await selectProtocol(form, 'rdp');
  await expect(encoding).toBeHidden();
  await form.getByLabel('連線名稱').fill('Unicode 中文 日本語 한국어 😀');
  await form.getByLabel('主機名稱或 IP').fill('example.invalid');
  await form.getByRole('button', { name: '儲存連線' }).click();
  await expect(page.locator('#connection-editor')).not.toBeVisible();
  const saved = (await (await page.request.get('api/connections')).json())
    .find(connection => connection.name === 'Unicode 中文 日本語 한국어 😀');
  expect(saved.params['character-encoding']).toBeUndefined();
  await page.request.delete(`api/connections/${saved.id}`, {
    headers: { Origin: new URL(page.url()).origin },
  });
});

for (const protocol of ['ssh', 'telnet']) {
  for (const [encoding, text, bytes] of cases) {
    test(`encoding: ${protocol} ${encoding} displays and sends exact text bytes`, async ({ page }, info) => {
      await login(page);
      const headers = { Origin: new URL(page.url()).origin };
      const name = `${protocol} ${encoding} ${Date.now()}`;
      const params = {
        hostname: process.env.CRACAMOLE_E2E_BACKEND_CONTAINER ? '127.0.0.1' : 'backend',
        port: process.env.CRACAMOLE_E2E_BACKEND_CONTAINER
          ? (protocol === 'ssh' ? '26022' : '26023') : '',
        username: 'tester', password: 'tester-pw', 'character-encoding': encoding,
      };
      if (protocol === 'ssh') {
        params['host-key'] = remoteFile('/etc/ssh/ssh_host_ed25519_key.pub')
          .trim().split(/\s+/).slice(0, 2).join(' ');
      } else {
        params['login-success-regex'] = ':~[$] ';
        params['login-failure-regex'] = 'Login incorrect';
      }
      const response = await page.request.post('api/connections', { headers, data: { name, protocol, params } });
      expect(response.ok()).toBe(true);
      const saved = await response.json();
      const connect = async () => {
        await page.reload();
        await page.getByRole('button', { name: `連線 ${name}`, exact: true }).click();
        await expect(page.locator('#terminal-status')).toHaveText('已連線');
        await expect.poll(() => visibleInk(page)).toBeGreaterThan(100);
        await page.locator('#terminal').click({ position: { x: 20, y: 20 } });
      };
      const show = async wire => {
        await page.keyboard.type(`printf '\\033[2J\\033[H\\033[?25l${octal(wire)}\\r\\n'`);
        await page.keyboard.press('Enter');
        const box = await page.locator('#terminal').boundingBox();
        const clip = { x: box.x, y: box.y, width: 40, height: 18 };
        await expect.poll(() => visibleInk(page, clip)).toBeGreaterThan(30);
        await page.waitForTimeout(250);
        return page.screenshot({ clip });
      };
      await connect();
      const actual = await show(bytes);
      await info.attach('legacy-composited-screen', { body: actual, contentType: 'image/png' });
      // Capture input in raw mode, so the remote shell cannot reinterpret multibyte input.
      const file = `/tmp/text-${Date.now()}`;
      await page.keyboard.type(`stty raw -echo; touch ${file}-ready; dd bs=1 count=${bytes.length} of=${file} 2>/dev/null; stty sane`);
      await page.keyboard.press('Enter');
      await expect.poll(() => backend(`test -f ${file}-ready && echo ready || true`)).toBe('ready');
      await page.getByRole('button', { name: '剪貼簿', exact: true }).click();
      await page.locator('#clipboard-text').fill(text);
      await page.getByRole('button', { name: '貼上到終端機' }).click();
      await expect.poll(() => backend(`od -An -tx1 ${file} 2>/dev/null || true`)).toBe(bytes.map(byte => byte.toString(16).padStart(2, '0')).join(' '));
      // The same Unicode text in UTF-8 must produce identical composited glyph pixels.
      const update = await page.request.put(`api/connections/${saved.id}`, {
        headers,
        data: { name, protocol, params: { ...params, 'character-encoding': 'UTF-8' } },
      });
      expect(update.ok()).toBe(true);
      await connect();
      expect(await show([...Buffer.from(text, 'utf8')])).toEqual(actual);
      await page.request.delete(`api/connections/${saved.id}`, { headers });
    });
  }
}
