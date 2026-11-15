const { test, expect } = require('@playwright/test');
const http = require('node:http');
const https = require('node:https');
const { ready } = require('./helpers');

test('content-versioned entry loads with immutable cache policy', async ({ page, request }) => {
  await ready(request);
  await page.goto('./');
  const moduleUrl = await page.locator('script[type="module"]').getAttribute('src');
  expect(moduleUrl).toMatch(/^\/assets\/[a-f0-9]{64}\/js\/main\.js$/);
  const response = await request.get(moduleUrl);
  expect(response.status()).toBe(200);
  expect(response.headers()['cache-control']).toBe('public, max-age=31536000, immutable');
  await expect(page.locator('#login-form')).toBeVisible();
});

test('browser receives consistent static assets across repeated requests', async ({ page, request }, info) => {
  await ready(request);
  const upstream = new URL(process.env.CRACAMOLE_URL || 'http://127.0.0.1:18080/');
  const requests = [];
  const proxy = http.createServer((incoming, outgoing) => {
    const tracked = incoming.url.startsWith('/assets/css/base.css?cache-regression=');
    const forwarded = (upstream.protocol === 'https:' ? https : http).request(new URL(incoming.url, upstream), { method: incoming.method, headers: incoming.headers }, response => {
      if (tracked) requests.push({ tag: incoming.headers['if-none-match'], status: response.statusCode });
      outgoing.writeHead(response.statusCode, response.headers);
      response.pipe(outgoing);
    });
    forwarded.on('error', () => { if (!outgoing.headersSent) outgoing.writeHead(502); outgoing.end(); });
    incoming.pipe(forwarded);
  });
  await new Promise(resolve => proxy.listen(0, '127.0.0.1', resolve));
  try {
    await page.goto(`http://127.0.0.1:${proxy.address().port}/`);
    const asset = `/assets/css/base.css?cache-regression=${Date.now()}`;
    const first = await page.evaluate(async url => { const result = await fetch(url); return { status: result.status, body: await result.text() }; }, asset);
    const second = await page.evaluate(async url => { const result = await fetch(url); return { status: result.status, body: await result.text() }; }, asset);
    expect(first.status).toBe(200);
    expect(second.status).toBe(200);
    expect(first.body.length).toBeGreaterThan(0);
    expect(second.body).toBe(first.body);
    expect(requests).toHaveLength(2);
    expect(requests[0].status).toBe(200);
    if (info.project.name === 'webkit' && !requests[1].tag) {
      expect(requests[1].status).toBe(200);
    } else {
      expect(requests[1].tag).toMatch(/^"[a-f0-9]{64}"$/);
      expect(requests[1].status).toBe(304);
    }
  } finally {
    await new Promise(resolve => proxy.close(resolve));
  }
});
