const { test, expect } = require('@playwright/test');
const { execFileSync } = require('node:child_process');
const path = require('node:path');
const PASSWORD = 'fixture-admin-password-2026';

function compose(args) {
  const remote = process.env.CRACAMOLE_E2E_SSH;
  const directory = process.env.CRACAMOLE_E2E_REMOTE_DIR;
  if (remote && !directory) throw new Error('CRACAMOLE_E2E_REMOTE_DIR is required');
  const file = remote ? `${directory}/docker/compose.e2e.yml` : path.resolve(__dirname, '../../docker/compose.e2e.yml');
  const command = ['compose', '-p', 'cracamole-e2e', '-f', file, ...args];
  const quote = value => "'" + value.replaceAll("'", "'\\''") + "'";
  return execFileSync(remote ? 'ssh' : 'docker', remote
    ? ['-o', 'BatchMode=yes', '-o', 'ConnectTimeout=5', remote, ['docker', ...command].map(quote).join(' ')]
    : command, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: 30000 });
}
function remoteFile(file) { try { return compose(['exec', '-T', 'backend', 'cat', file]); } catch { return ''; } }
async function ready(request) {
  await expect.poll(async () => { try { return (await request.get('health', { timeout: 3000 })).status(); } catch { return 0; } }, { timeout: 90000 }).toBe(200);
}
async function login(page) {
  await page.goto('./');
  await page.locator('#login-form [name=username]').fill('admin');
  await page.locator('#login-form [name=password]').fill(PASSWORD);
  await page.getByRole('button', { name: '登入', exact: true }).click();
  await expect(page.locator('#workspace')).toBeVisible();
}
async function visibleTerminalInk(page) {
  const screenshot = await page.locator('#terminal').screenshot();
  return page.evaluate(async encoded => {
    const image = new Image();
    image.src = 'data:image/png;base64,' + encoded;
    await image.decode();
    const canvas = document.createElement('canvas');
    canvas.width = image.width; canvas.height = image.height;
    const context = canvas.getContext('2d'); context.drawImage(image, 0, 0);
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
    let ink = 0;
    for (let i = 0; i < pixels.length; i += 4)
      if (pixels[i + 3] && pixels[i] + pixels[i + 1] + pixels[i + 2] > 100) ink++;
    return ink;
  }, screenshot.toString('base64'));
}
async function addConnection(page, name, protocol, host='backend') {
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  const form = page.locator('#connection-form');
  await form.locator('[name=name]').fill(name);
  await form.locator('[name=protocol]').selectOption(protocol);
  await form.locator('[name=hostname]').fill(host);
  await form.locator('[name=username]').fill('tester');
  await form.locator('[name=password]').fill('tester-pw');
  if (protocol === 'telnet') {
    await form.locator('summary').click();
    await form.locator('[name=login-success-regex]').fill(':~[$] ');
    await form.locator('[name=login-failure-regex]').fill('Login incorrect');
  }
  await form.getByRole('button', { name: '儲存連線' }).click();
  await expect(page.locator('#connection-editor')).not.toBeVisible();
  await expect(page.getByRole('button', { name: `連線 ${name}`, exact: true })).toBeVisible();
}

test.beforeAll(async ({ request }) => ready(request));

