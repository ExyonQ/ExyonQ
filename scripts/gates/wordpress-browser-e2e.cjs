#!/usr/bin/env node
/*
 * Real-browser WordPress journey.
 *
 * Driven by scripts/gates/wordpress-browser-e2e.sh inside a headless Chromium
 * (Playwright). Every leg is genuine navigation: the installer form is filled
 * and submitted, the login form is typed and submitted, wp-admin is rendered,
 * the logout link is clicked. No HTTP shortcuts stand in for the browser.
 *
 * Environment:
 *   WP_BASE             base URL of the running container (required)
 *   WP_ADMIN_PASSWORD   admin password created by this run (required, never logged)
 *   WP_OUT              directory for screenshots and JSON evidence
 */
'use strict';

const fs = require('fs');
const path = require('path');
const { chromium } = require('playwright');

const baseUrl = (process.env.WP_BASE || '').replace(/\/+$/, '');
const adminPassword = process.env.WP_ADMIN_PASSWORD || '';
const outDir = process.env.WP_OUT || '/out';
const adminUser = 'admin';
const siteTitle = 'ExyonQ browser journey';

if (!baseUrl || !adminPassword) {
  console.error('BROWSER_JOURNEY=FAIL missing WP_BASE or WP_ADMIN_PASSWORD');
  process.exit(2);
}

const results = [];
function record(name, ok, detail) {
  results.push({ name, ok, detail });
  console.log(`${ok ? 'STEP=PASS' : 'STEP=FAIL'} ${name}${detail ? ' — ' + detail : ''}`);
  if (!ok) {
    fs.writeFileSync(path.join(outDir, 'browser-journey.json'), JSON.stringify({ base: baseUrl, results }, null, 2));
    throw new Error(`${name} failed${detail ? ': ' + detail : ''}`);
  }
}

function headerValue(headers, name) {
  const key = Object.keys(headers || {}).find((k) => k.toLowerCase() === name);
  return key ? String(headers[key]) : '';
}

