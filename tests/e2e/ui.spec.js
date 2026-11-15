// Interface behaviour (see web/GUIDELINES.md §9). Tests prefixed "ui:" need no remote host;
// tests prefixed "mobile:" that connect need the sshd fixture (scripts/e2e_remote.sh).
const { test, expect } = require('@playwright/test');
const AxeBuilder = require('@axe-core/playwright').default;
const { remoteFile, ready, login, addConnection, selectProtocol, protocolLabel } = require('./helpers');

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

test('ui: logs dialog is accessible and returns focus when dismissed', async ({ page }) => {
  await login(page);
  const button = page.getByRole('button', { name: '系統日誌' });
  await button.click();
  const dialog = page.getByRole('dialog', { name: '系統日誌' });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('button', { name: '關閉', exact: true }).first()).toBeFocused();
  await expectAccessible(page, '系統日誌對話框');
  await page.evaluate(() => document.documentElement.setAttribute('data-theme', 'light'));
  await expectAccessible(page, '淺色系統日誌對話框');
  await page.keyboard.press('Escape');
  await expect(dialog).toBeHidden();
  await expect(button).toBeFocused();
  await button.click();
  await dialog.getByRole('button', { name: '關閉', exact: true }).last().click();
  await expect(dialog).toBeHidden();
  await expect(button).toBeFocused();
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
  await expect(form.getByRole('radio', { name: 'RDP', exact: true })).toBeChecked();
  await expect(form.getByLabel('網域（可選）')).toBeVisible();
  await expect(form.getByLabel('主機金鑰（known_hosts）')).toBeHidden();
  await expect(form.getByLabel('連接埠')).toHaveAttribute('placeholder', '3389');
});

test('ui: pinned hosts and collapsed groups persist, while search expands matching groups', async ({ page }) => {
  await login(page);
  const group = `測試群組-${Date.now()}`;
  const name = `${group} / server-01`;
  await addConnection(page, name, 'ssh', 'example.invalid');
  const row = page.getByRole('article').filter({ has: page.getByRole('button', { name: `連線 ${name}`, exact: true }) });
  await row.scrollIntoViewIfNeeded();
  const box = await row.boundingBox();
  expect(box.height).toBeGreaterThanOrEqual(44);
  expect(box.height).toBeLessThanOrEqual(50);
  await page.mouse.move(box.x + 4, box.y + 4);
  await expect.poll(() => page.getByRole('button', { name: `釘選 ${name}`, exact: true })
    .evaluate(button => getComputedStyle(button.parentElement).opacity)).toBe('1');
  await page.getByRole('button', { name: `連線 ${name}`, exact: true }).focus();
  await page.getByRole('button', { name: `釘選 ${name}`, exact: true }).click();
  await expect(page.getByText('★ 常用釘選（1）')).toBeVisible();
  await page.reload();
  await page.getByRole('button', { name: `連線 ${name}`, exact: true }).focus();
  await expect(page.getByRole('button', { name: `取消釘選 ${name}`, exact: true })).toBeVisible();
  await page.getByRole('button', { name: `取消釘選 ${name}`, exact: true }).click();
  await page.getByText(`${group}（1）`).click();
  await expect(page.getByRole('button', { name: `連線 ${name}`, exact: true })).toBeHidden();
  await page.reload();
  await expect(page.getByRole('button', { name: `連線 ${name}`, exact: true })).toBeHidden();
  await page.getByRole('searchbox', { name: '搜尋連線' }).fill(name);
  await expect(page.getByRole('button', { name: `連線 ${name}`, exact: true })).toBeVisible();
  await page.getByRole('searchbox', { name: '搜尋連線' }).press('ArrowDown');
  await expect(page.getByRole('button', { name: `連線 ${name}`, exact: true })).toBeFocused();
  await page.keyboard.press('Control+k');
  await expect(page.getByRole('searchbox', { name: '搜尋連線' })).toBeFocused();
});

