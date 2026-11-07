// Interface behaviour (see web/GUIDELINES.md §9). Tests prefixed "ui:" need no remote host;
// tests prefixed "mobile:" that connect need the sshd fixture (scripts/e2e_remote.sh).
const { test, expect } = require('@playwright/test');
const AxeBuilder = require('@axe-core/playwright').default;
const { remoteFile, ready, login, addConnection } = require('./helpers');

const origin = page => new URL(page.url()).origin;

// Zero serious/critical WCAG 2.x A/AA violations.
async function expectAccessible(page, where) {
  const results = await new AxeBuilder({ page })
    .withTags(['wcag2a', 'wcag2aa', 'wcag21a', 'wcag21aa', 'wcag22aa'])
    .analyze();
  const serious = results.violations
    .filter(violation => ['serious', 'critical'].includes(violation.impact))
    .map(violation => `${violation.id} (${violation.impact}): ${violation.nodes.map(node => node.target.join(' ')).join(' | ')}`);
  expect(serious, `accessibility violations on ${where}`).toEqual([]);
}

async function removeAllConnections(page) {
  const list = await (await page.request.get('api/connections')).json();
  for (const connection of list) {
    await page.request.delete(`api/connections/${connection.id}`, { headers: { Origin: origin(page) } });
  }
}

test.beforeAll(async ({ request }) => ready(request));
test.beforeEach(async ({ page }) => {
  page.on('pageerror', error => {
    throw error;
  });
});

test('ui: login rejects empty and wrong credentials, and the password can be revealed', async ({ page }) => {
  await page.goto('./');
  const form = page.locator('#login-form');
  await expect(form.getByRole('heading', { name: '登入' })).toBeVisible();
  await form.getByRole('button', { name: '登入', exact: true }).click();
  await expect(form.getByRole('alert')).toHaveText('請輸入管理員帳號與密碼。');
  await form.getByLabel('管理員帳號').fill('admin');
  const password = form.getByLabel('密碼', { exact: true });
  await password.fill('not-the-password');
  await expect(password).toHaveAttribute('type', 'password');
  await form.getByLabel('顯示密碼').check();
  await expect(password).toHaveAttribute('type', 'text');
  await form.getByRole('button', { name: '登入', exact: true }).click();
  await expect(form.getByRole('alert')).toContainText('帳號或密碼不正確');
  await expect(password).toBeFocused();
});

test('ui: first run guides to the first host and opens the editor for the chosen protocol', async ({ page }) => {
  await login(page);
  await removeAllConnections(page);
  await page.reload();
  await expect(page.getByRole('heading', { name: '新增你的第一台主機' })).toBeVisible();
  await page.getByRole('button', { name: '新增 RDP 主機' }).click();
  const form = page.locator('#connection-form');
  await expect(form.getByLabel('協定', { exact: true })).toHaveValue('rdp');
  await expect(form.getByLabel('網域（可選）')).toBeVisible();
  await expect(form.getByLabel('主機金鑰（known_hosts）')).toBeHidden();
  await expect(form.getByLabel('連接埠')).toHaveAttribute('placeholder', '3389');
});