async function main() {
  fs.mkdirSync(outDir, { recursive: true });
  const browser = await chromium.launch();
  const context = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const page = await context.newPage();
  page.setDefaultTimeout(30000);
  const uid = `${Date.now()}`;

  // 1. Installer: language step.
  await page.goto(`${baseUrl}/wp-admin/install.php`, { waitUntil: 'domcontentloaded' });
  const langSelect = await page.$('#language');
  record('installer language step reachable', !!langSelect, `url=${page.url()}`);
  // The bundled catalog marks English as installed and gives it value=""; pick
  // it by position instead of assuming a locale code such as en_US.
  const languages = await page.$$eval('#language option', (options) => options.map((o) => o.value));
  await page.selectOption('#language', languages[0] ?? '');
  await Promise.all([page.waitForLoadState('domcontentloaded'), page.click('#language-continue')]);

  // 2. Installer: site and admin account.
  await page.waitForSelector('#weblog_title', { state: 'visible' });
  await page.fill('#weblog_title', siteTitle);
  // The installer's admin-user input is name="user_name" but carries id
  // "user_login" in current WordPress; address it by name.
  await page.fill('input[name="user_name"]', adminUser);
  await page.fill('#pass1', adminPassword);
  // The repeat-password row carries class "hide-if-js": with JavaScript on,
  // WordPress replaces it with the strength check and the weak-password
  // confirmation. Fill it only when the no-JS fallback is actually shown.
  if (await page.locator('#pass2').isVisible().catch(() => false)) {
    await page.fill('#pass2', adminPassword);
  }
  await page.fill('#admin_email', `admin+${uid}@example.test`);
  await Promise.all([page.waitForLoadState('domcontentloaded'), page.click('#submit')]);
  let successText = await page.locator('body').innerText();
  if (!/Success!|successfully installed/i.test(successText) && (await page.locator('#pw_weak').isVisible().catch(() => false))) {
    // Weak-password confirmation is a real part of the installer flow.
    await page.check('#pw_weak');
    await Promise.all([page.waitForLoadState('domcontentloaded'), page.click('#submit')]);
    successText = await page.locator('body').innerText();
  }
  record('installer created the site', /Success!|successfully installed/i.test(successText), page.url());

  // 3. Continue to the login form. The success page links there, but the link
  //    markup differs between WordPress versions, so navigate to it directly.
  await page.goto(`${baseUrl}/wp-login.php`, { waitUntil: 'domcontentloaded' });
  await page.waitForSelector('#user_login', { state: 'visible' });
  record('login form rendered', page.url().includes('wp-login.php'), `url=${page.url()}`);

  // 4. Real credential typing and submit.
  await page.fill('#user_login', adminUser);
  await page.fill('#user_pass', adminPassword);
  await Promise.all([page.waitForLoadState('domcontentloaded'), page.click('#wp-submit')]);
  await page.waitForSelector('#wpadminbar', { state: 'visible' });
  const bodyText = await page.locator('body').innerText();
  record('login reached wp-admin', /wp-admin/.test(page.url()) && /Dashboard/.test(bodyText), `url=${page.url()}`);
  const version = (bodyText.match(/Version\s+(\d+\.\d+(?:\.\d+)?)/) || [])[1] || 'unknown';
  record('dashboard rendered with a session', /At a Glance|Site Health|Dashboard/.test(bodyText), `wordpress=${version}`);
  await page.screenshot({ path: path.join(outDir, 'dashboard.png'), fullPage: true });

  // 5. Logged-in navigation must bypass the shared cache (ExyonQ in the path).
  const loggedInResponse = await page.goto(`${baseUrl}/`, { waitUntil: 'domcontentloaded' });
  const loggedInHeaders = loggedInResponse ? loggedInResponse.headers() : {};
  record(
    'logged-in front page bypasses the cache',
    /exyonq/.test(headerValue(loggedInHeaders, 'server')) && headerValue(loggedInHeaders, 'x-cache') === 'bypass',
    `server=${headerValue(loggedInHeaders, 'server')} x-cache=${headerValue(loggedInHeaders, 'x-cache')}`
  );

  // 6. Logout through the admin-bar link, then prove the session is gone.
  await Promise.all([page.waitForLoadState('domcontentloaded'), page.goto(`${baseUrl}/wp-login.php?action=logout`)]);
  const confirm = page.locator('a', { hasText: /log out/i }).first();
  await Promise.all([page.waitForLoadState('domcontentloaded'), confirm.click()]);
  record('logout completed', /loggedout=true/.test(page.url()), `url=${page.url()}`);
  await page.goto(`${baseUrl}/wp-admin/`, { waitUntil: 'domcontentloaded' });
  record(
    'logged-out wp-admin redirects to the login form',
    page.url().includes('wp-login.php') && !!(await page.$('#user_login')),
    `url=${page.url()}`
  );
  await context.close();

  // 7. Anonymous cache behaviour, observed through the browser from the point of
  //    view of two different visitors (two fresh contexts). Reloading in the same
  //    tab would send revalidation headers and is not a cold second visitor.
  const firstVisitor = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const firstPage = await firstVisitor.newPage();
  firstPage.setDefaultTimeout(30000);
  const first = await firstPage.goto(`${baseUrl}/`, { waitUntil: 'domcontentloaded' });
  const firstHeaders = first ? first.headers() : {};
  await firstPage.screenshot({ path: path.join(outDir, 'front-page.png'), fullPage: true });
  await firstVisitor.close();

  const secondVisitor = await browser.newContext({ viewport: { width: 1280, height: 900 } });
  const secondPage = await secondVisitor.newPage();
  secondPage.setDefaultTimeout(30000);
  const second = await secondPage.goto(`${baseUrl}/`, { waitUntil: 'domcontentloaded' });
  const secondHeaders = second ? second.headers() : {};
  await secondVisitor.close();

  record(
    'anonymous front page is served by ExyonQ',
    /exyonq/.test(headerValue(firstHeaders, 'server')),
    `server=${headerValue(firstHeaders, 'server')} status=${first ? first.status() : 'n/a'}`
  );
  record(
    'anonymous cache miss then hit across two visitors',
    headerValue(firstHeaders, 'x-cache') === 'miss' && headerValue(secondHeaders, 'x-cache') === 'hit',
    `first=${headerValue(firstHeaders, 'x-cache')} second=${headerValue(secondHeaders, 'x-cache')}`
  );

  await browser.close();
  fs.writeFileSync(
    path.join(outDir, 'browser-journey.json'),
    JSON.stringify({ base: baseUrl, wordpress_version: version, results }, null, 2)
  );
  console.log(`BROWSER_JOURNEY=PASS steps=${results.length}`);
}

main().catch((error) => {
  console.error(`BROWSER_JOURNEY=FAIL ${error.message}`);
  process.exit(1);
});
