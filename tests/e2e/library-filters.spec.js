const { test, expect } = require('@playwright/test');
const { ready, login } = require('./helpers');

test('filters and group folding work with search and keyboard', async ({ page, request }, info) => {
  await ready(request);
  await login(page);
  const group = `篩選測試 ${info.project.name} ${Date.now()}`;
  const names = [`${group} / SSH`, `${group} / RDP`, `${group} / VNC`];
  for (const [index, protocol] of ['ssh', 'rdp', 'vnc'].entries()) {
    const response = await page.request.post('api/connections', {
      headers: { Origin: new URL(page.url()).origin },
      data: { name: names[index], protocol, params: { hostname: '127.0.0.1' } },
    });
    expect(response.status()).toBe(201);
  }
  await page.reload();
  const filters = page.getByRole('group', { name: '篩選連線' });
  const ssh = page.getByRole('button', { name: `連線 ${names[0]}`, exact: true });
  const rdp = page.getByRole('button', { name: `連線 ${names[1]}`, exact: true });
  await expect(ssh).toHaveText('SSH');
  await expect(rdp).toHaveText('RDP');
  await ssh.focus();
  await page.getByRole('button', { name: `釘選 ${names[0]}` }).click();
  await expect(ssh).toHaveText(names[0]);
  await filters.getByRole('button', { name: 'RDP', exact: true }).focus();
  await page.keyboard.press('Enter');
  await expect(filters.getByRole('button', { name: 'RDP', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(rdp).toBeVisible();
  await expect(ssh).toHaveCount(0);
  await filters.getByRole('button', { name: '★ 釘選' }).click();
  await expect(ssh).toBeVisible();
  await expect(rdp).toHaveCount(0);
  await filters.getByRole('button', { name: '全部', exact: true }).click();
  await filters.getByRole('button', { name: '全部折疊' }).click();
  await expect(ssh).toBeHidden();
  await expect(rdp).toBeHidden();
  await page.reload();
  await expect(ssh).toBeHidden();
  await expect(filters.getByRole('button', { name: '全部展開' })).toBeVisible();
  const search = page.getByRole('searchbox', { name: '搜尋連線' });
  await search.fill(names[1]);
  await expect(rdp).toBeVisible();
  await expect(filters.getByRole('button', { name: '全部展開' })).toBeDisabled();
  await search.fill('');
  await expect(rdp).toBeHidden();
  await filters.getByRole('button', { name: '全部展開' }).click();
  await expect(ssh).toBeVisible();
  await expect(rdp).toBeVisible();
});

test('quick connect requires an unambiguous visible saved connection', async ({ page, request }, info) => {
  await ready(request);
  await login(page);
  const suffix = `${info.project.name}-${Date.now()}`;
  const hosts = [`duplicate-${suffix}.test`, `unique-${suffix}.test`];
  const names = [`第一筆 ${suffix}`, `第二筆 ${suffix}`, `唯一 ${suffix}`];
  for (const [index, name] of names.entries()) {
    const response = await page.request.post('api/connections', {
      headers: { Origin: new URL(page.url()).origin },
      data: { name, protocol: 'ssh', params: {
        hostname: hosts[index === 2 ? 1 : 0], username: 'alice', port: '22',
      } },
    });
    expect(response.status()).toBe(201);
  }
  await page.reload();
  let sockets = 0;
  page.on('websocket', () => { sockets += 1; });
  const search = page.getByRole('searchbox', { name: '搜尋連線' });
  const filters = page.getByRole('group', { name: '篩選連線' });
  await filters.getByRole('button', { name: '全部折疊' }).click();
  await search.press('Enter');
  expect(sockets).toBe(0);
  await search.fill(`ssh://alice@${hosts[0]}:22`);
  await expect(page.getByRole('status').filter({ hasText: '有多筆符合的連線' })).toBeVisible();
  await expect(page.getByRole('button', { name: `連線 ${names[0]}`, exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: `連線 ${names[1]}`, exact: true })).toBeVisible();
  await search.press('Enter');
  expect(sockets).toBe(0);
  await filters.getByRole('button', { name: 'RDP', exact: true }).click();
  await search.fill(`ssh://alice@${hosts[1]}:22`);
  await expect(page.getByRole('status').filter({ hasText: '符合的連線已被篩選' })).toBeVisible();
  await search.press('Enter');
  expect(sockets).toBe(0);
  await filters.getByRole('button', { name: '全部', exact: true }).click();
  await expect(page.getByRole('button', { name: `連線 ${names[2]}`, exact: true })).toBeVisible();
  await search.press('Enter');
  await expect(page.locator('#terminal-name')).toHaveText(names[2]);
});

test('folding a filtered view preserves groups outside that view', async ({ page, request }, info) => {
  await ready(request);
  await login(page, { fresh: true });
  const prefix = `群組範圍 ${info.project.name} ${Date.now()}`;
  const sshName = `${prefix} SSH / 主機`;
  const rdpName = `${prefix} RDP / 主機`;
  for (const [name, protocol] of [[sshName, 'ssh'], [rdpName, 'rdp']]) {
    const response = await page.request.post('api/connections', {
      headers: { Origin: new URL(page.url()).origin },
      data: { name, protocol, params: { hostname: '127.0.0.1' } },
    });
    expect(response.status()).toBe(201);
  }
  await page.evaluate(() => {
    localStorage.removeItem('cracamole-collapsed-groups');
    localStorage.removeItem('cracamole-collapsed-groups-v2');
  });
  await page.reload();
  const filters = page.getByRole('group', { name: '篩選連線' });
  const ssh = page.getByRole('button', { name: `連線 ${sshName}`, exact: true });
  const rdp = page.getByRole('button', { name: `連線 ${rdpName}`, exact: true });
  await filters.getByRole('button', { name: 'SSH', exact: true }).click();
  await filters.getByRole('button', { name: '全部折疊' }).click();
  await expect(ssh).toBeHidden();
  await filters.getByRole('button', { name: 'RDP', exact: true }).click();
  await expect(rdp).toBeVisible();
  await expect(filters.getByRole('button', { name: '全部折疊' })).toBeEnabled();
  await filters.getByRole('button', { name: '全部', exact: true }).click();
  await expect(ssh).toBeHidden();
  await expect(rdp).toBeVisible();
});
test('RDP quick connection distinguishes accounts in different domains', async ({ page, request }, info) => {
  await ready(request);
  await login(page, { fresh: true });
  const suffix = info.project.name + '-' + Date.now();
  const host = 'rdp-' + suffix + '.example';
  const names = ['第一個網域 ' + suffix, '第二個網域 ' + suffix];
  for (const [index, domain] of ['FIRST', 'SECOND'].entries()) {
    const response = await page.request.post('api/connections', {
      headers: { Origin: new URL(page.url()).origin },
      data: { name: names[index], protocol: 'rdp', params: { hostname: host, username: 'alice', domain } },
    });
    expect(response.status()).toBe(201);
  }
  await page.reload();
  const search = page.getByRole('searchbox', { name: '搜尋連線' });
  await search.fill('rdp://SECOND\\alice@' + host);
  await expect(page.getByRole('button', { name: '連線 ' + names[1], exact: true })).toBeVisible();
  await expect(page.getByRole('button', { name: '連線 ' + names[0], exact: true })).toHaveCount(0);
  await search.press('Enter');
  await expect(page.locator('#terminal-name')).toHaveText(names[1]);
});

test('arrow keys navigate search results from search and card actions', async ({ page, request }, info) => {
  await ready(request);
  await login(page, { fresh: true });
  const prefix = '鍵盤導覽 ' + info.project.name + ' ' + Date.now();
  const names = [prefix + ' 甲', prefix + ' 乙'];
  for (const name of names) {
    const response = await page.request.post('api/connections', {
      headers: { Origin: new URL(page.url()).origin },
      data: { name, protocol: 'ssh', params: { hostname: 'example.test' } },
    });
    expect(response.status()).toBe(201);
  }
  await page.reload();
  const search = page.getByRole('searchbox', { name: '搜尋連線' });
  await search.fill(prefix);
  const buttons = page.locator('#connections-list details[open] .card__open');
  await expect(buttons).toHaveCount(2);
  await search.press('ArrowUp');
  await expect(buttons.nth(1)).toBeFocused();
  await search.focus();
  await search.press('ArrowDown');
  await expect(buttons.nth(0)).toBeFocused();
  const firstCard = buttons.nth(0).locator('xpath=ancestor::article');
  await firstCard.locator('button[data-action="pin"]').focus();
  await page.keyboard.press('ArrowDown');
  await expect(buttons.nth(1)).toBeFocused();
});

test('empty search can reset both query and filter', async ({ page, request }) => {
  await ready(request);
  await login(page, { fresh: true });
  const response = await page.request.post('api/connections', {
    headers: { Origin: new URL(page.url()).origin },
    data: { name: '清除篩選測試', protocol: 'ssh', params: { hostname: 'example.test' } },
  });
  expect(response.status()).toBe(201);
  await page.reload();
  const filters = page.getByRole('group', { name: '篩選連線' });
  await filters.getByRole('button', { name: 'RDP', exact: true }).click();
  const search = page.getByRole('searchbox', { name: '搜尋連線' });
  await search.fill('沒有符合的唯一搜尋 ' + Date.now());
  await expect(page.getByText('沒有符合搜尋的連線。')).toBeVisible();
  await page.getByRole('button', { name: '清除搜尋與篩選' }).click();
  await expect(search).toHaveValue('');
  await expect(search).toBeFocused();
  await expect(filters.getByRole('button', { name: '全部', exact: true })).toHaveAttribute('aria-pressed', 'true');
  await expect(page.getByRole('button', { name: '清除搜尋與篩選' })).toBeHidden();
});

test('group header and actions remain available while scrolling its hosts', async ({ page, request }, info) => {
  await ready(request);
  await login(page, { fresh: true });
  const group = '吸附群組 ' + info.project.name + ' ' + Date.now();
  for (let index = 0; index < 18; index++) {
    const response = await page.request.post('api/connections', {
      headers: { Origin: new URL(page.url()).origin },
      data: { name: group + ' / 主機 ' + index, protocol: 'ssh', params: { hostname: 'example.test' } },
    });
    expect(response.status()).toBe(201);
  }
  await page.reload();
  await page.getByRole('searchbox', { name: '搜尋連線' }).fill(group);
  const drawer = page.locator('#library');
  const summary = page.getByText(group + '（18）', { exact: true });
  const add = page.getByRole('button', { name: '新增至群組 ' + group });
  await drawer.evaluate((element, groupName) => {
    const title = [...element.querySelectorAll('.conn-group__title')]
      .find(item => item.textContent.startsWith(groupName + '（'));
    element.scrollTop += title.getBoundingClientRect().top - element.getBoundingClientRect().top + 160;
  }, group);
  const [drawerBox, summaryBox, addBox] = await Promise.all([
    drawer.boundingBox(), summary.boundingBox(), add.boundingBox(),
  ]);
  const paddingTop = await drawer.evaluate(element => parseFloat(getComputedStyle(element).paddingTop));
  expect(Math.abs(summaryBox.y - drawerBox.y - paddingTop)).toBeLessThanOrEqual(2);
  expect(Math.abs(addBox.y - summaryBox.y)).toBeLessThanOrEqual(2);
  await add.click();
  await expect(page.getByRole('dialog', { name: '新增連線' }).getByLabel('所屬群組（選填）')).toHaveValue(group);
});

test('card actions remain clickable beside the next group header', async ({ page, request }, info) => {
  await ready(request);
  await login(page, { fresh: true });
  const suffix = `${info.project.name} ${Date.now()}`;
  const name = `遮擋檢查 ${suffix}`;
  for (const title of [name, `遮擋群組 ${suffix} / 主機`]) {
    const response = await page.request.post('api/connections', {
      headers: { Origin: new URL(page.url()).origin },
      data: { name: title, protocol: 'ssh', params: { hostname: 'example.test' } },
    });
    expect(response.status()).toBe(201);
  }
  await page.reload();
  await page.getByRole('button', { name: `連線 ${name}`, exact: true }).focus();
  await page.getByRole('button', { name: `編輯 ${name}`, exact: true }).click({ timeout: 5000 });
  await expect(page.getByRole('dialog', { name: '編輯連線' })).toBeVisible();
});