test('ui: the editor shows only the fields of the selected protocol and explains mistakes in place', async ({ page }) => {
  await login(page);
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  const form = page.locator('#connection-form');
  const protocol = form.getByLabel('協定', { exact: true });

  // fields follow the protocol
  await expect(form.getByLabel('主機金鑰（known_hosts）')).toBeVisible();
  await expect(form.getByLabel('登入成功提示')).toBeHidden();
  await expect(form.getByLabel('網域（可選）')).toBeHidden();
  await protocol.selectOption('telnet');
  await expect(form.getByLabel('登入成功提示')).toBeVisible();
  await expect(form.getByLabel('主機金鑰（known_hosts）')).toBeHidden();
  await protocol.selectOption('rdp');
  await expect(form.getByLabel('網域（可選）')).toBeVisible();
  await expect(form.getByRole('checkbox', { name: /不驗證伺服器憑證/ })).toBeVisible();
  await protocol.selectOption('ssh');

  // each mistake is reported next to its field, marks it invalid and moves focus there
  const save = form.getByRole('button', { name: '儲存連線' });
  await save.click();
  await expect(form.getByText('請輸入 1 到 120 個字元的連線名稱。')).toBeVisible();
  await expect(form.getByLabel('連線名稱')).toHaveAttribute('aria-invalid', 'true');
  await expect(form.getByLabel('連線名稱')).toBeFocused();
  await form.getByLabel('連線名稱').fill('validation check');
  await save.click();
  await expect(form.getByText('請填寫主機名稱或 IP（最長 255 字元）。')).toBeVisible();
  await form.getByLabel('主機名稱或 IP').fill('example.invalid');
  await form.getByLabel('連接埠').fill('70000');
  await save.click();
  await expect(form.getByText('連接埠須為 1 到 65535 的數字，或留空使用預設值。')).toBeVisible();
  await expect(form.getByLabel('連接埠')).toHaveAttribute('aria-invalid', 'true');

  // Esc closes the dialog and focus returns to the control that opened it
  await page.keyboard.press('Escape');
  await expect(page.locator('#connection-editor')).not.toBeVisible();
  await expect(page.getByRole('button', { name: '新增連線', exact: true })).toBeFocused();
});

test('ui: the API reports which field is wrong', async ({ page }) => {
  await login(page);
  const response = await page.request.post('api/connections', {
    headers: { Origin: origin(page) },
    data: { name: 'bad port', protocol: 'ssh', params: { hostname: 'h', port: '0' } },
  });
  expect(response.status()).toBe(400);
  expect(await response.json()).toEqual({ error: '連接埠須為 1 到 65535 的數字，或留空使用預設值。', field: 'port' });
});

test('ui: deleting a host asks first, defaults to the safe choice, and can be cancelled', async ({ page }) => {
  const name = `Delete ${Date.now()}`;
  await login(page);
  await addConnection(page, name, 'ssh', 'example.invalid');
  const card = page.getByRole('button', { name: `連線 ${name}`, exact: true });

  await page.getByRole('button', { name: `刪除 ${name}`, exact: true }).click();
  const dialog = page.getByRole('dialog', { name: `刪除「${name}」？` });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('button', { name: '取消' })).toBeFocused();
  await dialog.getByRole('button', { name: '取消' }).click();
  await expect(dialog).toBeHidden();
  await expect(card).toBeVisible();

  await page.getByRole('button', { name: `刪除 ${name}`, exact: true }).click();
  await page.getByRole('button', { name: '刪除這台主機', exact: true }).click();
  await expect(card).toHaveCount(0);
  await expect(page.getByText(`已刪除「${name}」`)).toBeVisible();
});

test('ui: a failed connection says what happened and offers the next step, not a blank screen', async ({ page }) => {
  const name = `Unreachable ${Date.now()}`;
  await login(page);
  await addConnection(page, name, 'ssh', 'no-such-host.invalid');
  await page.getByRole('button', { name: `連線 ${name}`, exact: true }).click();

  const overlay = page.locator('#overlay');
  await expect(overlay).toContainText('找不到主機', { timeout: 30000 });
  await expect(overlay).toContainText('請確認拼字與 DNS');
  await expect(page.locator('#terminal-status')).toHaveText('連線失敗');
  await expect(overlay.getByRole('button', { name: '重新連線' })).toBeVisible();
  await expect(overlay.getByRole('button', { name: '編輯連線' })).toBeVisible();

  await overlay.getByRole('button', { name: '編輯連線' }).click();
  await expect(page.locator('#connection-form').getByLabel('連線名稱')).toHaveValue(name);
  await page.keyboard.press('Escape');

  await overlay.getByRole('button', { name: '回到連線庫' }).click();
  await expect(overlay).toBeHidden();
  await expect(page.getByRole('complementary', { name: '連線庫' })).toBeVisible();
  await expect(page.locator('#terminal-status')).toHaveText('未連線');
});

test('ui: the theme can be switched and is remembered', async ({ page }) => {
  await page.emulateMedia({ colorScheme: 'dark' });
  await login(page);
  const toggle = page.locator('#theme-toggle');
  const html = page.locator('html');
  await expect(toggle).toHaveText('淺色主題');
  await toggle.click();
  await expect(html).toHaveAttribute('data-theme', 'light');
  await expect(toggle).toHaveText('深色主題');
  await page.reload();
  await expect(html).toHaveAttribute('data-theme', 'light');
  await page.locator('#theme-toggle').click();
  await expect(html).toHaveAttribute('data-theme', 'dark');
});