test('ui: add a host from a collapsed group without changing its expansion state', async ({ page }) => {
  await login(page);
  const group = `群組新增-${Date.now()}`;
  await addConnection(page, `${group} / node-01`, 'ssh', 'example.invalid');
  const header = page.getByText(`${group}（1）`, { exact: true });
  await header.click();
  const details = header.locator('..');
  await expect(details).not.toHaveAttribute('open', '');
  await page.getByRole('button', { name: `新增至群組 ${group}` }).click();
  const editor = page.getByRole('dialog', { name: '新增連線' });
  await expect(editor.getByLabel('所屬群組（選填）')).toHaveValue(group);
  await expect(editor.getByLabel('連線名稱')).toHaveValue('');
  await expect(editor.getByLabel('連線名稱')).toBeFocused();
  await editor.getByLabel('連線名稱').fill('node-02');
  await editor.getByLabel('主機名稱或 IP').fill('other.invalid');
  await editor.getByRole('button', { name: '儲存連線', exact: true }).click();
  await expect(editor).toBeHidden();
  await expect(page.getByText(`${group}（2）`, { exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: `連線 ${group} / node-02` })).toBeHidden();
});

test('ui: group field edits the compatible name prefix', async ({ page }) => {
  await login(page);
  const original = `舊群組 ${Date.now()} / 主機`;
  await addConnection(page, original, 'ssh', 'example.invalid');
  await page.getByRole('button', { name: `連線 ${original}`, exact: true }).focus();
  await page.getByRole('button', { name: `編輯 ${original}`, exact: true }).click();
  const editor = page.getByRole('dialog', { name: '編輯連線' });
  await expect(editor.getByLabel('連線名稱')).toHaveValue('主機');
  const group = editor.getByLabel('所屬群組（選填）');
  await expect(group).toHaveValue(original.split(' / ')[0]);
  await expect(editor.locator(`#editor-group-options option[value=${JSON.stringify(original.split(' / ')[0])}]`)).toHaveCount(1);
  await group.fill('新群組');
  const request = page.waitForRequest(req => req.method() === 'PUT' && req.url().includes('/api/connections/'));
  await editor.getByRole('button', { name: '儲存連線', exact: true }).click();
  expect((await request).postDataJSON().name).toBe('新群組 / 主機');
  await expect(editor).toBeHidden();
});

test('ui: move a group to ungrouped without deleting its connections', async ({ page }) => {
  await login(page);
  const group = `待移動 ${Date.now()}`;
  const members = [`${group} 甲`, `${group} 乙`];
  const names = members.map(member => `${group} / ${member}`);
  for (const name of names) await addConnection(page, name, 'ssh', 'example.invalid');
  await page.getByRole('button', { name: `重新命名群組 ${group}` }).click();
  const dialog = page.getByRole('dialog', { name: '重新命名群組' });
  const request = page.waitForResponse(response => response.url().endsWith('/api/connections/rename-group'));
  await dialog.getByRole('button', { name: '移至未分類' }).click();
  expect((await request).status()).toBe(200);
  await expect(dialog).toBeHidden();
  for (const member of members) await expect(page.getByRole('button', { name: `連線 ${member}`, exact: true })).toBeVisible();
});

test('ui: merging groups preserves both connection identities and settings', async ({ page }) => {
  await login(page);
  const tag = String(Date.now());
  const source = `來源 ${tag}`;
  const target = `目標 ${tag}`;
  const ids = [];
  for (const [group, host] of [[source, 'source.example'], [target, 'target.example']]) {
    const response = await page.request.post('api/connections', {
      headers: { Origin: origin(page) },
      data: { name: `${group} / 主機`, protocol: 'ssh', params: { hostname: host, username: 'alice', password: 'fixture-merge-secret' } },
    });
    expect(response.status()).toBe(201);
    ids.push((await response.json()).id);
  }
  const before = await Promise.all(ids.map(async id => (await page.request.get(`api/connections/${id}`)).json()));
  await page.reload();
  await page.getByRole('button', { name: `重新命名群組 ${source}` }).click();
  const dialog = page.getByRole('dialog', { name: '重新命名群組' });
  await dialog.getByLabel('新群組名稱').fill(target);
  const response = page.waitForResponse(item => item.url().endsWith('/api/connections/rename-group'));
  await dialog.getByRole('button', { name: '重新命名', exact: true }).click();
  expect((await response).status()).toBe(200);
  await expect(dialog).toBeHidden();
  await expect(page.getByText(`${target}（2）`, { exact: true })).toBeVisible();
  const after = await Promise.all(ids.map(async id => (await page.request.get(`api/connections/${id}`)).json()));
  expect(after.map(item => item.id)).toEqual(ids);
  expect(after.map(item => item.name)).toEqual([`${target} / 主機`, `${target} / 主機`]);
  expect(after.map(item => item.params)).toEqual(before.map(item => item.params));
  expect(after.every(item => !('password' in item.params))).toBe(true);
});

