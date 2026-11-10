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
