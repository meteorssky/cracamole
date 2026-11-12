const { test, expect } = require('@playwright/test');
const AxeBuilder = require('@axe-core/playwright').default;
const { ready, login } = require('./helpers');

test('portable settings can be reviewed and saved without clipboard access', async ({ page, request }) => {
  await ready(request);
  await page.addInitScript(() => {
    let stored = '';
    Object.defineProperty(navigator, 'clipboard', { value: {
      writeText: async text => { stored = text; },
      readText: async () => stored,
    }, configurable: true });
  });
  await login(page);
  const origin = new URL(page.url()).origin;
  const names = ['交換甲', '交換乙'].map(name => name + Date.now());
  for (const name of names) {
    const response = await page.request.post('api/connections', {
      headers: { Origin: origin },
      data: { name, protocol: 'ssh', params: { hostname: 'example.test', username: 'alice', password: 'fixture-secret-password' } },
    });
    expect(response.status()).toBe(201);
  }
  await page.reload();
  await page.getByRole('button', { name: '連線 ' + names[0], exact: true }).focus();
  await page.getByRole('button', { name: '匯出 ' + names[0] + ' 的設定', exact: true }).click();
  const single = page.locator('#portable-dialog');
  await expect(single.locator('#portable-results input:checked')).toHaveCount(1);
  await expect(single.locator('#portable-results input:checked').locator('..')).toContainText(names[0]);
  await single.getByRole('button', { name: '關閉' }).click();
  await page.getByRole('button', { name: '匯出設定' }).click();
  const dialog = page.locator('#portable-dialog');
  const accessibility = await new AxeBuilder({ page }).include('#portable-dialog')
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa']).analyze();
  expect(accessibility.violations.filter(v => ['serious', 'critical'].includes(v.impact))).toEqual([]);
  for (const box of await dialog.locator('#portable-results input').all()) {
    const label = await box.locator('..').textContent();
    if (!names.some(name => label.includes(name))) await box.uncheck();
    else await box.check();
  }
  await expect(dialog.locator('#portable-selection-count')).toContainText('已選取 2 筆');
  await dialog.getByRole('button', { name: '清除選取' }).click();
  await expect(dialog.getByRole('button', { name: '產生交換資料' })).toBeDisabled();
  await expect(dialog.locator('#portable-selection-count')).toContainText('已選取 0 筆');
  await dialog.getByRole('button', { name: '只選取本頁' }).click();
  expect(await dialog.locator('#portable-results input:checked').count())
    .toBe(Math.min(100, await dialog.locator('#portable-results input').count()));
  await dialog.getByRole('button', { name: '清除選取' }).click();
  for (const box of await dialog.locator('#portable-results input').all()) {
    const label = await box.locator('..').textContent();
    if (names.some(name => label.includes(name))) await box.check();
  }
  await expect(dialog.getByRole('button', { name: '產生交換資料' })).toBeEnabled();
  await dialog.getByRole('button', { name: '產生交換資料' }).click();
  await expect(dialog.getByRole('status')).toContainText('已產生交換資料');
  const content = await dialog.getByLabel('交換資料').inputValue();
  expect(content).not.toContain('fixture-secret-password');
  expect(JSON.parse(content).connections).toHaveLength(2);
  await dialog.getByLabel('匯出格式').selectOption('uri');
  await expect(dialog.getByLabel('交換資料')).toHaveValue('');
  await dialog.getByRole('button', { name: '產生交換資料' }).click();
  await expect(dialog.getByLabel('交換資料')).toHaveValue(/ssh:\/\/alice@example\.test/);
  const uri = await dialog.getByLabel('交換資料').inputValue();
  expect(uri.split('\n')).toHaveLength(2);
  await dialog.getByLabel('匯出格式').selectOption('json');
  await dialog.getByRole('button', { name: '產生交換資料' }).click();
  await expect(dialog.getByLabel('交換資料')).toHaveValue(content);
  await dialog.getByRole('button', { name: '複製到剪貼簿' }).click();
  await expect(dialog.getByRole('status')).toContainText('已複製');
  await dialog.getByRole('button', { name: '關閉' }).click();
  await page.getByRole('button', { name: '匯入設定' }).click();
  await dialog.getByRole('button', { name: '從剪貼簿貼上' }).click();
  await expect(dialog.getByLabel('交換資料')).toHaveValue(content);
  await dialog.getByRole('button', { name: '驗證並預覽' }).click();
  await expect(dialog.getByRole('status')).toContainText('已驗證 2 筆');
  await dialog.getByRole('button', { name: '檢查並新增 ' + names[0] }).click();
  const editor = page.locator('#connection-editor');
  await expect(editor.getByRole('heading', { name: '檢查並新增匯入連線' })).toBeVisible();
  await expect(editor.locator('[name="name"]')).toHaveValue(names[0]);
  await expect(editor.locator('[name="hostname"]')).toHaveValue('example.test');
  await expect(editor.locator('[name="password"]')).toHaveValue('');
  await editor.getByRole('button', { name: '儲存連線', exact: true }).click();
  await expect(editor).toBeHidden();
  await page.getByRole('button', { name: '匯入設定' }).click();
  await expect(dialog.getByRole('button', { name: '檢查並新增 ' + names[1] })).toBeVisible();
  await dialog.getByRole('button', { name: '關閉' }).click();
  const list = await (await page.request.get('api/connections')).json();
  expect(list.filter(connection => names.includes(connection.name))).toHaveLength(3);
});

