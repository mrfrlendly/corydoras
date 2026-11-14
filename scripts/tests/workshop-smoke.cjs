// Run "npm run build" first. Install Playwright and its WebKit browser in a test environment.
// Run "node scripts/tests/workshop-smoke.cjs". NODE_PATH can point to an existing Playwright installation.
const { resolve, extname, sep } = require('node:path');
const { mkdir, readFile } = require('node:fs/promises');
const assert = require('node:assert/strict');
const { webkit } = require('playwright');
const { expect } = require('playwright/test');
const { mockDesktop } = require('./workshop-fixture.cjs');

const dist = resolve(__dirname, '../../dist');
const origin = 'https://corydoras.test';
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.png': 'image/png' };
const png = Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO6+q4sAAAAASUVORK5CYII=', 'base64');

async function openApp(browser, errors) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 920 }, reducedMotion: 'reduce', serviceWorkers: 'block' });
  await context.addInitScript(mockDesktop);
  await context.route('**/*', async route => {
    const url = new URL(route.request().url());
    const file = resolve(dist, '.' + (url.pathname === '/' ? '/index.html' : url.pathname));
    if (url.origin !== origin || !file.startsWith(dist + sep)) return route.abort();
    try {
      await route.fulfill({ body: await readFile(file), contentType: types[extname(file)] });
    } catch {
      await route.fulfill({ status: 404, body: 'Missing test asset' });
    }
  });
  const page = await context.newPage();
  page.on('pageerror', error => errors.push(error.message));
  page.setDefaultTimeout(8_000);
  await page.goto(origin);
  return page;
}

async function enableWorkshop(page, enabled) {
  await page.getByRole('complementary').getByRole('button', { name: /Settings/ }).click();
  await page.getByRole('button', { name: 'Modules', exact: true }).click();
  const control = page.getByRole('checkbox', { name: /Workshop inventory/i });
  await control.setChecked(enabled);
  const nav = page.getByRole('complementary').getByRole('button', { name: 'Workshop inventory' });
  if (enabled) await nav.click();
  else await expect(nav).toHaveCount(0);
}

const button = (page, name) => page.getByRole('button', { name, exact: true });

async function manualFlow(page) {
  const item = 'Bench vise';
  await button(page, 'Add item').click();
  await page.getByLabel('Item name').fill(item);
  await page.getByLabel('Category', { exact: true }).fill('Tools');
  await page.getByLabel('Quantity on hand').fill('2');
  await button(page, 'Save item').click();
  await button(page, item).click();
  await page.getByLabel('Quantity on hand').fill('0');
  await button(page, 'Save item').click();

  const viseRow = page.getByRole('row').filter({ has: button(page, item) });
  await expect(viseRow.getByRole('cell', { name: '0', exact: true })).toBeVisible();
  await page.getByLabel('Filter by category').selectOption('Tools');
  const filter = page.getByRole('searchbox', { name: 'Search inventory' });
  await filter.fill('washers');
  await expect(button(page, item)).toHaveCount(0);
  await filter.fill('vise');
  await button(page, item).click();
  await expect(page.getByLabel('Quantity on hand')).toHaveValue('0');
  await button(page, 'Delete item').click();
  await button(page, 'Confirm deletion').click();
  await expect(button(page, item)).toHaveCount(0);
}

async function labelWithoutLookup(page) {
  await button(page, 'Add item').click();
  await page.getByRole('checkbox', { name: /Search suppliers after label analysis/ }).uncheck();
  await page.getByLabel('Choose label photo').setInputFiles({ name: 'label.png', mimeType: 'image/png', buffer: png });
  await expect(page.getByLabel('Item name')).toHaveValue('Socket head screws');
  await button(page, 'Save item').click();
  await expect(button(page, 'Socket head screws')).toBeVisible();
  assert.equal(await page.evaluate(() => window.workshopProbe.searches), 0);
  await button(page, 'Socket head screws').click();
  await button(page, 'Delete item').click();
  await button(page, 'Confirm deletion').click();
  await expect(button(page, 'Socket head screws')).toHaveCount(0);
}

async function photoFlow(page) {
  await button(page, 'Add item').click();
  await page.evaluate(() => { window.workshopProbe.failSearch = true; });
  await page.getByLabel('Choose label photo').setInputFiles({ name: 'label.png', mimeType: 'image/png', buffer: png });
  await expect(page.getByLabel('Item name')).toHaveValue('Socket head screws');
  await expect(page.getByLabel('Choose label photo')).toHaveCount(0);
  await expect(page.getByRole('img', { name: 'Selected product label' })).toBeVisible();
  await expect(page.getByLabel('Brand', { exact: true })).toHaveValue('Sample Hardware');
  await expect(page.getByLabel('Dimensions')).toHaveValue('M3 × 6 mm');
  await expect(page.getByLabel('Category', { exact: true })).toHaveValue('Fasteners');
  await expect(page.getByLabel('Quantity on hand')).toHaveValue('');
  await expect(page.getByLabel('Quantity per pack')).toHaveValue('25');
  await expect(page.getByText(/Supplier connection failed/)).toBeVisible();
  await page.evaluate(() => { window.workshopProbe.failSearch = false; });
  await button(page, 'Search suppliers').click();
  await expect(button(page, 'Use listing')).toBeVisible();
  await expect(button(page, 'Remove listing')).toHaveCount(0);
  await button(page, 'Use listing').click();
  await page.getByLabel('Quantity on hand').fill('11');
  await button(page, 'Save item').click();
  await expect(button(page, 'Socket head screws')).toBeVisible();
  assert.equal(await page.evaluate(() => window.workshopProbe.searches), 2);
}

async function smoke() {
  await readFile(resolve(dist, 'index.html'));
  const browser = await webkit.launch();
  try {
    const errors = [];
    const page = await openApp(browser, errors);
    const workshop = button(page, 'Workshop inventory');
    await expect(page.getByRole('heading', { name: 'Make your first design language.' })).toBeVisible();
    await expect(workshop).toHaveCount(0);
    assert.equal(await page.evaluate(() => window.workshopProbe.commands.includes('list_inventory')), false);
    await enableWorkshop(page, true);
    await manualFlow(page);
    assert.equal(await page.evaluate(() => window.workshopProbe.searches), 0);
    await labelWithoutLookup(page);
    await photoFlow(page);
    await enableWorkshop(page, false);
    await page.reload();
    await expect(workshop).toHaveCount(0);
    await enableWorkshop(page, true);
    const saved = button(page, 'Socket head screws');
    await expect(saved).toBeVisible();
    if (process.env.WORKSHOP_SCREENSHOTS) {
      await mkdir(process.env.WORKSHOP_SCREENSHOTS, { recursive: true });
      await page.screenshot({ path: resolve(process.env.WORKSHOP_SCREENSHOTS, 'inventory-list.png') });
    }
    await saved.click();
    await expect(page.getByLabel('Quantity on hand')).toHaveValue('11');
    await expect(page.getByLabel('Quantity per pack')).toHaveValue('25');
    await expect(button(page, 'Open Socket head screws SH-306 on supplier.example.com')).toBeVisible();
    await expect(page.getByText('Stainless steel, A2 grade.')).toBeVisible();
    if (process.env.WORKSHOP_SCREENSHOTS) {
      await page.screenshot({ path: resolve(process.env.WORKSHOP_SCREENSHOTS, 'inventory-item.png') });
    }
    assert.deepEqual(errors, []);
    console.log('Workshop inventory browser smoke test passed.');
  } finally {
    await browser.close();
  }
}
smoke().catch(error => { console.error(error); process.exitCode = 1; });
