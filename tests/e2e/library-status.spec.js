const { test, expect } = require('@playwright/test');
const { ready, login } = require('./helpers');

test('connection cards show policy and retain failure until logout', async ({ page, request }, info) => {
  await ready(request);
  await login(page, { fresh: true });
  const name = `狀態測試 ${info.project.name} ${Date.now()}`;
  const response = await page.request.post('api/connections', {
    headers: { Origin: new URL(page.url()).origin },
    data: {
      name,
      protocol: 'telnet',
      params: { hostname: '127.0.0.1', port: '1', 'read-only': 'true' },
    },
  });
  expect(response.status()).toBe(201);
  const longName = name + ' 長主機名稱';
  const longResponse = await page.request.post('api/connections', {
    headers: { Origin: new URL(page.url()).origin },
    data: {
      name: longName,
      protocol: 'telnet',
      params: { hostname: 'a'.repeat(100) + '.test', 'read-only': 'true' },
    },
  });
  expect(longResponse.status()).toBe(201);
  await page.reload();
  const card = page.getByRole('article').filter({
    has: page.getByRole('button', { name: `連線 ${name}`, exact: true }),
  });
  await expect(card).toContainText('唯讀');
  const longCard = page.getByRole('article').filter({
    has: page.getByRole('button', { name: `連線 ${longName}`, exact: true }),
  });
  const cardBox = await longCard.boundingBox();
  const policyBox = await longCard.getByText('唯讀', { exact: true }).boundingBox();
  expect(policyBox.x + policyBox.width).toBeLessThanOrEqual(cardBox.x + cardBox.width);
  await expect(card).toContainText('未連線');
  await card.hover();
  await expect(card.locator('.card__status')).toBeVisible();
  await card.getByRole('button', { name: `複製 ${name}` }).focus();
  await expect(card.locator('.card__status')).toBeVisible();
  await card.getByRole('button', { name: `連線 ${name}`, exact: true }).click();
  await expect(page.getByRole('alert')).toBeVisible();
  const failure = await page.locator('#terminal-status').textContent();
  expect(failure).not.toBe('未連線');
  await page.getByRole('button', { name: '回到連線庫' }).click();
  await expect(card).toContainText(failure);
  await page.getByRole('button', { name: '登出', exact: true }).click();
  await expect(page.getByRole('heading', { name: '登入' })).toBeVisible();
  const loginForm = page.locator('#login-form');
  await loginForm.getByLabel('管理員帳號').fill('admin');
  await loginForm.getByLabel('密碼', { exact: true }).fill('fixture-admin-password-2026');
  await loginForm.getByRole('button', { name: '登入', exact: true }).click();
  await expect(card).toContainText('未連線');
  await expect(card).toContainText('唯讀');
});

test('updating connection status preserves keyboard focus on the same card', async ({ page, request }, info) => {
  await ready(request);
  await login(page);
  const name = `同名焦點 ${info.project.name} ${Date.now()}`;
  const ids = [];
  for (const host of ['first.example', 'second.example']) {
    const response = await page.request.post('api/connections', {
      headers: { Origin: new URL(page.url()).origin },
      data: { name, protocol: 'telnet', params: { hostname: host } },
    });
    expect(response.status()).toBe(201);
    ids.push((await response.json()).id);
  }
  await page.reload();
  const card = page.getByRole('article').filter({ hasText: 'second.example' });
  const open = card.getByRole('button', { name: `連線 ${name}`, exact: true });
  await open.focus();
  await page.evaluate(async id => {
    const library = await import('/assets/js/views/library.js');
    library.setActive(id, 'connecting', '正在連線…');
  }, ids[0]);
  await expect(open).toBeFocused();
  await card.getByRole('button', { name: `釘選 ${name}` }).click();
  await expect(card.getByRole('button', { name: `取消釘選 ${name}` })).toBeFocused();
});

test('updating connection status preserves group control focus', async ({ page, request }, info) => {
  await ready(request);
  await login(page);
  const group = `群組焦點 ${info.project.name} ${Date.now()}`;
  const response = await page.request.post('api/connections', {
    headers: { Origin: new URL(page.url()).origin },
    data: { name: `${group} / 主機`, protocol: 'telnet', params: { hostname: '127.0.0.1' } },
  });
  expect(response.status()).toBe(201);
  const id = (await response.json()).id;
  await page.reload();
  const summary = page.getByText(`${group}（1）`, { exact: true });
  await summary.focus();
  await page.evaluate(async connectionId => {
    const library = await import('/assets/js/views/library.js');
    library.setActive(connectionId, 'connecting', '正在連線…');
  }, id);
  await expect(summary).toBeFocused();
  const add = page.getByRole('button', { name: `新增至群組 ${group}` });
  await add.focus();
  await page.evaluate(async connectionId => {
    const library = await import('/assets/js/views/library.js');
    library.setActive(connectionId, 'connected', '已連線');
  }, id);
  await expect(add).toBeFocused();
});

test('recent and connected filters follow session state and search', async ({ page, request }, info) => {
  await ready(request);
  await login(page, { fresh: true });
  const prefix = `快速篩選 ${info.project.name} ${Date.now()}`;
  const ids = [];
  for (const suffix of ['最近', '未使用']) {
    const response = await page.request.post('api/connections', {
      headers: { Origin: new URL(page.url()).origin },
      data: { name: `${prefix} ${suffix}`, protocol: 'telnet', params: { hostname: '127.0.0.1' } },
    });
    expect(response.status()).toBe(201);
    ids.push((await response.json()).id);
  }
  await page.reload();
  await page.getByLabel('搜尋連線').fill(prefix);
  await page.evaluate(async id => {
    const library = await import('/assets/js/views/library.js');
    library.setActive(id, 'connecting', '正在連線…');
    library.setActive(id, 'connected', '已連線');
  }, ids[0]);
  const filters = page.getByRole('group', { name: '篩選連線' });
  const recent = filters.getByRole('button', { name: '最近', exact: true });
  const connected = filters.getByRole('button', { name: '已連線', exact: true });
  await connected.click();
  await expect(connected).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: `連線 ${prefix} 最近`, exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: `連線 ${prefix} 未使用` })).toHaveCount(0);
  await page.evaluate(async id => {
    const library = await import('/assets/js/views/library.js');
    library.setActive(id, 'closed', '連線已結束');
  }, ids[0]);
  await expect(page.getByRole('button', { name: `連線 ${prefix} 最近`, exact: true })).toHaveCount(0);
  await recent.click();
  await expect(recent).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: `連線 ${prefix} 最近`, exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: `連線 ${prefix} 未使用` })).toHaveCount(0);
  await page.reload();
  await page.getByLabel('搜尋連線').fill(prefix);
  await page.getByRole('group', { name: '篩選連線' }).getByRole('button', { name: '最近', exact: true }).click();
  await expect(page.getByRole('button', { name: `連線 ${prefix} 最近`, exact: true })).toBeVisible();
});