test('stale preview is discarded and logout clears exchange data', async ({ page, request }) => {
  await ready(request);
  await login(page, { fresh: true });
  await page.getByRole('button', { name: '匯入設定' }).click();
  const dialog = page.locator('#portable-dialog');
  const field = dialog.getByLabel('交換資料');
  const content = JSON.stringify({ format: 'cracamole-connections', version: 1,
    connections: [{ name: '舊預覽', protocol: 'ssh', params: { hostname: 'example.test' } }] });
  let release;
  let intercepted;
  const pending = new Promise(resolve => { release = resolve; });
  const started = new Promise(resolve => { intercepted = resolve; });
  await page.route('**/api/connections/import-preview', async route => {
    intercepted();
    await pending;
    await route.continue();
  });
  await field.fill(content);
  await dialog.getByRole('button', { name: '驗證並預覽' }).click();
  await started;
  await field.fill('修改後內容');
  const response = page.waitForResponse('**/api/connections/import-preview');
  release();
  await response;
  await expect(dialog.getByRole('button', { name: '檢查並新增 舊預覽' })).toHaveCount(0);
  await expect(dialog.getByRole('status')).toHaveText('');
  await dialog.getByRole('button', { name: '關閉' }).click();
  await page.getByRole('button', { name: '登出', exact: true }).click();
  await login(page);
  await page.getByRole('button', { name: '匯入設定' }).click();
  await expect(field).toHaveValue('');
});

test('clipboard denial offers manual paste', async ({ page, request }) => {
  await ready(request);
  await page.addInitScript(() => Object.defineProperty(navigator, 'clipboard', { value: {
    readText: async () => { throw new Error('denied'); },
  }, configurable: true }));
  await login(page);
  await page.getByRole('button', { name: '匯入設定' }).click();
  const dialog = page.locator('#portable-dialog');
  await dialog.getByRole('button', { name: '從剪貼簿貼上' }).click();
  await expect(dialog.getByRole('status')).toContainText('請手動貼入文字');
  await expect(dialog.getByLabel('交換資料')).toBeEditable();
});