const section = (form, title) => form.locator('summary', { hasText: title });

test('ui: advanced settings are different for SSH, Telnet and RDP', async ({ page }) => {
  await login(page);
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  const form = page.locator('#connection-form');
  const protocol = form.getByLabel('協定', { exact: true });
  const shows = async (visible, hidden) => {
    for (const title of visible) await expect(section(form, title), title).toBeVisible();
    for (const title of hidden) await expect(section(form, title), title).toBeHidden();
  };

  await shows(['終端機外觀與輸入', 'SSH 連線選項', '連線控制'], ['RDP 畫面與效能', 'RDP 工作階段']);
  await protocol.selectOption('telnet');
  await shows(['終端機外觀與輸入', '連線控制'], ['SSH 連線選項', 'RDP 畫面與效能', 'RDP 工作階段']);
  await protocol.selectOption('rdp');
  await shows(['RDP 畫面與效能', 'RDP 工作階段', '連線控制'], ['終端機外觀與輸入', 'SSH 連線選項']);

  await section(form, 'RDP 畫面與效能').click();
  await expect(form.getByRole('checkbox', { name: '顯示桌布' })).toBeVisible();
  await expect(form.getByLabel('字型大小（pt）')).toBeHidden();
  await protocol.selectOption('ssh');
  await section(form, 'SSH 連線選項').click();
  await expect(form.getByLabel('連線後執行的指令')).toBeVisible();
  await expect(form.getByRole('checkbox', { name: '顯示桌布' })).toBeHidden();
});

test('ui: advanced settings are saved, restored, and dropped when the protocol changes', async ({ page }) => {
  const name = `Options ${Date.now()}`;
  await login(page);
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  const form = page.locator('#connection-form');
  await form.getByLabel('連線名稱').fill(name);
  await form.getByLabel('主機名稱或 IP').fill('example.invalid');
  await section(form, '終端機外觀與輸入').click();
  await form.getByLabel('字型大小（pt）').fill('16');
  await form.getByLabel('配色').selectOption('green-black');
  await form.getByLabel('捲動緩衝（行數）').fill('5000');
  await section(form, '連線控制').click();
  await form.getByRole('checkbox', { name: /唯讀/ }).check();
  await form.getByRole('button', { name: '儲存連線' }).click();
  await expect(page.locator('#connection-editor')).not.toBeVisible();

  const stored = async () => (await (await page.request.get('api/connections')).json()).find(c => c.name === name);
  let params = (await stored()).params;
  expect(params).toMatchObject({ 'font-size': '16', 'color-scheme': 'green-black', scrollback: '5000', 'read-only': 'true' });
  expect(params.command).toBeUndefined(); // unset options are not stored at all

  // reopening restores them
  await page.getByRole('button', { name: `編輯 ${name}`, exact: true }).click();
  await expect(form.getByLabel('字型大小（pt）')).toHaveValue('16');
  await expect(form.getByLabel('配色')).toHaveValue('green-black');
  await expect(form.getByRole('checkbox', { name: /唯讀/ })).toBeChecked();

  // switching to RDP removes the terminal-only settings and keeps the shared ones
  await form.getByLabel('協定', { exact: true }).selectOption('rdp');
  await section(form, 'RDP 畫面與效能').click();
  await form.getByRole('checkbox', { name: '顯示桌布' }).check();
  await form.getByRole('button', { name: '儲存連線' }).click();
  await expect(page.locator('#connection-editor')).not.toBeVisible();
  params = (await stored()).params;
  expect(params['enable-wallpaper']).toBe('true');
  expect(params['read-only']).toBe('true');
  for (const gone of ['font-size', 'color-scheme', 'scrollback', 'terminal-type', 'host-key']) expect(params[gone]).toBeUndefined();
});

