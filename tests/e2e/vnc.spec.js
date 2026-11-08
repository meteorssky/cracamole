const { test, expect } = require('@playwright/test');
const { ready, login, visibleInk, addConnection } = require('./helpers');

// The VNC fixture is tests/vnc (TigerVNC with VNC password authentication). Forward its port first:
//   ssh -N -L 15901:127.0.0.1:15901 user@docker-host
// The fixture-backed tests are skipped unless CRACAMOLE_VNC_PORT is set (scripts/vnc_dev.sh sets it).
const VNC_HOST = process.env.CRACAMOLE_VNC_HOST || '127.0.0.1';
const VNC_PORT = process.env.CRACAMOLE_VNC_PORT || '';
const VNC_PASSWORD = process.env.CRACAMOLE_VNC_PASSWORD || 'fixture1';

test.beforeAll(async ({ request }) => ready(request));

test.beforeEach(async ({ page }) => {
  page.on('pageerror', error => {
    throw error;
  });
});

const section = (form, title) => form.locator('summary', { hasText: title });

test('vnc: the editor shows macOS guidance and VNC settings, and saves only VNC parameters', async ({ page }) => {
  const name = `VNC editor ${Date.now()}`;
  await login(page);
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  const form = page.locator('#connection-form');
  await form.getByLabel('協定', { exact: true }).selectOption('vnc');
  await expect(form.getByLabel('連接埠')).toHaveAttribute('placeholder', '5900');
  await expect(form.getByText('VNC 與 macOS 螢幕共享', { exact: true })).toBeVisible();
  await expect(form.getByText('系統設定', { exact: false })).toBeVisible();
  await expect(section(form, 'VNC 畫面與剪貼簿')).toBeVisible();
  await expect(section(form, '連線控制')).toBeVisible();
  for (const hidden of ['終端機外觀與輸入', 'SSH 連線選項', 'RDP 畫面與效能', 'RDP 工作階段']) {
    await expect(section(form, hidden), hidden).toBeHidden();
  }
  await expect(form.getByLabel('網域（可選）')).toBeHidden();

  await form.getByLabel('連線名稱').fill(name);
  await form.getByLabel('主機名稱或 IP').fill('mac.example.test');
  await form.getByLabel('帳號', { exact: true }).fill('admin');
  await form.getByLabel('密碼', { exact: true }).fill('mac-secret-pw');
  await section(form, 'VNC 畫面與剪貼簿').click();
  await form.getByLabel('色彩深度').selectOption('16');
  await form.getByLabel('剪貼簿文字編碼').selectOption('UTF-8');
  await form.getByRole('button', { name: '儲存連線' }).click();
  await expect(page.locator('#connection-editor')).not.toBeVisible();
  await expect(page.getByRole('button', { name: `連線 ${name}`, exact: true })).toBeVisible();

  const saved = (await (await page.request.get('api/connections')).json()).find(c => c.name === name);
  expect(saved.protocol).toBe('vnc');
  expect(saved.params.password).toBeUndefined();
  expect(saved.configured_secrets).toContain('password');
  expect(saved.params['color-depth']).toBe('16');
  expect(saved.params['clipboard-encoding']).toBe('UTF-8');
  for (const foreign of ['terminal-type', 'func-keys-and-keypad', 'host-key', 'security', 'ignore-cert']) {
    expect(saved.params[foreign], foreign).toBeUndefined();
  }
  await expect(
    page.getByRole('article').filter({ hasText: name }).getByText('admin@mac.example.test:5900'),
  ).toBeVisible();
});

test('vnc: connect to TigerVNC, paint the desktop, keyboard and mouse reach the remote session', async ({ page }) => {
  test.skip(!VNC_PORT, 'needs the TigerVNC fixture (scripts/vnc_dev.sh)');
  const name = `VNC ${Date.now()}`;
  await login(page);
  await addConnection(page, name, 'vnc', VNC_HOST, VNC_PORT, VNC_PASSWORD);
  await page.getByRole('button', { name: `連線 ${name}`, exact: true }).click();
  await expect(page.locator('#terminal-status')).toHaveText('已連線', { timeout: 30000 });
  // The #336699 desktop alone is far more than a few hundred "inked" pixels.
  await expect.poll(() => visibleInk(page), { timeout: 30000 }).toBeGreaterThan(20000);
  const terminal = page.locator('#terminal');
  await terminal.click({ position: { x: 100, y: 80 } });
  // Earlier runs may have filled the shared xterm; start from a blank window.
  await page.keyboard.type('clear');
  await page.keyboard.press('Enter');
  await page.waitForTimeout(1500);
  const before = await visibleInk(page);

  await page.keyboard.type(`echo cracamole-vnc-${Date.now()}`);
  await page.keyboard.press('Enter');
  // Text drawn by the remote xterm arrives as framebuffer updates and adds light pixels.
  await expect.poll(() => visibleInk(page), { timeout: 20000 }).toBeGreaterThan(before + 150);

  await page.getByRole('button', { name: '中斷連線', exact: true }).click();
  await expect(page.locator('#terminal-status')).not.toHaveText('已連線');
});

test('vnc: a wrong password is reported as an authentication failure with a way to fix it', async ({ page }) => {
  test.skip(!VNC_PORT, 'needs the TigerVNC fixture (scripts/vnc_dev.sh)');
  const name = `VNC wrong ${Date.now()}`;
  await login(page);
  await addConnection(page, name, 'vnc', VNC_HOST, VNC_PORT, 'definitely-wrong');
  await page.getByRole('button', { name: `連線 ${name}`, exact: true }).click();
  await expect(page.getByText('驗證失敗', { exact: true })).toBeVisible({ timeout: 30000 });
  await expect(page.getByRole('button', { name: '編輯連線', exact: true })).toBeVisible();
});