test('ui: reserved group names remain separate and legacy collapsed state is migrated', async ({ page }) => {
  await login(page);
  const tag = String(Date.now());
  const names = [`plain-${tag}`, `未分類 / named-${tag}`, `★ 常用釘選 / named-${tag}`, `pinned-${tag}`];
  const ids = [];
  for (const name of names) {
    const response = await page.request.post('api/connections', {
      headers: { Origin: origin(page) },
      data: { name, protocol: 'ssh', params: { hostname: 'example.invalid' } },
    });
    expect(response.status()).toBe(201);
    ids.push((await response.json()).id);
  }
  await page.evaluate(id => {
    localStorage.setItem('cracamole-pinned', JSON.stringify([id]));
    localStorage.setItem('cracamole-collapsed-groups', JSON.stringify(['未分類', '★ 常用釘選']));
    localStorage.removeItem('cracamole-collapsed-groups-v2');
  }, ids[3]);
  await page.reload();
  const summary = text => page.locator('summary.conn-group__title').filter({ hasText: text });
  const named = summary('未分類（1）');
  const ungrouped = summary('未分類（無群組）');
  const pinned = summary('★ 常用釘選（1）').filter({ hasNotText: '群組：' });
  const explicitPinned = summary('群組：★ 常用釘選（1）');
  for (const group of [named, ungrouped, pinned, explicitPinned]) {
    await expect(group).toBeVisible();
    await expect(group.locator('..')).not.toHaveAttribute('open', '');
  }
  await named.click();
  await expect(page.getByRole('button', { name: `連線 ${names[1]}`, exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: `連線 ${names[0]}`, exact: true })).toBeHidden();
  await page.reload();
  await expect(page.getByRole('button', { name: `連線 ${names[1]}`, exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: `連線 ${names[0]}`, exact: true })).toBeHidden();
  await page.getByRole('button', { name: '新增至群組 未分類', exact: true }).click();
  const editor = page.getByRole('dialog', { name: '新增連線' });
  await expect(editor.getByLabel('所屬群組（選填）')).toHaveValue('未分類');
  await expect(editor.getByLabel('連線名稱')).toHaveValue('');
  await editor.getByRole('button', { name: '取消' }).click();
  for (const id of ids) {
    await page.request.delete(`api/connections/${id}`, { headers: { Origin: origin(page) } });
  }
});

test('ui: quick address opens a prefilled editor and rejects passwords and invalid ports', async ({ page }) => {
  await login(page);
  const search = page.getByRole('searchbox', { name: '搜尋連線' });
  for (const [address, protocol, user, host, port] of [
    ['alice@server.example:2222', 'ssh', 'alice', 'server.example', '2222'],
    ['rdp://admin@[2001:db8::1]:3389', 'rdp', 'admin', '2001:db8::1', '3389'],
  ]) {
    await search.fill(address);
    await expect(page.getByRole('button', { name: /快速設定/ })).toBeVisible();
    await search.press('Enter');
    const editor = page.getByRole('dialog', { name: '新增連線' });
    await expect(editor.getByRole('radio', { name: protocolLabel(protocol), exact: true })).toBeChecked();
    await expect(editor.getByLabel('帳號', { exact: true })).toHaveValue(user);
    await expect(editor.getByLabel('主機名稱或 IP')).toHaveValue(host);
    await expect(editor.getByLabel('連接埠')).toHaveValue(port);
    await expect(editor.getByLabel('密碼', { exact: true })).toBeFocused();
    await editor.getByRole('button', { name: '取消' }).click();
  }
  for (const invalid of ['ssh://alice:password@host', 'host:99999', 'https://host', 'host/path',
    'ssh -p 22 host;whoami', 'ssh -p 22 host whoami']) {
    await search.fill(invalid);
    await expect(page.getByRole('button', { name: /快速設定/ })).toHaveCount(0);
  }
  const existing = await page.request.post('api/connections', {
    headers: { Origin: origin(page) },
    data: { name: '既有 SSH', protocol: 'ssh', params: { hostname: 'server.example', username: 'Alice' } },
  });
  expect(existing.status()).toBe(201);
  await page.reload();
  await search.fill('ssh://Alice@server.example:22');
  await expect(page.getByRole('button', { name: /快速設定/ })).toHaveCount(0);
  await search.fill('ssh://alice@server.example:22');
  await expect(page.getByRole('button', { name: /快速設定/ })).toBeVisible();
});

