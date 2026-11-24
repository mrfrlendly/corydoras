import test from 'node:test';
import assert from 'node:assert/strict';
import { sortItems, parseListSort, nextListSort, swatchColours, effectiveSort, listColumns } from '../src/components/workshopList.ts';

const item = (id, name, values = {}) =>
  ({ id, name, category: 'Fasteners', colour: '', quantity: null, pack_quantity: null, price_cents: null, ...values });
const order = (items, field, direction) => sortItems(items, { field, direction }).map(i => i.id);

test('the default order is natural, with stable tie breaks and no input changes', () => {
  const items = [item('b','M3 bolt'), item('c','M10 bolt'), item('a','m3 BOLT')];
  assert.deepEqual(order(items, 'name', 'asc'), ['a','b','c']);
  assert.deepEqual(order(items, 'name', 'desc'), ['c','a','b']);
  assert.deepEqual(items.map(i => i.id), ['b','c','a']);
});

test('unknown counts stay last in both directions and zero is a stock value', () => {
  const items = [item('a','Pins'), item('b','Washers',{quantity:0,pack_quantity:20}),
    item('c','Screws',{quantity:10,pack_quantity:2})];
  assert.deepEqual(order(items, 'quantity', 'asc'), ['b','c','a']);
  assert.deepEqual(order(items, 'quantity', 'desc'), ['c','b','a']);
  assert.deepEqual(order(items, 'pack_quantity', 'asc'), ['c','b','a']);
});

test('colour and category use natural labels and keep blanks last', () => {
  const items = [item('a','M10 nut',{colour:'red',category:'Type 10'}),item('b','M3 nut',{colour:' Red ',category:'Type 2'}),
    item('c','Washer',{colour:'Blue',category:' '}),item('d','Bolt',{colour:'',category:'Type 2'})];
  assert.deepEqual(order(items, 'colour', 'asc'), ['c','b','a','d']);
  assert.deepEqual(order(items, 'colour', 'desc'), ['b','a','c','d']);
  assert.deepEqual(order(items, 'category', 'asc'), ['d','b','a','c']);
  assert.deepEqual(order(items, 'category', 'desc'), ['a','d','b','c']);
});

test('the saved sort has a safe default and each column can reverse', () => {
  const first = { field:'name', direction:'asc' };
  for (const value of [null,'','not valid','__proto__:asc','colour:sideways','colour:asc:more']) {
    assert.deepEqual(parseListSort(value), first);
  }
  assert.deepEqual(parseListSort('quantity:desc'), { field:'quantity', direction:'desc' });
  assert.deepEqual(nextListSort(first,'name'), { field:'name', direction:'desc' });
  assert.deepEqual(nextListSort(first,'colour'), { field:'colour', direction:'asc' });
});

test('swatches use known colour names or literal hex values and keep compound colours distinct', () => {
  assert.deepEqual(swatchColours(' BLUE / Black '), ['blue','black']);
  // Single words keep their CSS colour name; a phrase is computed by the engine, so Dark Grey is darker than grey.
  assert.deepEqual(swatchColours('grey'), ['gray']);
  assert.deepEqual(swatchColours('dark grey'), ['#5a5b5d']);
  assert.deepEqual(swatchColours('Dark Gray'), ['#5a5b5d']);
  assert.deepEqual(swatchColours('darkgrey'), ['darkgray']);
  assert.deepEqual(swatchColours('Light Blue'), ['#75ade4']);
  assert.deepEqual(swatchColours('Olive Green / Black'), ['#6b8e23', 'black']);
  assert.deepEqual(swatchColours('#a0B1C2'), ['#a0b1c2']);
  assert.deepEqual(swatchColours('Clear'), ['transparent']);
  for (const text of ['', 'Misty Grey', 'Blue / unknown', 'url(https://example.com)', 'var(--bg)', 'inherit', '__proto__']) {
    assert.deepEqual(swatchColours(text), [], text);
  }
});

