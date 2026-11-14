import test from 'node:test';
import assert from 'node:assert/strict';
import { sortItems, parseListSort, nextListSort, swatchColours } from '../src/components/workshopList.ts';

const item = (id, name, values = {}) =>
  ({ id, name, category: 'Fasteners', colour: '', quantity: null, pack_quantity: null, ...values });
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
  assert.deepEqual(swatchColours('dark grey'), ['darkgray']);
  assert.deepEqual(swatchColours('#a0B1C2'), ['#a0b1c2']);
  assert.deepEqual(swatchColours('Clear'), ['transparent']);
  for (const text of ['', 'Misty', 'Blue / unknown', 'url(https://example.com)', 'var(--bg)', 'inherit', '__proto__']) {
    assert.deepEqual(swatchColours(text), [], text);
  }
});
