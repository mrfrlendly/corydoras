// Run "npm run build" first. Install Playwright and its WebKit browser in a test environment.
// Run "node scripts/tests/workshop-smoke.cjs". NODE_PATH can point to an existing Playwright installation.
const { resolve, extname, sep } = require('node:path');
const { mkdir, readFile } = require('node:fs/promises');
const assert = require('node:assert/strict');
const { webkit } = require('playwright');
const { expect } = require('playwright/test');
const { mockDesktop } = require('./workshop-fixture.cjs');

const dist = process.env.WORKSHOP_DIST || resolve(__dirname, '../../dist');
const origin = 'https://corydoras.test';
const types = { '.html': 'text/html', '.js': 'text/javascript', '.css': 'text/css', '.png': 'image/png' };

async function openApp(browser, errors, seed) {
  const context = await browser.newContext({ viewport: { width: 1440, height: 920 }, reducedMotion: 'reduce', serviceWorkers: 'block' });
  await context.addInitScript(mockDesktop, seed);
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

const fixture = name => resolve(__dirname, 'fixtures', name + '-spool.png');
const itemNames = page => page.getByRole('table').locator('tbody th button');
const itemRow = (page, name) => page.getByRole('row').filter({ has: button(page, name) });
const sortBy = (page, field) => page.getByRole('columnheader').getByRole('button', { name: field, exact: true }).click();

async function catalogueFlow(browser, errors) {
  const blue = (await readFile(fixture('blue'))).toString('base64');
  const red = (await readFile(fixture('red'))).toString('base64');
  const key = 'a'.repeat(64);
  const otherKey = 'b'.repeat(64);
  const row = { category: 'Fasteners', brand: '', notes: '', part_number: '', dimensions: '', supplier: null };
  const seed = {
    records: [
      { ...row, id: 'm10', name: 'M10 bolt', colour: 'Blue', quantity: 0, pack_quantity: 20, photo_key: key },
      { ...row, id: 'm8', name: 'M8 bolt', colour: '', quantity: null, pack_quantity: null, photo_key: null },
      { ...row, id: 'm2', name: 'M2 bolt', colour: 'Red', quantity: 50, pack_quantity: 5, photo_key: otherKey },
      { ...row, id: 'm4', name: 'M4 bolt', colour: 'Blue', quantity: 9, pack_quantity: 20, photo_key: key },
    ],
    media: { [key]: 'data:image/png;base64,' + blue, [otherKey]: 'data:image/png;base64,' + red },
  };
  const page = await openApp(browser, errors, seed);
  await page.setViewportSize({ width: 960, height: 620 });
  await enableWorkshop(page, true);
  const names = itemNames(page);
  await expect(names).toHaveText(['M2 bolt', 'M4 bolt', 'M8 bolt', 'M10 bolt']);
  const savedImage = itemRow(page, 'M4 bolt').locator('img[data-workshop-thumbnail]');
  await expect(savedImage).toBeVisible();
  await expect.poll(() => savedImage.evaluate(image => image.naturalWidth)).toBe(56);
  await expect(itemRow(page, 'M8 bolt').locator('img[data-workshop-thumbnail]')).toHaveCount(0);
  await expect(itemRow(page, 'M2 bolt').getByText('Red', { exact: true })).toBeVisible();
  await sortBy(page, 'Colour');
  await expect(names).toHaveText(['M4 bolt', 'M10 bolt', 'M2 bolt', 'M8 bolt']);
  await sortBy(page, 'Colour');
  await expect(names).toHaveText(['M2 bolt', 'M4 bolt', 'M10 bolt', 'M8 bolt']);
  await sortBy(page, 'On hand');
  await expect(names).toHaveText(['M10 bolt', 'M4 bolt', 'M2 bolt', 'M8 bolt']);
  await sortBy(page, 'On hand');
  await expect(names).toHaveText(['M2 bolt', 'M4 bolt', 'M10 bolt', 'M8 bolt']);
  const search = page.getByRole('searchbox', { name: 'Search inventory' });
  await search.fill('Blue');
  await expect(names).toHaveText(['M4 bolt', 'M10 bolt']);
  await search.fill('');
  await expect(page.getByRole('columnheader').filter({ has: button(page, 'On hand') })).toHaveAttribute('aria-sort', 'descending');
  if (process.env.WORKSHOP_SCREENSHOTS) {
    await mkdir(process.env.WORKSHOP_SCREENSHOTS, { recursive: true });
    await page.screenshot({ path: resolve(process.env.WORKSHOP_SCREENSHOTS, 'sorted-inventory-960.png') });
  }
  await page.reload();
  await button(page, 'Workshop inventory').click();
  await expect(names).toHaveText(['M2 bolt', 'M4 bolt', 'M10 bolt', 'M8 bolt']);
  await page.close();
}

async function attachmentFlow(page) {
  const name = 'Sample spool';
  const blue = fixture('blue');
  const red = fixture('red');
  const before = await page.evaluate(() => window.workshopProbe.commands.filter(c => c === 'read_inventory_label').length);
  await button(page, 'Add item').click();
  await page.getByLabel('Item name').fill(name);
  await page.getByLabel('Colour', { exact: true }).fill('Blue');
  await page.getByLabel('Add item photo', { exact: true }).setInputFiles(blue);
  await button(page, 'Save item').click();
  const img = itemRow(page, name).locator('img[data-workshop-thumbnail]');
  await expect(img).toBeVisible();
  const original = await img.getAttribute('src');
  await button(page, name).click();
  await page.getByLabel('Replace item photo').setInputFiles(red);
  await button(page, 'Cancel').click();
  await expect(img).toHaveAttribute('src', original);
  await button(page, name).click();
  await page.getByLabel('Replace item photo').setInputFiles(red);
  await page.getByLabel('Colour', { exact: true }).fill('Red');
  await button(page, 'Save item').click();
  await expect(itemRow(page, name).getByText('Red', { exact: true })).toBeVisible();
  await expect(img).toBeVisible();
  await expect(img).not.toHaveAttribute('src', original);
  await button(page, name).click();
  await button(page, 'Remove photo').click();
  await button(page, 'Save item').click();
  await expect(button(page, name)).toBeVisible();
  await expect(img).toHaveCount(0);
  assert.equal(await page.evaluate(() => window.workshopProbe.commands.filter(c => c === 'read_inventory_label').length), before);
  await button(page, name).click();
  await button(page, 'Delete item').click();
  await button(page, 'Confirm deletion').click();
}

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
  await page.getByLabel('Choose label photo').setInputFiles(fixture('blue'));
  await expect(page.getByLabel('Item name')).toHaveValue('Socket head screws');
  await button(page, 'Save item').click();
  await expect(button(page, 'Socket head screws')).toBeVisible();
  assert.equal(await page.evaluate(() => window.workshopProbe.searches), 0);
  await button(page, 'Socket head screws').click();
  await button(page, 'Delete item').click();
  await button(page, 'Confirm deletion').click();
  await expect(button(page, 'Socket head screws')).toHaveCount(0);
}