test('ui: editor identity probe rejects stale results and never saves trust automatically', async ({ page }) => {
  await login(page);
  const name = `身分探測測試-${Date.now()}`;
  const response = await page.request.post('api/connections', {
    headers: { Origin: origin(page) },
    data: { name, protocol: 'ssh', params: { hostname: 'fixture.invalid' } },
  });
  expect(response.status()).toBe(201);
  const saved = await response.json();
  let release;
  const gate = new Promise(resolve => { release = resolve; });
  let started;
  const called = new Promise(resolve => { started = resolve; });
  await page.route('**/api/connections/*/host-key', async route => {
    started();
    await gate;
    await route.fulfill({ json: { hostname: 'fixture.invalid', port: 22,
      public_key: 'ssh-ed25519 test', fingerprint: 'SHA256:test', trusted: false, matches: false } });
  });
  await page.reload();
  await page.getByRole('button', { name: `連線 ${name}`, exact: true }).focus();
  await page.getByRole('button', { name: `編輯 ${name}` }).click();
  const dialog = page.getByRole('dialog', { name: '編輯連線' });
  const button = dialog.getByRole('button', { name: '探測主機身分' });
  await button.click();
  await called;
  await dialog.getByLabel('主機名稱或 IP').fill('other.invalid');
  release();
  await expect(dialog.getByRole('status')).toBeHidden();
  await dialog.getByLabel('主機名稱或 IP').fill('fixture.invalid');
  await expect(button).toBeEnabled();
  await button.click();
  await expect(dialog.getByRole('status')).toContainText('尚未保存信任');
  await dialog.getByLabel('主機名稱或 IP').evaluate(input => {
    const event = new Event('paste', { bubbles: true, cancelable: true });
    Object.defineProperty(event, 'clipboardData', { value: { getData: () => 'alice@other.invalid:2222' } });
    input.dispatchEvent(event);
  });
  await expect(button).toBeEnabled();
  await expect(dialog.getByRole('status')).toBeHidden();
  const unchanged = await (await page.request.get(`api/connections/${saved.id}`)).json();
  expect(unchanged.params['host-key']).toBeFalsy();
  await dialog.getByRole('button', { name: '取消' }).click();
});

test('ui: draft identity refuses mismatched SSH trust and can fill a verified RDP pin', async ({ page }) => {
  await login(page);
  await page.route('**/api/identity-probe', async route => {
    const body = route.request().postDataJSON();
    if (body.protocol === 'ssh') {
      await route.fulfill({ json: { hostname: body.hostname, port: Number(body.port), fingerprint: 'SHA256:test',
        public_key: 'ssh-ed25519 test', trusted: true, matches: false } });
    } else {
      await route.fulfill({ json: { hostname: body.hostname, port: Number(body.port), sha256: 'a'.repeat(64) } });
    }
  });
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: '新增連線' });
  await dialog.getByLabel('主機名稱或 IP').fill('host.invalid');
  await dialog.getByRole('button', { name: '探測主機身分' }).click();
  await expect(dialog.getByRole('status')).toContainText('不符或已撤銷');
  await expect(dialog.getByRole('button', { name: '已核對，填入信任設定' })).toBeHidden();
  await selectProtocol(dialog, 'rdp');
  await dialog.getByRole('button', { name: '探測主機身分' }).click();
  await expect(dialog.getByRole('status')).toContainText('尚未比對已保存信任');
  await dialog.getByRole('button', { name: '已核對，填入信任設定' }).click();
  await page.getByRole('dialog', { name: '確認主機身分' }).getByRole('button', { name: '已核對，填入設定' }).click();
  await expect(dialog.locator('[name="server-cert-sha256"]')).toHaveValue('a'.repeat(64));
  await dialog.getByRole('button', { name: '取消' }).click();
});

