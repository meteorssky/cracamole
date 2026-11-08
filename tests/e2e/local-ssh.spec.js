// Windows 本機跨介面回歸：Rust SSH fixture 與 Rust Web 服務由外層測試啟動。
const { test, expect } = require('@playwright/test');
const { login, ready, visibleInk } = require('./helpers');

test.skip(!process.env.CRACAMOLE_LOCAL_SSH_PORT, '需由 Rust 本機 SSH 測試啟動 fixture');

test('local SSH renders composited terminal and closes normally', async ({ page, request }) => {
  await ready(request);
  await page.addInitScript(() => {
    const NativeWebSocket = window.WebSocket;
    window.__testCloseCodes = [];
    window.WebSocket = class extends NativeWebSocket {
      constructor(...args) {
        super(...args);
        this.addEventListener('close', event => window.__testCloseCodes.push(event.code));
      }
    };
  });
  await login(page);
  const response = await page.request.post('api/connections', {
    headers: { Origin: new URL(page.url()).origin },
    data: {
      name: 'Local SSH regression', protocol: 'ssh',
      params: { hostname: '127.0.0.1', port: process.env.CRACAMOLE_LOCAL_SSH_PORT,
        username: 'alice', password: 'secret', 'host-key': process.env.CRACAMOLE_LOCAL_SSH_KEY },
    },
  });
  expect(response.status()).toBe(201);
  await page.reload();
  await page.getByRole('button', { name: '連線 Local SSH regression', exact: true }).click();
  await expect(page.locator('#terminal-status')).toHaveText('已連線');
  await expect.poll(() => visibleInk(page), { timeout: 20000 }).toBeGreaterThan(100);
  await page.locator('#terminal').click({ position: { x: 30, y: 30 } });
  await page.keyboard.type('browser-test');
  await page.waitForTimeout(500);
  await page.getByRole('button', { name: '中斷連線', exact: true }).click();
  await expect.poll(() => page.evaluate(() => window.__testCloseCodes)).toContain(1000);
});
