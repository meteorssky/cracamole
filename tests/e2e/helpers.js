// Shared by the Playwright specs: fixture access (docker compose / ssh) and common UI actions.
const { expect } = require('@playwright/test');
const { execFileSync } = require('node:child_process');
const path = require('node:path');

const PASSWORD = 'fixture-admin-password-2026';
const SSH_OPTIONS = ['-o', 'BatchMode=yes', '-o', 'ConnectTimeout=5'];

// POSIX single-quote escaping for one shell word.
function shellQuote(value) {
  return "'" + String(value).split("'").join("'\\''") + "'";
}

function run(command, args) {
  return execFileSync(command, args, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: 30000 });
}

function compose(args) {
  const remote = process.env.CRACAMOLE_E2E_SSH;
  const directory = process.env.CRACAMOLE_E2E_REMOTE_DIR;
  if (remote && !directory) throw new Error('CRACAMOLE_E2E_REMOTE_DIR is required');
  const file = remote ? `${directory}/docker/compose.e2e.yml` : path.resolve(__dirname, '../../docker/compose.e2e.yml');
  const command = ['compose', '-p', 'cracamole-e2e', '-f', file, ...args];
  return remote
    ? run('ssh', [...SSH_OPTIONS, remote, ['docker', ...command].map(shellQuote).join(' ')])
    : run('docker', command);
}

// Contents of a file inside a fixture container ('' when it does not exist yet).
function remoteFile(file, service = 'backend') {
  const container = service === 'rdp' && process.env.CRACAMOLE_E2E_RDP_CONTAINER;
  try {
    if (container) {
      return run('ssh', [...SSH_OPTIONS, process.env.CRACAMOLE_E2E_SSH, `docker exec ${container} cat ${shellQuote(file)}`]);
    }
    return compose(['exec', '-T', service, 'cat', file]);
  } catch {
    return '';
  }
}

// Run a shell command as the RDP session user inside the xrdp fixture.
function rdpExec(script) {
  const container = process.env.CRACAMOLE_E2E_RDP_CONTAINER;
  const inner = ['exec', '-u', 'tester', '-e', 'DISPLAY=:10', container || 'rdp', 'sh', '-c', script];
  if (container) {
    return run('ssh', [...SSH_OPTIONS, process.env.CRACAMOLE_E2E_SSH, ['docker', ...inner].map(shellQuote).join(' ')]);
  }
  return compose([...inner.slice(0, 5), 'rdp', 'sh', '-c', script]);
}

async function ready(request) {
  await expect
    .poll(
      async () => {
        try {
          return (await request.get('health', { timeout: 3000 })).status();
        } catch {
          return 0;
        }
      },
      { timeout: 90000 },
    )
    .toBe(200);
}

// The server allows 10 login attempts per IP per minute, so tests share one signed-in session.
// Pass { fresh: true } when the test signs out (which ends the shared session) or tests the login itself.
let sharedCookies = null;

async function loginWithForm(page) {
  await page.goto('./');
  const loginForm = page.locator('#login-form');
  await loginForm.getByLabel('管理員帳號').fill('admin');
  await loginForm.getByLabel('密碼', { exact: true }).fill(PASSWORD);
  await page.getByRole('button', { name: '登入', exact: true }).click();
  await expect(page.locator('#workspace')).toBeVisible();
}

async function login(page, { fresh = false } = {}) {
  if (!fresh && sharedCookies) {
    await page.context().addCookies(sharedCookies);
    await page.goto('./');
    const workspace = page.locator('#workspace');
    const loginView = page.locator('#login-view');
    await expect.poll(async () => (await workspace.isVisible()) || (await loginView.isVisible())).toBe(true);
    if (await workspace.isVisible()) return;
    await page.context().clearCookies(); // the shared session ended (for example the server restarted)
  }
  await loginWithForm(page);
  if (!fresh) sharedCookies = await page.context().cookies();
}

// Number of "inked" (bright) pixels in the terminal, or in `clip` (a page-coordinate rectangle).
async function visibleInk(page, clip) {
  const screenshot = await (clip ? page.screenshot({ clip }) : page.locator('#terminal').screenshot());
  return page.evaluate(async encoded => {
    const image = new Image();
    image.src = 'data:image/png;base64,' + encoded;
    await image.decode();
    const canvas = document.createElement('canvas');
    canvas.width = image.width;
    canvas.height = image.height;
    const context = canvas.getContext('2d');
    context.drawImage(image, 0, 0);
    const pixels = context.getImageData(0, 0, canvas.width, canvas.height).data;
    let ink = 0;
    for (let i = 0; i < pixels.length; i += 4) {
      if (pixels[i + 3] && pixels[i] + pixels[i + 1] + pixels[i + 2] > 100) ink++;
    }
    return ink;
  }, screenshot.toString('base64'));
}

async function addConnection(page, name, protocol, host = 'backend', port = '', password = 'tester-pw') {
  await page.getByRole('button', { name: '新增連線', exact: true }).click();
  const form = page.locator('#connection-form');
  await form.getByLabel('連線名稱').fill(name);
  await form.getByLabel('協定', { exact: true }).selectOption(protocol);
  await form.getByLabel('主機名稱或 IP').fill(host);
  if (port) await form.getByLabel('連接埠').fill(String(port));
  await form.getByLabel('帳號', { exact: true }).fill('tester');
  await form.getByLabel('密碼', { exact: true }).fill(password);
  if (protocol === 'rdp') await form.getByRole('checkbox', { name: /接受自簽/ }).check();
  if (protocol === 'telnet') {
    await form.getByLabel('登入成功提示').fill(':~[$] ');
    await form.getByLabel('登入失敗提示').fill('Login incorrect');
  }
  await form.getByRole('button', { name: '儲存連線' }).click();
  await expect(page.locator('#connection-editor')).not.toBeVisible();
  await expect(page.getByRole('button', { name: `連線 ${name}`, exact: true })).toBeVisible();
}

module.exports = { PASSWORD, compose, remoteFile, rdpExec, ready, login, visibleInk, addConnection, shellQuote };