test('shades and prefixed words get a badge from the shared dictionary', () => {
  // "Misty" alone was a documented limitation (no swatch); it is a shade of blue now.
  assert.deepEqual(swatchColours('Misty'), ['#9db8cc']);
  assert.deepEqual(swatchColours('Titan Grey'), ['#8d9094']);
  assert.deepEqual(swatchColours('Dark Brown'), ['#4c352e']);
  assert.deepEqual(swatchColours('Misty Blue / Black'), ['#9db8cc', 'black']);
  assert.deepEqual(swatchColours('Translucent'), ['transparent']);
  assert.deepEqual(swatchColours('Prairie Rose'), ['#e08aa3']);
});

test('the list can sort by price, unit price, stock value and supplier, with missing values last', () => {
  const items = [
    item('a', 'No price', { quantity: 5 }),
    item('b', 'Pricey', { price_cents: 2000, pack_quantity: 1, quantity: 1 }),
    item('c', 'Cheap pack', { price_cents: 999, pack_quantity: 45, quantity: 90, supplier: { title: 'Zed', url: '', details: '' } }),
    item('d', 'Free', { price_cents: 0, quantity: 3, supplier: { title: 'Acme', url: '', details: '' } }),
  ];
  assert.deepEqual(order(items, 'price', 'asc'), ['d', 'c', 'b', 'a']);
  assert.deepEqual(order(items, 'price', 'desc'), ['b', 'c', 'd', 'a']);
  assert.deepEqual(order(items, 'unit_price', 'asc'), ['d', 'c', 'b', 'a']);
  // Stock value: Free 0, Cheap pack 90 x 22.2 = 1998, Pricey 2000; No price has no value and stays last.
  assert.deepEqual(order(items, 'stock_value', 'asc'), ['d', 'c', 'b', 'a']);
  assert.deepEqual(order(items, 'stock_value', 'desc'), ['b', 'c', 'd', 'a']);
  assert.deepEqual(order(items, 'supplier', 'asc'), ['d', 'c', 'a', 'b']);
});

test('a saved sort may name any column and rejects unknown ones', () => {
  assert.deepEqual(parseListSort('price:desc'), { field: 'price', direction: 'desc' });
  assert.deepEqual(parseListSort('stock_value:asc'), { field: 'stock_value', direction: 'asc' });
  for (const value of ['toString:asc', 'constructor:desc', 'price_cents:asc']) {
    assert.deepEqual(parseListSort(value), { field: 'name', direction: 'asc' });
  }
});

test('a sort on a hidden column displays by name and leaves the preference alone', () => {
  const stored = { field: 'price', direction: 'desc' };
  assert.deepEqual(effectiveSort(stored, ['colour', 'quantity']), { field: 'name', direction: 'asc' });
  assert.deepEqual(stored, { field: 'price', direction: 'desc' });
  assert.equal(effectiveSort(stored, ['quantity', 'price']), stored);
  const byName = { field: 'name', direction: 'desc' };
  assert.equal(effectiveSort(byName, []), byName);
});

test('the fixed list columns keep their labels', () => {
  assert.deepEqual(listColumns.map(c => [c.field, c.label]), [['name', 'Item'], ['category', 'Category'],
    ['colour', 'Colour'], ['quantity', 'On hand'], ['pack_quantity', 'Per pack']]);
});

test('unit price sorts measured items after unmeasured ones in both directions, missing prices last', () => {
  const spool = { measure_unit: 'g', measure_full: 1000, measure_left: null };
  const items = [
    item('a', 'Filament', { price_cents: 3000, quantity: 1, ...spool }),
    item('b', 'Pricey', { price_cents: 2000, pack_quantity: 1, quantity: 1 }),
    item('c', 'No price', { quantity: 1 }),
    item('d', 'Cheap pack', { price_cents: 999, pack_quantity: 45, quantity: 1 }),
    item('e', 'Cheap resin', { price_cents: 100, quantity: 1, ...spool, measure_unit: 'mL' }),
    item('f', 'Unpriced spool', { quantity: 1, ...spool }),
  ];
  // Filament is 3 cents per g, Cheap resin 0.1 cent per mL: lower than any piece price, yet they come after the pieces.
  assert.deepEqual(order(items, 'unit_price', 'asc'), ['d', 'b', 'e', 'a', 'c', 'f']);
  assert.deepEqual(order(items, 'unit_price', 'desc'), ['b', 'd', 'a', 'e', 'c', 'f']);
});