test('pasting again invalidates the previous reviewed import', async ({ page, request }) => {
  await ready(request);
  await login(page, { fresh: true });
  await page.getByRole('button', { name: '匯入設定' }).click();
  const dialog = page.locator('#portable-dialog');
  const field = dialog.getByLabel('交換資料');
  const imported = name => JSON.stringify({ format: 'cracamole-connections', version: 1,
    connections: [{ name, protocol: 'ssh', params: { hostname: 'example.test' } }] });
  await field.fill(imported('已核對'));
  await dialog.getByRole('button', { name: '驗證並預覽' }).click();
  await expect(dialog.getByRole('button', { name: '新增全部' })).toBeVisible();
  await page.evaluate(content => Object.defineProperty(navigator, 'clipboard', { configurable: true,
    value: { readText: async () => content } }), imported('新資料'));
  await dialog.getByRole('button', { name: '從剪貼簿貼上' }).click();
  await expect(field).toHaveValue(imported('新資料'));
  await expect(dialog.getByRole('button', { name: '新增全部' })).toBeHidden();
  await expect(dialog.getByRole('button', { name: '檢查並新增 已核對' })).toHaveCount(0);
  await dialog.getByRole('button', { name: '驗證並預覽' }).click();
  await expect(dialog.getByRole('button', { name: '檢查並新增 新資料' })).toBeVisible();
  await expect(dialog.getByRole('button', { name: '新增全部' })).toBeVisible();
});

test('export search and paging retain selections and replace them only on request', async ({ page, request }) => {
  await ready(request);
  await login(page);
  await page.evaluate(async () => {
    const { state } = await import('/assets/js/state.js');
    state.set('connections', Array.from({ length: 103 }, (_, index) => ({
      id: `fixture-${index}`, name: `主機 ${String(index).padStart(3, '0')}`,
      protocol: 'ssh', params: { hostname: `host-${index}.example`, username: 'tester' },
    })));
  });
  await page.getByRole('button', { name: '匯出設定' }).click();
  const dialog = page.locator('#portable-dialog');
  await expect(dialog.locator('#portable-page-count')).toHaveText('第 1 / 2 頁（符合 103 筆）');
  await dialog.getByRole('button', { name: '只選取本頁' }).click();
  await expect(dialog.locator('#portable-results input:checked')).toHaveCount(100);
  await dialog.getByRole('button', { name: '下一頁' }).click();
  await expect(dialog.locator('#portable-selection-count')).toContainText('其中 100 筆不在本頁');
  await dialog.getByRole('button', { name: '只選取本頁' }).click();
  await expect(dialog.locator('#portable-results input:checked')).toHaveCount(3);
  await dialog.getByLabel('搜尋匯出清單').fill('host-102.example');
  await expect(dialog.locator('#portable-page-count')).toHaveText('第 1 / 1 頁（符合 1 筆）');
  await expect(dialog.locator('#portable-selection-count')).toContainText('其中 2 筆不在本頁');
  await dialog.getByRole('button', { name: '只選取本頁' }).click();
  await expect(dialog.locator('#portable-results input:checked')).toHaveCount(1);
  await expect(dialog.locator('#portable-results input:checked').locator('..')).toContainText('主機 102');
});

test('bulk import requires confirmation and creates all reviewed connections', async ({ page, request }, info) => {
  await ready(request);
  await login(page);
  const tag = `${info.project.name}-${Date.now()}`;
  const names = [`匯入甲 ${tag}`, `匯入乙 ${tag}`];
  const content = JSON.stringify({ format: 'cracamole-connections', version: 1,
    connections: names.map((name, index) => ({ name, protocol: 'ssh', params: { hostname: `host${index}.example` } })) });
  await page.getByRole('button', { name: '匯入設定' }).click();
  const dialog = page.locator('#portable-dialog');
  await dialog.getByLabel('交換資料').fill(content);
  await dialog.getByRole('button', { name: '驗證並預覽' }).click();
  await expect(dialog.getByRole('status')).toContainText('已驗證 2 筆');
  await dialog.getByRole('button', { name: '新增全部' }).click();
  const confirm = page.getByRole('dialog', { name: '新增全部匯入設定？' });
  await confirm.getByRole('button', { name: '取消' }).click();
  let list = await (await page.request.get('api/connections')).json();
  expect(list.filter(item => names.includes(item.name))).toHaveLength(0);
  await dialog.getByRole('button', { name: '新增全部' }).click();
  await confirm.getByRole('button', { name: '新增全部' }).click();
  await expect(dialog.getByRole('status')).toContainText('已新增 2 筆');
  list = await (await page.request.get('api/connections')).json();
  expect(list.filter(item => names.includes(item.name))).toHaveLength(2);
});