test('ui: host paste splits address and a blank name is generated without reading the system clipboard', async ({ page }) => {
  await login(page);
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  const form = page.getByRole('dialog', { name: '新增連線' });
  const hostname = form.getByLabel('主機名稱或 IP');
  async function paste(text) {
    await hostname.evaluate((element, value) => {
      const event = new Event('paste', { bubbles: true, cancelable: true });
      Object.defineProperty(event, 'clipboardData', { value: { getData: () => value } });
      element.dispatchEvent(event);
    }, text);
  }
  await paste('ssh -p 2222 ubuntu@10.0.1.5');
  await expect(form.getByRole('radio', { name: 'SSH', exact: true })).toBeChecked();
  await expect(hostname).toHaveValue('10.0.1.5');
  await expect(form.getByLabel('帳號', { exact: true })).toHaveValue('ubuntu');
  await expect(form.getByLabel('連接埠')).toHaveValue('2222');
  await form.getByLabel('密碼', { exact: true }).fill('未保存的 SSH 密碼');
  await form.getByLabel('私鑰（可選）').fill('未保存的私鑰');
  await form.getByLabel('私鑰密語').fill('未保存的密語');
  await paste('rdp://Alice@[2001:db8::4]:3390');
  await expect(form.getByRole('radio', { name: 'RDP', exact: true })).toBeChecked();
  await expect(form.getByLabel('密碼', { exact: true })).toHaveValue('');
  await expect(form.getByLabel('私鑰（可選）')).toHaveValue('');
  await expect(form.getByLabel('私鑰密語')).toHaveValue('');
  await form.getByLabel('密碼', { exact: true }).fill('未保存的 RDP 密碼');
  await selectProtocol(form, 'ssh');
  await expect(form.getByLabel('密碼', { exact: true })).toHaveValue('');
  await selectProtocol(form, 'rdp');
  await expect(hostname).toHaveValue('2001:db8::4');
  await expect(form.getByLabel('帳號', { exact: true })).toHaveValue('Alice');
  await expect(form.getByLabel('連接埠')).toHaveValue('3390');
  await paste('rdp://CORP\\alice@server.example:3391');
  await expect(form.getByLabel('網域')).toHaveValue('CORP');
  await expect(form.getByLabel('帳號', { exact: true })).toHaveValue('alice');
  await expect(hostname).toHaveValue('server.example');
  await paste('other\\bob@new.example:3392');
  await expect(form.getByLabel('網域')).toHaveValue('other');
  await expect(form.getByLabel('帳號', { exact: true })).toHaveValue('bob');
  await expect(hostname).toHaveValue('new.example');
  await paste('rdp://blank.example');
  await expect(form.getByLabel('網域')).toHaveValue('');
  await expect(form.getByLabel('帳號', { exact: true })).toHaveValue('');
  await expect(form.getByLabel('連接埠')).toHaveValue('');
  await paste('rdp://alice%3Asecret@other.example');
  await expect(hostname).toHaveValue('blank.example');
  await expect(form.getByText('連線位址不可包含密碼，請在密碼欄位輸入。')).toBeVisible();
  await paste('rdp://Alice@[2001:db8::4]:3390');
  await expect(form.getByLabel('網域')).toHaveValue('');
  await paste('rdp://Alice:secret@other.example');
  await expect(hostname).toHaveValue('2001:db8::4');
  await expect(form.getByText('連線位址不可包含密碼，請在密碼欄位輸入。')).toBeVisible();
  const request = page.waitForRequest(req => req.method() === 'POST' && req.url().endsWith('/api/connections'));
  await form.getByRole('button', { name: '儲存連線', exact: true }).click();
  expect((await request).postDataJSON().name).toBe('Alice@2001:db8::4');
  await expect(form).toBeHidden();
});

