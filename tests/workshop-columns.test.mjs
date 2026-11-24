import test from 'node:test';
import assert from 'node:assert/strict';
import {
  autoColumns, captionFields, columnDefs, formatMoney, formatUnitPrice, stockValueCents, unitPriceCents,
  isEstimatedPrice, viewTotals, visibleColumns,
} from '../src/components/workshopColumns.ts';

const item = (id, values = {}) => ({
  id, name: id, category: 'Fasteners', brand: '', part_number: '', colour: '', dimensions: '', quantity: null,
  pack_quantity: null, price_cents: null, price_source: '', measure_unit: '', measure_full: null, measure_left: null, notes: '', specs: [], supplier: null, photo_key: null, ...values,
});
const ids = ['category', 'brand', 'part_number', 'colour', 'dimensions', 'quantity', 'pack_quantity', 'open_amount', 'price',
  'unit_price', 'stock_value', 'notes', 'supplier'];

test('every column has a definition with the label the list shows', () => {
  assert.deepEqual(Object.keys(columnDefs).sort(), ['name', ...ids].sort());
  const labels = { name: 'Item', category: 'Category', brand: 'Brand', part_number: 'Part number', colour: 'Colour',
    dimensions: 'Dimensions', quantity: 'On hand', pack_quantity: 'Per pack', open_amount: 'Open pack', price: 'Price', unit_price: 'Unit price',
    stock_value: 'Stock value', notes: 'Notes', supplier: 'Supplier' };
  for (const [id, label] of Object.entries(labels)) {
    assert.equal(columnDefs[id].label, label);
    assert.equal(columnDefs[id].id, id);
    assert.equal(typeof columnDefs[id].sortable, 'boolean');
  }
  const numeric = ids.filter(id => columnDefs[id].numeric);
  assert.deepEqual(numeric, ['quantity', 'pack_quantity', 'open_amount', 'price', 'unit_price', 'stock_value']);
});

test('unit price divides the pack price by the pack count and keeps fractions of a cent', () => {
  assert.equal(unitPriceCents(item('a', { price_cents: 999, pack_quantity: 1 })), 999);
  assert.ok(Math.abs(unitPriceCents(item('a', { price_cents: 999, pack_quantity: 45 })) - 22.2) < 1e-9);
  assert.equal(unitPriceCents(item('a', { price_cents: 999, pack_quantity: null })), 999);
  assert.equal(unitPriceCents(item('a', { price_cents: 999, pack_quantity: 0 })), 999);
  assert.equal(unitPriceCents(item('a', { price_cents: 0, pack_quantity: 10 })), 0);
  assert.equal(unitPriceCents(item('a', { pack_quantity: 10 })), null);
});

test('stock value is the count times the unit price in whole cents, and never treats a missing price as zero', () => {
  assert.equal(stockValueCents(item('a', { price_cents: 999, pack_quantity: 1, quantity: 3 })), 2997);
  assert.equal(stockValueCents(item('a', { price_cents: 999, pack_quantity: 45, quantity: 90 })), 1998);
  assert.equal(stockValueCents(item('a', { price_cents: 999, pack_quantity: 45, quantity: 10 })), 222);
  assert.equal(stockValueCents(item('a', { price_cents: 999, pack_quantity: null, quantity: 2 })), 1998);
  assert.equal(stockValueCents(item('a', { price_cents: 999, pack_quantity: 0, quantity: 2 })), 1998);
  assert.equal(stockValueCents(item('a', { price_cents: 999, quantity: 0 })), 0);
  assert.equal(stockValueCents(item('a', { price_cents: 999, quantity: null })), null);
  assert.equal(stockValueCents(item('a', { price_cents: null, quantity: 5 })), null);
  assert.equal(stockValueCents(item('a', { price_cents: 0, quantity: 5 })), 0);
});

test('money shows the currency of the library', () => {
  assert.match(formatMoney(999, 'CAD'), /9[.,]99/);
  assert.match(formatMoney(123450, 'CAD'), /1\D?234[.,]50/);
  assert.match(formatMoney(0, 'CAD'), /0[.,]00/);
  assert.notEqual(formatMoney(999, 'CAD'), formatMoney(999, 'EUR'));
  assert.match(formatMoney(999, 'ZZZ'), /9[.,]99/);
});

test('the library own dollar shows as a bare symbol, not CA$', () => {
  // The symbol and its position depend on the locale, but "CA" never prefixes a Canadian dollar.
  assert.doesNotMatch(formatMoney(899, 'CAD'), /CA/);
  assert.doesNotMatch(formatUnitPrice(22.2, 'CAD'), /CA/);
  assert.match(formatMoney(899, 'CAD'), /^\D*8[.,]99\D*$/);
});

test('a unit price has two decimals from one dollar and up to four below it', () => {
  assert.match(formatUnitPrice(22.2, 'CAD'), /0[.,]222(?!\d)/);
  assert.doesNotMatch(formatUnitPrice(22.2, 'CAD'), /0[.,]2220/);
  assert.match(formatUnitPrice(999, 'CAD'), /9[.,]99(?!\d)/);
  assert.match(formatUnitPrice(100, 'CAD'), /1[.,]00(?!\d)/);
  assert.match(formatUnitPrice(50, 'CAD'), /0[.,]50(?!\d)/);
  assert.match(formatUnitPrice(0, 'CAD'), /0[.,]00(?!\d)/);
  assert.match(formatUnitPrice(1999.4, 'CAD'), /19[.,]99(?!\d)/);
  assert.match(formatUnitPrice(0.333333, 'CAD'), /0[.,]0033(?!\d)/);
});