for (const protocol of ['ssh', 'telnet']) {
  test(`${protocol}: Java-free login, connection management, vim, resize, clipboard, CJK and idle`, async ({ page }, info) => {
    const name = `${protocol.toUpperCase()} ${Date.now()}`;
    await login(page);
    await addConnection(page, name, protocol);
    const listing = await page.request.get('api/connections');
    const profile = (await listing.json()).find(c => c.name === name);
    expect(profile.params.password).toBeUndefined();
    expect(profile.configured_secrets).toContain('password');
    await page.getByRole('button', { name: `連線 ${name}`, exact: true }).click();
    await expect(page.locator('#terminal-status')).toHaveText('已連線');
    await expect.poll(() => visibleTerminalInk(page)).toBeGreaterThan(100);
    const terminal = page.locator('#terminal');
    await terminal.click({ position: { x: 30, y: 30 } });
    const prefix = `/tmp/cracamole-${protocol}-${Date.now()}`;
    const marker = `${protocol}-${Date.now()}`;
    await page.keyboard.type(`printf '${marker}' > ${prefix}-keyboard`);
    await page.keyboard.press('Enter');
    await expect.poll(() => remoteFile(`${prefix}-keyboard`)).toBe(marker);
    if (protocol === 'ssh') {
      await page.waitForTimeout(17000);
      await expect(page.locator('#terminal-status')).toHaveText('已連線');
      await page.keyboard.type(`printf '${marker}' > ${prefix}-idle`);
      await page.keyboard.press('Enter');
      await expect.poll(() => remoteFile(`${prefix}-idle`)).toBe(marker);
    }
    await page.keyboard.type(`vim -Nu NONE -n ${prefix}-vim`);
    await page.keyboard.press('Enter');
    await page.waitForTimeout(1000);
    await page.keyboard.type('i' + marker);
    await page.keyboard.press('Escape');
    await page.keyboard.type(':wq');
    await page.keyboard.press('Enter');
    await expect.poll(() => remoteFile(`${prefix}-vim`)).toBe(`${marker}\n`);
    await page.keyboard.type(`stty size > ${prefix}-size-before`);
    await page.keyboard.press('Enter');
    await expect.poll(() => remoteFile(`${prefix}-size-before`)).toMatch(/^\d+ \d+\r?\n$/);
    const oldSize = remoteFile(`${prefix}-size-before`).trim();
    await page.keyboard.type(`while [ "$(stty size)" = "${oldSize}" ]; do sleep 0.1; done; stty size > ${prefix}-size-after`);
    await page.keyboard.press('Enter');
    await page.setViewportSize({ width: 1000, height: 700 });
    await expect.poll(() => remoteFile(`${prefix}-size-after`)).toMatch(/^\d+ \d+\r?\n$/);
    expect(remoteFile(`${prefix}-size-after`).trim()).not.toBe(oldSize);
    await page.getByRole('button', { name: '剪貼簿', exact: true }).click();
    await page.locator('#clipboard-text').fill(`printf '${marker}' > ${prefix}-clipboard\n`);
    await page.getByRole('button', { name: '貼上到終端機' }).click();
    await expect.poll(() => remoteFile(`${prefix}-clipboard`)).toBe(marker);
    await page.getByRole('button', { name: '剪貼簿', exact: true }).click();
    await terminal.click({ position: { x: 30, y: 30 } });
    await page.keyboard.type(`printf '\\344\\270\\255\\346\\226\\207' | tee ${prefix}-cjk`);
    await page.keyboard.press('Enter');
    await expect.poll(() => remoteFile(`${prefix}-cjk`)).toBe('中文');
    await expect.poll(() => visibleTerminalInk(page)).toBeGreaterThan(100);
    await page.waitForTimeout(300);
    await page.screenshot({ path: info.outputPath(`${protocol}-cjk.png`) });
    await page.getByRole('button', { name: '登出', exact: true }).click();
    await expect(page.locator('#login-view')).toBeVisible();
    expect((await page.request.get('api/connections')).status()).toBe(401);
  });
}

test('Java-free connection editing, credential redaction, restart persistence and deletion', async ({ page, request }) => {
  const name = `Persist ${Date.now()}`;
  await login(page);
  await addConnection(page, name, 'ssh', 'example.invalid');
  await page.getByRole('button', { name: `編輯 ${name}`, exact: true }).click();
  const form = page.locator('#connection-form');
  await expect(form.locator('[name=password]')).toHaveValue('');
  await expect(form.locator('[name=password]')).toHaveAttribute('placeholder', '已設定；留空保留');
  await form.locator('[name=name]').fill(name + ' edited');
  await form.getByRole('button', { name: '儲存連線' }).click();
  await expect(page.getByRole('button', { name: `連線 ${name} edited`, exact: true })).toBeVisible();
  compose(['restart', 'web']);
  await ready(request);
  await login(page);
  await expect(page.getByRole('button', { name: `連線 ${name} edited`, exact: true })).toBeVisible();
  page.once('dialog', dialog => dialog.accept());
  await page.getByRole('button', { name: `刪除 ${name} edited`, exact: true }).click();
  await expect(page.getByRole('button', { name: `連線 ${name} edited`, exact: true })).toHaveCount(0);
});

for (const signal of ['SIGTERM', 'SIGINT']) {
  test(`Java-free ${signal} shuts down with an active browser SSH session`, async ({ page, request }) => {
    await ready(request);
    await login(page);
    const name = `Shutdown ${signal} ${Date.now()}`;
    await addConnection(page, name, 'ssh');
    await page.getByRole('button', { name: `連線 ${name}`, exact: true }).click();
    await expect(page.locator('#terminal-status')).toHaveText('已連線');
    await page.locator('#terminal').click({ position: { x: 30, y: 30 } });
    const file = `/tmp/cracamole-shutdown-${signal}-${Date.now()}`;
    await page.keyboard.type(`printf ready > ${file}`);
    await page.keyboard.press('Enter');
    await expect.poll(() => remoteFile(file)).toBe('ready');
    const start = Date.now();
    try {
      compose(['kill', '--signal', signal, 'web']);
      await expect.poll(() => JSON.parse(compose(['ps', '--all', '--format', 'json', 'web'])).State).toBe('exited');
      expect(JSON.parse(compose(['ps', '--all', '--format', 'json', 'web'])).ExitCode).toBe(0);
      expect(Date.now() - start).toBeLessThan(10000);
      await expect(page.locator('#terminal-status')).toHaveText('連線已結束');
    } finally {
      compose(['start', 'web']);
      await ready(request);
    }
  });
}