test('ui: duplicating a host omits secrets and drops trust only when its destination changes', async ({ page }) => {
  await login(page);
  const name = `clone-${Date.now()}-01`;
  const response = await page.request.post('api/connections', {
    headers: { Origin: origin(page) },
    data: {
      name, protocol: 'ssh',
      params: { hostname: 'server.example', port: '22', username: 'alice', password: 'fixture-secret',
        'host-key': 'ssh-ed25519 fixture-key' },
    },
  });
  expect(response.status()).toBe(201);
  await page.reload();
  await page.getByRole('button', { name: `連線 ${name}`, exact: true }).focus();
  await page.getByRole('button', { name: `編輯 ${name}`, exact: true }).click();
  await page.getByRole('dialog', { name: '編輯連線' })
    .getByRole('button', { name: '複製為新連線' }).click();
  const fromEditor = page.getByRole('dialog', { name: '複製連線' });
  await expect(fromEditor.getByLabel('主機名稱或 IP')).toBeFocused();
  await fromEditor.getByRole('button', { name: '取消' }).click();
  await page.getByRole('button', { name: `連線 ${name}`, exact: true }).focus();
  await page.keyboard.press('Control+d');
  await expect(page.getByRole('dialog', { name: '複製連線' })).toBeVisible();
  await page.getByRole('dialog', { name: '複製連線' }).getByRole('button', { name: '取消' }).click();
  await page.getByRole('button', { name: `釘選 ${name}`, exact: true }).focus();
  await page.keyboard.press('Alt+d');
  await expect(page.getByRole('dialog', { name: '複製連線' })).toBeVisible();
  await page.getByRole('dialog', { name: '複製連線' }).getByRole('button', { name: '取消' }).click();
  const intercepted = await page.getByRole('button', { name: `釘選 ${name}`, exact: true }).evaluate(button => {
    const event = new KeyboardEvent('keydown', { key: 'd', ctrlKey: true, altKey: true, bubbles: true, cancelable: true });
    button.dispatchEvent(event);
    return event.defaultPrevented;
  });
  expect(intercepted).toBe(false);
  await expect(page.getByRole('dialog', { name: '複製連線' })).toBeHidden();
  for (const changed of [false, true]) {
    await page.getByRole('button', { name: `連線 ${name}`, exact: true }).focus();
    await page.getByRole('button', { name: `複製 ${name}`, exact: true }).click();
    const dialog = page.getByRole('dialog', { name: '複製連線' });
    await expect(dialog.getByLabel('連線名稱')).toHaveValue(name.replace(/01$/, '02'));
    const host = dialog.getByLabel('主機名稱或 IP');
    await expect(host).toBeFocused();
    await expect(dialog.getByLabel('密碼', { exact: true })).toHaveValue('');
    if (changed) await host.fill('new.example');
    const request = page.waitForRequest(req => req.method() === 'POST' && req.url().endsWith('/api/connections'));
    await dialog.getByRole('button', { name: '儲存連線', exact: true }).click();
    const body = (await request).postDataJSON();
    expect(body.params.password).toBeUndefined();
    expect(body.params['private-key']).toBeUndefined();
    if (changed) expect(body.params['host-key']).toBeUndefined();
    else expect(body.params['host-key']).toBe('ssh-ed25519 fixture-key');
    await expect(dialog).toBeHidden();
  }
});

test('ui: the editor shows only the fields of the selected protocol and explains mistakes in place', async ({ page }) => {
  await login(page);
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  const form = page.locator('#connection-form');
  const protocol = { selectOption: value => selectProtocol(form, value) };

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

test('ui: protocol segments support pointer, keyboard and forced colours', async ({ page }) => {
  await page.emulateMedia({ forcedColors: 'active' });
  await login(page);
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  const dialog = page.getByRole('dialog', { name: '新增連線' });
  await dialog.getByText('RDP', { exact: true }).click();
  const selected = dialog.getByRole('radio', { name: 'RDP', exact: true });
  await expect(selected).toBeChecked();
  expect(await selected.evaluate(input => getComputedStyle(input.nextElementSibling).outlineStyle)).toBe('solid');
  await selectProtocol(dialog, 'ssh');
  await expect(dialog.getByRole('radio', { name: 'SSH', exact: true })).toBeChecked();
  await expect(dialog.getByLabel('主機金鑰（known_hosts）')).toBeVisible();
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
  await card.focus();

  await page.getByRole('button', { name: `刪除 ${name}`, exact: true }).click();
  const dialog = page.getByRole('dialog', { name: `刪除「${name}」？` });
  await expect(dialog).toBeVisible();
  await expect(dialog.getByRole('button', { name: '取消' })).toBeFocused();
  await dialog.getByRole('button', { name: '取消' }).click();
  await expect(dialog).toBeHidden();
  await expect(card).toBeVisible();

  await card.focus();
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
  await expect(page.locator('#terminal-status')).toHaveText('找不到主機');
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
  const protocol = { selectOption: value => selectProtocol(form, value) };
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
  await page.getByRole('button', { name: `連線 ${name}`, exact: true }).focus();
  await page.getByRole('button', { name: `編輯 ${name}`, exact: true }).click();
  await expect(form.getByLabel('字型大小（pt）')).toHaveValue('16');
  await expect(form.getByLabel('配色')).toHaveValue('green-black');
  await expect(form.getByRole('checkbox', { name: /唯讀/ })).toBeChecked();

  // switching to RDP removes the terminal-only settings and keeps the shared ones
  await selectProtocol(form, 'rdp');
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
  // Browser Back during an active session is covered with a real SSH fixture in local-ssh.spec.js.
  // This intentionally unreachable host may already have disconnected at this point.
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
      await selectProtocol(page.locator('#connection-form'), protocol);
      await expectAccessible(page, `${protocol} editor (${scheme})`);
      await page.keyboard.press('Escape');
    }

    await page.getByRole('button', { name: `連線 ${name}`, exact: true }).focus();
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
