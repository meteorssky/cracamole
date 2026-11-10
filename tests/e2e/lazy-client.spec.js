const { test, expect } = require('@playwright/test');
const { ready, login, addConnection } = require('./helpers');

test.beforeEach(async ({ request }) => ready(request));

test('client loads on demand and a failed load can be retried', async ({ page }, testInfo) => {
  const name = `延遲載入測試 ${testInfo.project.name} ${Date.now()}`;
  let requests = 0;
  await page.route('**/assets/vendor/guacamole.js', async route => {
    requests += 1;
    if (requests === 1) await route.abort('failed');
    else await route.continue();
  });
  await login(page);
  await addConnection(page, name, 'telnet', '127.0.0.1', '1');
  expect(requests).toBe(0);
  await page.getByRole('button', { name: `連線 ${name}`, exact: true }).click();
  await expect(page.getByText('無法載入連線元件')).toBeVisible();
  expect(requests).toBe(1);
  await page.getByRole('button', { name: '重新連線' }).click();
  await expect.poll(() => requests).toBe(2);
  await expect.poll(() => page.evaluate(() => Boolean(window.Guacamole?.Client))).toBe(true);
});

test('closing while the client loads does not start a tunnel', async ({ page }, testInfo) => {
  const name = `取消載入測試 ${testInfo.project.name} ${Date.now()}`;
  let release;
  const held = new Promise(resolve => { release = resolve; });
  let requested = false;
  let sockets = 0;
  page.on('websocket', () => { sockets += 1; });
  await page.route('**/assets/vendor/guacamole.js', async route => {
    requested = true;
    await held;
    await route.continue();
  });
  await login(page);
  await addConnection(page, name, 'telnet', '127.0.0.1', '1');
  await page.getByRole('button', { name: `連線 ${name}`, exact: true }).click();
  await expect.poll(() => requested).toBe(true);
  await page.getByRole('button', { name: '回到連線庫', exact: true }).click();
  release();
  await expect.poll(() => page.evaluate(() => Boolean(window.Guacamole?.Client))).toBe(true);
  await expect(page.getByText('選擇一台主機')).toBeVisible();
  expect(sockets).toBe(0);
});