const spec = (role, value, unit, source, evidence) => ({ role, name: '', value, unit, tolerance: '', source, evidence });
const draft = (name, pack, specs) => ({
  name, brand: 'Sample Hardware', part_number: '', colour: '', category: 'Fasteners', dimensions: '',
  quantity: null, pack_quantity: pack, notes: '', specs, supplier: null,
});
const readCount = page => page.evaluate(() => window.workshopProbe.commands.filter(c => c === 'read_inventory_label').length);
const cards = page => page.getByRole('region', { name: /^Item \d/ });

async function multiItemFlow(page) {
  await page.evaluate(([a, b, c]) => { window.workshopProbe.labels = [a, b, c]; }, [
    draft('M3 heat-set insert', 50, [
      spec('thread', 'M3', '', 'notation', 'M3x5.7'), spec('length', '5.7', 'mm', 'notation', 'M3x5.7'),
      spec('pitch', '0.5', 'mm', 'standard', 'ISO 261 coarse pitch, M3')]),
    draft('M4 heat-set insert', 40, [spec('thread', 'M4', '', 'notation', 'M4x8')]),
    draft('Spare label row', null, []),
  ]);
  await button(page, 'Add from photo').click();
  await page.getByLabel('Choose label photo').setInputFiles(fixture('blue'));
  await expect(cards(page)).toHaveCount(3);
  const first = page.getByRole('region', { name: 'Item 1' });
  const third = page.getByRole('region', { name: 'Item 3' });
  await expect(first.getByLabel('Item name')).toHaveValue('M3 heat-set insert');
  await expect(first.getByText('Printed, role read from notation').first()).toBeVisible();
  await expect(first.getByText('Standard, not measured')).toBeVisible();
  await expect(first.getByLabel('Dimensions')).not.toBeEditable();
  await expect(first.getByLabel('Dimensions')).toHaveValue('M3 thread; length 5.7 mm; pitch 0.5 mm');
  await expect(first.getByLabel('Quantity on hand')).toHaveValue('');
  await first.getByRole('button', { name: 'Add spec' }).click();
  await first.getByLabel('Spec 4 role').selectOption('hole_diameter');
  await first.getByLabel('Spec 4 value').fill('4.2');
  await first.getByLabel('Spec 4 unit').selectOption('mm');
  await expect(first.getByText('Entered', { exact: true })).toBeVisible();
  await expect(first.getByLabel('Dimensions')).toHaveValue(/hole diameter 4\.2 mm$/);
  await first.getByRole('button', { name: 'I bought one pack' }).click();
  await expect(first.getByLabel('Quantity on hand')).toHaveValue('50');
  await expect(third.getByRole('button', { name: 'I bought one pack' })).toBeDisabled();
  if (process.env.WORKSHOP_SCREENSHOTS) {
    await mkdir(process.env.WORKSHOP_SCREENSHOTS, { recursive: true });
    await page.screenshot({ path: resolve(process.env.WORKSHOP_SCREENSHOTS, 'photo-review.png'), fullPage: true });
  }
  await third.getByRole('button', { name: 'Discard' }).click();
  await expect(cards(page)).toHaveCount(2);
  await button(page, 'Cancel').click();
  await expect(page.getByText('Discard the 2 unsaved items from this photo?')).toBeVisible();
  await button(page, 'Keep reviewing').click();
  await expect(page.getByText('Discard the 2 unsaved items from this photo?')).toHaveCount(0);
  await button(page, 'Save 2 items').click();
  await expect(button(page, 'M3 heat-set insert')).toBeVisible();
  await expect(button(page, 'M4 heat-set insert')).toBeVisible();
  assert.equal(await page.evaluate(() => window.workshopProbe.commands.filter(c => c === 'save_inventory_batch').length), 1);
  const stored = await page.evaluate(() => JSON.parse(sessionStorage.inventory).filter(r => r.name.endsWith('heat-set insert')));
  assert.equal(stored.length, 2);
  assert.equal(stored[0].photo_key, stored[1].photo_key);
  await button(page, 'M3 heat-set insert').click();
  await expect(page.getByLabel('Dimensions')).not.toBeEditable();
  await expect(page.getByLabel('Spec 4 value')).toHaveValue('4.2');
  await expect(page.getByLabel('Quantity on hand')).toHaveValue('50');
  await button(page, 'Cancel').click();
  await page.evaluate(() => { window.workshopProbe.labels = null; });
}