test('ui: an invalid advanced value is explained next to its field, even inside a collapsed section', async ({ page }) => {
  await login(page);
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  const form = page.locator('#connection-form');
  await form.getByLabel('連線名稱').fill('bad option');
  await form.getByLabel('主機名稱或 IP').fill('example.invalid');
  await section(form, '連線控制').click();
  await form.getByLabel('連線逾時（秒）').fill('999');
  await section(form, '連線控制').click(); // collapse again
  await form.getByRole('button', { name: '儲存連線' }).click();
  await expect(form.getByText('連線逾時（秒）須為 1 到 120 的整數，或留空使用預設值。')).toBeVisible();
  await expect(form.getByLabel('連線逾時（秒）')).toHaveAttribute('aria-invalid', 'true');

  // the server enforces the same rule
  const response = await page.request.post('api/connections', {
    headers: { Origin: origin(page) },
    data: { name: 'bad', protocol: 'ssh', params: { hostname: 'h', timeout: '999' } },
  });
  expect(response.status()).toBe(400);
  expect((await response.json()).field).toBe('timeout');
});

test('ui: collapsible menus, maximized stage, dense option summaries and live status work together', async ({ page }) => {
  const name = `StageMax ${Date.now()}`;
  await login(page);

  // 1. Dense settings show configured counts on collapsed summaries and can be collapsed in one click
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  const form = page.locator('#connection-form');
  await form.getByLabel('連線名稱').fill(name);
  await form.getByLabel('主機名稱或 IP').fill('no-such-host.invalid');
  await expect(section(form, '終端機外觀與輸入')).toContainText('預設');
  await section(form, '終端機外觀與輸入').click();
  await form.getByLabel('字型大小（pt）').fill('16');
  await expect(section(form, '終端機外觀與輸入')).toContainText('已設定 1 項');
  await expect(form.locator('#options-summary')).toHaveText('進階設定（已自訂 1 項）');
  await form.getByRole('button', { name: '全部收合' }).click();
  await expect(form.getByLabel('字型大小（pt）')).toBeHidden();

  // 2. Save & connect launches the session immediately and collapses the drawer so the stage gets full width
  await form.getByRole('button', { name: '儲存並連線' }).click();
  await expect(page.locator('#connection-editor')).not.toBeVisible();
  const library = page.getByRole('complementary', { name: '連線庫' });
  await expect(library).toBeHidden();
  await expect(page.locator('#terminal-status')).toHaveText('已連線');
  const terminalBox = await page.locator('#terminal').boundingBox();
  expect(terminalBox.width).toBe(1280);

  // 3. No top title bar: terminal starts at y=0, and collapsing chrome gives the full 1280x900 viewport
  expect(terminalBox.y).toBe(0);
  await page.getByRole('button', { name: '最大畫面' }).click();
  await expect(page.locator('#statusbar')).toBeHidden();
  await expect(page.locator('#chrome-pill')).toBeVisible();
  await expect(page.locator('#chrome-pill')).toContainText(name);
  await expect(page.locator('#chrome-pill')).toContainText('已連線');
  const maxBox = await page.locator('#terminal').boundingBox();
  expect(maxBox.width).toBe(1280);
  expect(maxBox.height).toBe(900);
  await page.getByRole('button', { name: '展開選單' }).click();
  await expect(page.locator('#statusbar')).toBeVisible();

  // 4. Opening the drawer during a session overlays it without shrinking the terminal width
  await page.getByRole('button', { name: '開啟連線庫' }).click();
  await expect(library).toBeVisible();
  expect((await page.locator('#terminal').boundingBox()).width).toBe(1280);
  await expect(library).toContainText('已連線');
  await library.getByRole('button', { name: '收合選單' }).click();
  await expect(library).toBeHidden();
  await page.getByRole('button', { name: '中斷連線' }).click();
});

