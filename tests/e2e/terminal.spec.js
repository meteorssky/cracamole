const { test, expect } = require('@playwright/test');
const { execFileSync } = require('node:child_process');
const path = require('node:path');

function remoteFile(file) {
  try {
    return execFileSync('docker', [
      'compose', '-p', 'cracamole-e2e', '-f',
      path.resolve(__dirname, '../../docker/compose.e2e.yml'),
      'exec', '-T', 'backend', 'cat', file,
    ], { encoding: 'utf8', stdio: ['ignore', 'pipe', 'pipe'], timeout: 5000 });
  } catch {
    return '';
  }
}

for (const protocol of ['SSH', 'Telnet']) {
  test(`${protocol}: official webapp login, keyboard, vim, resize and CJK`, async ({ page }, info) => {
    await page.goto('./');
    await page.locator('.login-ui input[type="text"]').first().fill('guacadmin');
    await page.locator('.login-ui input[type="password"]').first().fill('guacadmin');
    await page.locator('.login-ui input.login').click();
    await expect(page.locator('.login-ui')).toBeHidden();
    await page.getByText(`${protocol} MVP`, { exact: true }).first().click();
    const canvas = page.locator('canvas:visible').first();
    await expect(canvas).toBeVisible();
    // Wait for actual glyphs, not merely the canvas created during tunnel setup.
    await expect.poll(() => page.locator('canvas').evaluateAll(canvases => {
      let ink = 0;
      for (const canvas of canvases) {
        if (!canvas.width || !canvas.height) continue;
        const pixels = canvas.getContext('2d').getImageData(0, 0, canvas.width, canvas.height).data;
        for (let i = 0; i < pixels.length; i += 4) {
          if (pixels[i + 3] && (pixels[i] + pixels[i + 1] + pixels[i + 2]) > 100) ink++;
        }
      }
      return ink;
    })).toBeGreaterThan(100);
    await canvas.click({ position: { x: 30, y: 30 } });
    const prefix = `/tmp/cracamole-${protocol.toLowerCase()}`;
    const marker = `${protocol}-${Date.now()}`;
    await page.keyboard.type(`printf '${marker}' > ${prefix}-keyboard`);
    await page.keyboard.press('Enter');
    await expect.poll(() => remoteFile(`${prefix}-keyboard`)).toBe(marker);

    await page.keyboard.type(`vim -Nu NONE -n ${prefix}-vim`);
    await page.keyboard.press('Enter');
    // Vim is a full-screen program; allow it to finish its terminal query exchange.
    await page.waitForTimeout(1000);
    await page.keyboard.type('i');
    await page.keyboard.type(marker);
    await page.keyboard.press('Escape');
    await page.keyboard.type(':wq');
    await page.keyboard.press('Enter');
    await expect.poll(() => remoteFile(`${prefix}-vim`)).toBe(`${marker}\n`);

    await page.keyboard.type(`stty size > ${prefix}-size-before`);
    await page.keyboard.press('Enter');
    await expect.poll(() => remoteFile(`${prefix}-size-before`)).toMatch(/^\d+ \d+\r?\n$/);
    const oldSize = remoteFile(`${prefix}-size-before`).trim();
    // The command only finishes when the PTY itself observes the browser resize.
    await page.keyboard.type(`while [ "$(stty size)" = "${oldSize}" ]; do sleep 0.1; done; stty size > ${prefix}-size-after`);
    await page.keyboard.press('Enter');
    await page.setViewportSize({ width: 1000, height: 700 });
    await expect.poll(() => remoteFile(`${prefix}-size-after`)).toMatch(/^\d+ \d+\r?\n$/);
    expect(remoteFile(`${prefix}-size-after`).trim()).not.toBe(oldSize);
    // Exercise terminal Unicode output through real server bytes using ASCII shell input.
    await page.keyboard.type(`printf '\\344\\270\\255\\346\\226\\207' | tee ${prefix}-cjk`);
    await page.keyboard.press('Enter');
    await expect.poll(() => remoteFile(`${prefix}-cjk`)).toBe('中文');
    await page.screenshot({ path: info.outputPath(`${protocol}-cjk.png`) });
  });
}