async function handoffFlow(page) {
  await page.evaluate(() => {
    const spec = { role: 'thread', name: '', value: 'M3', unit: '', tolerance: '', source: 'label', evidence: 'M3' };
    const row = name => ({ name, brand: '', part_number: '', colour: '', category: 'Fasteners', dimensions: '', quantity: null, pack_quantity: 10, notes: '', specs: [spec], supplier: null });
    window.workshopProbe.labels = [row('M3 nut'), row('M3 washer')];
  });
  const before = await readCount(page);
  await button(page, 'Add item').click();
  await page.getByLabel('Choose label photo').setInputFiles(fixture('blue'));
  await expect(page.getByText('This photo shows 2 items.')).toBeVisible();
  await button(page, 'Open them in the review list').click();
  await expect(page.getByRole('heading', { name: 'Add from photo' })).toBeVisible();
  await expect(cards(page)).toHaveCount(2);
  assert.equal(await readCount(page), before + 1);
  await button(page, 'Cancel').click();
  await button(page, 'Discard items').click();
  await expect(page.getByRole('heading', { name: 'Workshop inventory' })).toBeVisible();
  await expect(button(page, 'M3 nut')).toHaveCount(0);
  await page.evaluate(() => { window.workshopProbe.labels = null; });
}

async function photoFlow(page) {
  await button(page, 'Add item').click();
  await page.evaluate(() => { window.workshopProbe.failSearch = true; });
  await page.getByLabel('Choose label photo').setInputFiles(fixture('blue'));
  await expect(page.getByLabel('Item name')).toHaveValue('Socket head screws');
  await expect(page.getByLabel('Choose label photo')).toHaveCount(0);
  await expect(page.getByRole('img', { name: 'Selected product label' })).toBeVisible();
  await expect(page.getByLabel('Colour', { exact: true })).toHaveValue('Blue');
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
    await catalogueFlow(browser, errors);
    const page = await openApp(browser, errors);
    const workshop = button(page, 'Workshop inventory');
    await expect(page.getByRole('heading', { name: 'Make your first design language.' })).toBeVisible();
    await expect(workshop).toHaveCount(0);
    assert.equal(await page.evaluate(() => window.workshopProbe.commands.includes('list_inventory')), false);
    await enableWorkshop(page, true);
    await manualFlow(page);
    await attachmentFlow(page);
    assert.equal(await page.evaluate(() => window.workshopProbe.searches), 0);
    await labelWithoutLookup(page);
    await multiItemFlow(page);
    await handoffFlow(page);
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