test('Auto shows only columns that have a value and always shows the count', () => {
  assert.deepEqual(autoColumns([], 'category'), ['quantity']);
  assert.deepEqual(autoColumns([], 'all'), ['category', 'quantity']);
  const plain = [item('a', { brand: 'Acme', part_number: 'P1', notes: 'x', supplier: { title: 'S', url: '', details: '' } })];
  assert.deepEqual(autoColumns(plain, 'category'), ['quantity']);
  const rich = [item('a', { colour: ' ', dimensions: 'M3 x 8' }), item('b', { colour: 'red', pack_quantity: 1 })];
  assert.deepEqual(autoColumns(rich, 'category'), ['colour', 'dimensions', 'pack_quantity', 'quantity']);
  assert.deepEqual(autoColumns(rich, 'all'), ['category', 'colour', 'dimensions', 'pack_quantity', 'quantity']);
  // A pack of 0 is still a recorded value.
  assert.deepEqual(autoColumns([item('a', { pack_quantity: 0 })], 'category'), ['pack_quantity', 'quantity']);
});

test('Auto shows price only with a priced item and stock value only when one item has a price and a count', () => {
  assert.deepEqual(autoColumns([item('a', { quantity: 3 })], 'category'), ['quantity']);
  assert.deepEqual(autoColumns([item('a', { price_cents: 100 })], 'category'), ['quantity', 'price']);
  assert.deepEqual(autoColumns([item('a', { price_cents: 100 }), item('b', { quantity: 2 })], 'category'),
    ['quantity', 'price']);
  assert.deepEqual(autoColumns([item('a', { price_cents: 0, quantity: 0 })], 'category'),
    ['quantity', 'price', 'stock_value']);
  const all = [item('a', { colour: 'red', dimensions: 'd', pack_quantity: 2, price_cents: 5, quantity: 1 })];
  assert.deepEqual(autoColumns(all, 'all'),
    ['category', 'colour', 'dimensions', 'pack_quantity', 'quantity', 'price', 'stock_value']);
});

test('a stored config sets the columns and their order; null means Auto', () => {
  const items = [item('a', { colour: 'red' })];
  assert.deepEqual(visibleColumns(items, null, 'category'), ['colour', 'quantity']);
  assert.deepEqual(visibleColumns(items, ['brand', 'price', 'colour'], 'category'), ['brand', 'price', 'colour']);
  assert.deepEqual(visibleColumns(items, [], 'all'), []);
  assert.deepEqual(visibleColumns(items, ['name', 'bogus', 'price', 'price', 'toString'], 'all'), ['price']);
});

test('totals count unpriced items and sum only items that have a price and a count', () => {
  assert.deepEqual(viewTotals([]), { count: 0, withoutCount: 0, unpriced: 0, stockValueCents: 0, estimatedPrices: 0 });
  const items = [
    item('a', { price_cents: 999, pack_quantity: 45, quantity: 90 }),
    item('b', { price_cents: 500, quantity: 2 }),
    item('c', { quantity: 10 }),
    item('d', { price_cents: 700 }),
    item('e'),
    item('f', { price_cents: 0, quantity: 0 }),
  ];
  assert.deepEqual(viewTotals(items), { count: 6, withoutCount: 2, unpriced: 2, stockValueCents: 1998 + 1000, estimatedPrices: 0 });
});

test('the caption keeps brand and part number unless they have their own column', () => {
  assert.deepEqual(captionFields(['colour', 'quantity']), ['brand', 'part_number']);
  assert.deepEqual(captionFields(['brand', 'quantity']), ['part_number']);
  assert.deepEqual(captionFields(['part_number', 'brand']), []);
  assert.deepEqual(captionFields([]), ['brand', 'part_number']);
});

test('derived columns sort by their values and supplier by title', () => {
  const pack = item('a', { price_cents: 999, pack_quantity: 45, quantity: 90, supplier: { title: ' Acme ', url: '', details: '' } });
  assert.ok(Math.abs(columnDefs.unit_price.sortValue(pack) - 22.2) < 1e-9);
  assert.equal(columnDefs.stock_value.sortValue(pack), 1998);
  assert.equal(columnDefs.price.sortValue(pack), 999);
  assert.equal(columnDefs.supplier.sortValue(pack), 'Acme');
  assert.equal(columnDefs.supplier.sortValue(item('b')), null);
  assert.equal(columnDefs.brand.sortValue(item('b', { brand: '  ' })), null);
  assert.equal(columnDefs.price.sortValue(item('b')), null);
});

test('an estimated price needs a price and a source, and the totals count them', () => {
  const listing = 'listing:canadabolts.ca';
  assert.equal(isEstimatedPrice(item('a', { price_cents: 500, price_source: listing })), true);
  assert.equal(isEstimatedPrice(item('a', { price_cents: 0, price_source: listing })), true);
  assert.equal(isEstimatedPrice(item('a', { price_cents: 500, price_source: '' })), false);
  assert.equal(isEstimatedPrice(item('a', { price_cents: 500, price_source: '  ' })), false);
  assert.equal(isEstimatedPrice(item('a', { price_cents: null, price_source: listing })), false);
  assert.equal(isEstimatedPrice({ price_cents: 500 }), false);
  const items = [
    item('a', { price_cents: 500, quantity: 2, price_source: listing }),
    item('b', { price_cents: 700, quantity: 1 }),
    item('c', { price_cents: null, price_source: listing }),
    item('d', { price_cents: 100, quantity: 1, price_source: 'listing:3dprintingcanada.com' }),
  ];
  assert.equal(viewTotals(items).estimatedPrices, 2);
  assert.equal(viewTotals(items).stockValueCents, 1000 + 700 + 100);
});
