// Windows 本機跨介面回歸：Rust SSH fixture 與 Rust Web 服務由外層測試啟動。
const { test, expect } = require('@playwright/test');
const { generateKeyPairSync } = require('node:crypto');
const { login, ready, visibleInk } = require('./helpers');

test.skip(!process.env.CRACAMOLE_LOCAL_SSH_PORT, '需由 Rust 本機 SSH 測試啟動 fixture');

test('local SSH renders composited terminal and closes normally', async ({ page, request }) => {
  await ready(request);
  await page.addInitScript(() => {
    const NativeWebSocket = window.WebSocket;
    window.__testCloseCodes = [];
    window.__testClipboardDenied = false;
    Object.defineProperty(navigator, 'clipboard', { configurable: true, value: {
      readText: async () => {
        if (window.__testClipboardDenied) throw new Error('fixture denied');
        return 'fixture-quick-paste';
      },
    } });
    window.WebSocket = class extends NativeWebSocket {
      constructor(...args) {
        super(...args);
        this.addEventListener('close', event => window.__testCloseCodes.push(event.code));
      }
    };
  });
  await login(page);
  const logsButton = page.getByRole('button', { name: '系統日誌' });
  await logsButton.click();
  const logsDialog = page.getByRole('dialog', { name: '系統日誌' });
  await expect(logsDialog.getByRole('button', { name: '關閉', exact: true }).first()).toBeFocused();
  await page.keyboard.press('Escape');
  await expect(logsButton).toBeFocused();
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  const draftEditor = page.getByRole('dialog', { name: '新增連線' });
  await draftEditor.getByLabel('主機名稱或 IP').fill('127.0.0.1');
  await draftEditor.getByLabel('連接埠').fill(process.env.CRACAMOLE_LOCAL_SSH_PORT);
  await draftEditor.getByLabel('密碼', { exact: true }).fill('never-send-to-probe');
  const probeRequest = page.waitForRequest(req => req.method() === 'POST' && req.url().endsWith('/api/identity-probe'));
  await draftEditor.getByRole('button', { name: '探測主機身分' }).click();
  expect((await probeRequest).postDataJSON()).toEqual({ protocol: 'ssh', hostname: '127.0.0.1', port: process.env.CRACAMOLE_LOCAL_SSH_PORT });
  await expect(draftEditor.getByRole('status')).toContainText('尚未比對已保存信任');
  await draftEditor.getByRole('button', { name: '已核對，填入信任設定' }).click();
  let identityConfirm = page.getByRole('dialog', { name: '確認主機身分' });
  await identityConfirm.getByRole('button', { name: '取消' }).click();
  await expect(draftEditor.locator('[name="host-key"]')).toHaveValue('');
  await draftEditor.getByRole('button', { name: '已核對，填入信任設定' }).click();
  identityConfirm = page.getByRole('dialog', { name: '確認主機身分' });
  await identityConfirm.getByRole('button', { name: '已核對，填入設定' }).click();
  await expect(draftEditor.locator('[name="host-key"]')).toHaveValue(process.env.CRACAMOLE_LOCAL_SSH_KEY);
  await draftEditor.getByRole('button', { name: '取消' }).click();
  const response = await page.request.post('api/connections', {
    headers: { Origin: new URL(page.url()).origin },
    data: {
      name: 'Local SSH regression', protocol: 'ssh',
      params: { hostname: '127.0.0.1', port: process.env.CRACAMOLE_LOCAL_SSH_PORT,
        username: 'alice', password: 'secret', 'host-key': process.env.CRACAMOLE_LOCAL_SSH_KEY },
    },
  });
  expect(response.status()).toBe(201);
  const firstId = (await response.json()).id;
  const other = await page.request.post('api/connections', {
    headers: { Origin: new URL(page.url()).origin },
    data: {
      name: '另一台測試主機', protocol: 'ssh',
      params: { hostname: '127.0.0.1', port: process.env.CRACAMOLE_LOCAL_SSH_PORT,
        username: 'alice', password: 'secret', 'host-key': process.env.CRACAMOLE_LOCAL_SSH_KEY,
        'read-only': 'true' },
    },
  });
  expect(other.status()).toBe(201);
  const secondId = (await other.json()).id;
  await page.reload();
  await page.getByRole('button', { name: '連線 Local SSH regression', exact: true }).focus();
  await page.getByRole('button', { name: '編輯 Local SSH regression' }).click();
  const editor = page.getByRole('dialog', { name: '編輯連線' });
  const probe = editor.getByRole('button', { name: '探測主機身分' });
  await probe.click();
  await expect(editor.getByRole('status')).toContainText('與已保存的信任相符');
  await editor.getByLabel('主機名稱或 IP').fill('localhost');
  await expect(probe).toBeEnabled();
  await expect(editor.getByRole('status')).toBeHidden();
  await editor.getByRole('button', { name: '取消' }).click();
  await page.getByRole('button', { name: '連線 Local SSH regression', exact: true }).click();
  await expect(page.locator('#terminal-status')).toHaveText('已連線');
  await expect(page.locator('.conn-group__title').filter({ hasText: '未分類（無群組）（2）' })).toContainText('1 已連線');
  await expect.poll(() => visibleInk(page), { timeout: 20000 }).toBeGreaterThan(100);
  await page.getByRole('button', { name: '開啟連線庫' }).click();
  await page.getByRole('button', { name: '連線 另一台測試主機' }).click();
  const switchDialog = page.getByRole('dialog', { name: '切換連線？' });
  await expect(switchDialog.getByRole('button', { name: '取消' })).toBeFocused();
  await switchDialog.getByRole('button', { name: '取消' }).click();
  await expect(page.locator('#terminal-status')).toHaveText('已連線');
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('cracamole-recent')))).toEqual([firstId]);
  const back = page.goBack();
  const backDialog = page.getByRole('dialog', { name: '返回連線庫？' });
  await expect(backDialog).toBeVisible();
  await backDialog.getByRole('button', { name: '取消' }).click();
  await back;
  await expect(page.locator('#terminal-status')).toHaveText('已連線');
  const prevented = await page.evaluate(() => {
    const event = new Event('beforeunload', { cancelable: true });
    window.dispatchEvent(event);
    return event.defaultPrevented;
  });
  expect(prevented).toBe(true);
  await page.getByRole('button', { name: '連線 Local SSH regression' }).click();
  await page.getByRole('button', { name: '貼上本機剪貼簿' }).click();
  await page.getByRole('button', { name: '剪貼簿', exact: true }).click();
  await page.getByLabel('剪貼簿內容').fill('fixture-panel-paste');
  await page.getByLabel('剪貼簿內容').press('Control+Enter');
  await expect(page.getByLabel('剪貼簿內容')).toBeHidden();
  await expect(page.locator('#terminal')).toBeFocused();
  await page.evaluate(() => { window.__testClipboardDenied = true; });
  await page.getByRole('button', { name: '貼上本機剪貼簿' }).click();
  await expect(page.getByText('無法讀取本機剪貼簿，請開啟剪貼簿面板後手動貼入。')).toBeVisible();
  await page.locator('#terminal').click({ position: { x: 30, y: 30 } });
  await page.keyboard.type('browser-test');
  await page.keyboard.press('Control+k');
  await expect(page.getByRole('searchbox', { name: '搜尋連線' })).toBeFocused();
  await page.getByRole('button', { name: '收合連線庫' }).click();
  await page.locator('#terminal').click({ position: { x: 30, y: 30 } });
  await page.locator('#terminal').dispatchEvent('pointerdown', { pointerType: 'touch' });
  const mobile = page.getByRole('textbox', { name: '遠端文字輸入' });
  await expect(mobile).toBeFocused();
  await mobile.evaluate(element => {
    element.dispatchEvent(new CompositionEvent('compositionstart', { bubbles: true }));
    element.value = '行動輸入🙂';
    element.dispatchEvent(new InputEvent('input', { bubbles: true, isComposing: true }));
    element.dispatchEvent(new CompositionEvent('compositionend', { bubbles: true, data: '行動輸入🙂' }));
    element.dispatchEvent(new InputEvent('input', { bubbles: true, inputType: 'insertCompositionText' }));
    element.dispatchEvent(new InputEvent('beforeinput', { bubbles: true, cancelable: true, inputType: 'deleteContentBackward' }));
  });
  await page.keyboard.press('Control+k');
  await expect(page.getByRole('searchbox', { name: '搜尋連線' })).toBeFocused();
  await page.getByRole('button', { name: '收合連線庫' }).click();
  await page.waitForTimeout(500);
  await page.getByRole('button', { name: '開啟連線庫' }).click();
  await page.getByRole('button', { name: '連線 另一台測試主機' }).click();
  await page.getByRole('dialog', { name: '切換連線？' })
    .getByRole('button', { name: '中斷並切換' }).click();
  await expect(page.locator('#terminal-name')).toHaveText('另一台測試主機');
  await expect(page.locator('#terminal-status')).toHaveText('已連線');
  await expect(page.getByRole('button', { name: '貼上本機剪貼簿' })).toBeDisabled();
  await page.getByRole('button', { name: '剪貼簿', exact: true }).click();
  await expect(page.getByRole('button', { name: '貼上到終端機' })).toBeDisabled();
  await page.getByLabel('剪貼簿內容').press('Escape');
  await expect(page.getByLabel('剪貼簿內容')).toBeHidden();
  await page.getByRole('button', { name: '最近連線 Local SSH regression' }).click();
  await page.getByRole('dialog', { name: '切換連線？' })
    .getByRole('button', { name: '取消' }).click();
  await expect(page.locator('#terminal-name')).toHaveText('另一台測試主機');
  const backAgain = page.goBack();
  await page.getByRole('dialog', { name: '返回連線庫？' })
    .getByRole('button', { name: '中斷並返回' }).click();
  await backAgain;
  await expect(page.locator('#terminal-status')).toHaveText('未連線');
  await expect.poll(() => page.evaluate(() => window.__testCloseCodes)).toEqual([1000, 1000]);
  expect(await page.evaluate(() => JSON.parse(localStorage.getItem('cracamole-recent'))))
    .toEqual([firstId, secondId]);
  await page.reload();
  await expect(page.getByRole('button', { name: '最近連線 另一台測試主機' })).toBeVisible();
  const names = await page.getByRole('button', { name: /^連線 (Local SSH regression|另一台測試主機)$/ }).allTextContents();
  expect(names).toEqual(['另一台測試主機', 'Local SSH regression']);

  const { publicKey } = generateKeyPairSync('ed25519');
  const raw = publicKey.export({ format: 'der', type: 'spki' }).subarray(-32);
  const wrongKey = 'ssh-ed25519 ' + Buffer.concat([
    Buffer.from('0000000b7373682d6564323535313900000020', 'hex'), raw,
  ]).toString('base64');
  for (const [name, hostKey] of [['待核對測試', ''], ['身分不符測試', wrongKey]]) {
    const created = await page.request.post('api/connections', {
      headers: { Origin: new URL(page.url()).origin },
      data: { name, protocol: 'ssh', params: {
        hostname: '127.0.0.1', port: process.env.CRACAMOLE_LOCAL_SSH_PORT,
        username: 'alice', password: 'secret', ...(hostKey ? { 'host-key': hostKey } : {}),
      } },
    });
    expect(created.status()).toBe(201);
  }
  await page.reload();
  for (const [name, status, heading] of [
    ['待核對測試', '待核對身分', '尚未信任 SSH 主機金鑰'],
    ['身分不符測試', '身分不符', '主機金鑰已變更或遭撤銷'],
  ]) {
    await page.getByRole('button', { name: `連線 ${name}`, exact: true }).click();
    await expect(page.locator('#overlay-title')).toHaveText(heading);
    await expect(page.locator('#terminal-status')).toHaveText(status);
    await page.getByRole('button', { name: '回到連線庫' }).click();
    const row = page.getByRole('article').filter({
      has: page.getByRole('button', { name: `連線 ${name}`, exact: true }),
    });
    await expect(row).toContainText(status);
    if (status === '身分不符') {
      await expect(row).toContainText('已保存指紋');
      await expect(row).not.toContainText('已信任');
    }
  }
});