for (const scheme of ['light', 'dark']) {
  test(`ui: no serious accessibility violations (${scheme})`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: scheme });
    await page.goto('./');
    await expect(page.locator('#login-form')).toBeVisible();
    await expectAccessible(page, `login (${scheme})`);

    await login(page);
    const name = `A11y ${Date.now()}`;
    await addConnection(page, name, 'ssh', 'no-such-host.invalid');
    await expectAccessible(page, `workspace (${scheme})`);

    for (const protocol of ['ssh', 'telnet', 'rdp']) {
      await page.getByRole('button', { name: '新增連線', exact: true }).click();
      await page.locator('#connection-form').getByLabel('協定', { exact: true }).selectOption(protocol);
      await expectAccessible(page, `${protocol} editor (${scheme})`);
      await page.keyboard.press('Escape');
    }

    await page.getByRole('button', { name: `刪除 ${name}`, exact: true }).click();
    await expectAccessible(page, `confirm dialog (${scheme})`);
    await page.getByRole('button', { name: '取消' }).click();

    await page.getByRole('button', { name: `連線 ${name}`, exact: true }).click();
    await expect(page.locator('#overlay')).toContainText('找不到主機', { timeout: 30000 });
    await expectAccessible(page, `connection failure overlay (${scheme})`);
  });
}

test.describe('mobile', () => {
  test.use({ viewport: { width: 390, height: 800 }, hasTouch: true, isMobile: true });
  test.skip(({ browserName }) => browserName === 'firefox', 'Firefox has no mobile emulation');

  test('mobile: nothing scrolls sideways and touch targets are at least 44px', async ({ page }) => {
    await page.goto('./');
    const overflow = () => page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
    expect(await overflow()).toBeLessThanOrEqual(0);
    await login(page);
    expect(await overflow()).toBeLessThanOrEqual(0);
    const add = await page.getByRole('button', { name: '新增連線', exact: true }).boundingBox();
    expect(add.height).toBeGreaterThanOrEqual(43.5);
    await expectAccessible(page, 'mobile workspace');
    for (const [width, height] of [[768, 1024], [1024, 768]]) {
      await page.setViewportSize({ width, height });
      expect(await overflow()).toBeLessThanOrEqual(0);
      const box = await page.locator('#terminal').boundingBox();
      expect(box.y).toBe(0);
      expect(box.width).toBe(width);
    }
    await expect(page.locator('#library')).toBeVisible();
    await page.locator('#terminal').dispatchEvent('pointerdown');
    await expect(page.locator('#library')).toBeHidden();
    await page.getByRole('button', { name: '開啟連線庫' }).click();
    await expect(page.locator('#library')).toBeVisible();
  });

  test('mobile: helper keys reach the remote shell', async ({ page }) => {
    const name = `Mobile ${Date.now()}`;
    await login(page);
    const hostKey = remoteFile('/etc/ssh/ssh_host_ed25519_key.pub')
      .trim().split(/\s+/).slice(0, 2).join(' ');
    await addConnection(page, name, 'ssh', 'backend', '', 'tester-pw', { hostKey });
    await page.getByRole('button', { name: `連線 ${name}`, exact: true }).click();
    await expect(page.locator('#terminal-status')).toHaveText('已連線', { timeout: 60000 });
    const keybar = page.getByRole('toolbar', { name: '輔助按鍵' });
    await expect(keybar).toBeVisible();

    const read = async (file, act) => {
      await page.locator('#terminal').click({ position: { x: 30, y: 30 } });
      await page.keyboard.type(`read -n1 -s k; printf '%s' "$k" | od -An -tx1 > ${file}`);
      await page.keyboard.press('Enter');
      await page.waitForTimeout(500);
      await act();
    };

    const escFile = `/tmp/cracamole-mobile-esc-${Date.now()}`;
    await read(escFile, () => keybar.getByRole('button', { name: 'Escape 鍵' }).click());
    await expect.poll(() => remoteFile(escFile), { timeout: 20000 }).toContain('1b');

    // Ctrl is sticky for exactly one following key: Ctrl+A is byte 0x01.
    const ctrlFile = `/tmp/cracamole-mobile-ctrl-${Date.now()}`;
    await read(ctrlFile, async () => {
      const ctrl = keybar.getByRole('button', { name: 'Ctrl 修飾鍵（下一個按鍵生效）' });
      await ctrl.click();
      await expect(ctrl).toHaveAttribute('aria-pressed', 'true');
      await page.keyboard.press('a');
      await expect(ctrl).toHaveAttribute('aria-pressed', 'false');
    });
    await expect.poll(() => remoteFile(ctrlFile), { timeout: 20000 }).toContain('01');
  });
});
