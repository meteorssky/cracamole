// 專用 loopback Rust RDP fixture；不使用使用者的剪貼簿或既有工作階段。
const { test, expect } = require('@playwright/test');
const { login, ready, selectProtocol } = require('./helpers');

test.skip(!process.env.CRACAMOLE_LOCAL_RDP_PORT || !process.env.CRACAMOLE_LOCAL_RDP_FINGERPRINT,
  '需由外層測試啟動專用本機 RDP fixture');

test('local RDP editor probes and explicitly pins the fixture certificate', async ({ page, request }) => {
  await ready(request);
  await login(page);
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  let editor = page.getByRole('dialog', { name: '新增連線' });
  await selectProtocol(editor, 'rdp');
  const name = `RDP probe ${Date.now()}`;
  await editor.getByLabel('連線名稱').fill(name);
  await editor.getByLabel('主機名稱或 IP').fill('127.0.0.1');
  await editor.getByLabel('連接埠').fill(process.env.CRACAMOLE_LOCAL_RDP_PORT);
  await editor.getByRole('button', { name: '探測主機身分' }).click();
  const formatted = process.env.CRACAMOLE_LOCAL_RDP_FINGERPRINT.toUpperCase().match(/../g).join(':');
  await expect(editor.getByRole('status')).toContainText(formatted);
  await editor.getByRole('button', { name: '已核對，填入信任設定' }).click();
  await page.getByRole('dialog', { name: '確認主機身分' }).getByRole('button', { name: '已核對，填入設定' }).click();
  await expect(editor.locator('[name="server-cert-sha256"]')).toHaveValue(process.env.CRACAMOLE_LOCAL_RDP_FINGERPRINT);
  await editor.getByRole('button', { name: '儲存連線', exact: true }).click();
  await expect(editor).toBeHidden();
  await page.getByRole('button', { name: `連線 ${name}`, exact: true }).focus();
  await page.getByRole('button', { name: `編輯 ${name}` }).click();
  editor = page.getByRole('dialog', { name: '編輯連線' });
  await editor.getByRole('button', { name: '探測主機身分' }).click();
  await expect(editor.getByRole('status')).toContainText('與已保存的信任相符');
  await expect(editor.getByRole('button', { name: '已核對，填入信任設定' })).toBeHidden();
  await editor.getByRole('button', { name: '取消' }).click();
});

test('RDP certificate problems remain visible on the connection card', async ({ page, request }, info) => {
  await ready(request);
  await login(page);
  const suffix = `${info.project.name}-${Date.now()}`;
  const wrongPin = process.env.CRACAMOLE_LOCAL_RDP_FINGERPRINT === '0'.repeat(64)
    ? '1'.repeat(64) : '0'.repeat(64);
  const cases = [
    { name: `RDP 待核對 ${suffix}`, status: '待核對身分', heading: '無法驗證伺服器憑證' },
    { name: `RDP 身分不符 ${suffix}`, status: '身分不符', heading: '憑證已變更', pin: wrongPin },
  ];
  for (const item of cases) {
    const response = await page.request.post('api/connections', {
      headers: { Origin: new URL(page.url()).origin },
      data: { name: item.name, protocol: 'rdp', params: {
        hostname: '127.0.0.1', port: process.env.CRACAMOLE_LOCAL_RDP_PORT,
        username: 'fixture', password: 'fixture-secret',
        security: process.env.CRACAMOLE_LOCAL_RDP_SECURITY || 'tls',
        'disable-copy': 'true', 'disable-paste': 'true',
        ...(item.pin ? { 'server-cert-sha256': item.pin } : {}),
      } },
    });
    expect(response.status()).toBe(201);
  }
  await page.reload();
  for (const item of cases) {
    await page.getByRole('button', { name: `連線 ${item.name}`, exact: true }).click();
    await expect(page.locator('#overlay-title')).toHaveText(item.heading);
    await expect(page.locator('#terminal-status')).toHaveText(item.status);
    await page.getByRole('button', { name: '回到連線庫' }).click();
    const card = page.getByRole('article').filter({
      has: page.getByRole('button', { name: `連線 ${item.name}`, exact: true }),
    });
    await expect(card).toContainText(item.status);
    if (item.pin) {
      await expect(card).toContainText('已保存指紋');
      await expect(card).not.toContainText('已信任');
    }
  }
});
