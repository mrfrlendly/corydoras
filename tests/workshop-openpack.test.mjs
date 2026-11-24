import test from 'node:test';
import assert from 'node:assert/strict';
import {
  autoColumns, columnDefs, formatAmount, fractionLeft, openPackText, stockValueCents, totalAmount, unitPriceCents, viewTotals,
} from '../src/components/workshopColumns.ts';
import {
  blankForm, chooseUnit, duplicateForm, editForm, formTotalAmount, fullFromSpecs, reviewLabel, saveDraft,
} from '../src/components/workshopForm.ts';
import { checkOpenAmount, pieceHint, stepQuantityItem } from '../src/components/workshopActions.ts';
import { sortItems } from '../src/components/workshopList.ts';

const item = (id, values = {}) => ({
  id, name: id, category: 'Filament', brand: '', part_number: '', colour: '', dimensions: '', quantity: null,
  pack_quantity: null, price_cents: null, measure_unit: '', measure_full: null, measure_left: null, notes: '', specs: [],
  supplier: null, photo_key: null, ...values,
});
const spool = (id, values = {}) => item(id, { quantity: 1, measure_unit: 'g', measure_full: 1000, measure_left: 100, ...values });
const spec = (value, unit, role = 'net_weight') => ({ role, name: '', value, unit, tolerance: '', source: 'label', evidence: '' });

test('the total amount counts whole packs, the opened one at what is left', () => {
  assert.equal(totalAmount(spool('a')), 100);
  assert.equal(totalAmount(spool('a', { quantity: 3, measure_left: 250 })), 2250);
  assert.equal(totalAmount(spool('a', { quantity: 3, measure_left: null })), 3000);
  assert.equal(totalAmount(spool('a', { quantity: 1, measure_left: null })), 1000);
  assert.equal(totalAmount(spool('a', { quantity: 2, measure_left: 0 })), 1000);
  assert.equal(totalAmount(spool('a', { quantity: null })), null);
  assert.equal(totalAmount(spool('a', { quantity: null, measure_left: null })), null);
  assert.equal(totalAmount(spool('a', { quantity: 0 })), 0);
  assert.equal(totalAmount(spool('a', { quantity: 0, measure_left: null })), 0);
  // Not measured, or no full pack: unknown.
  assert.equal(totalAmount(item('a', { quantity: 5 })), null);
  assert.equal(totalAmount(spool('a', { measure_full: null })), null);
  assert.equal(totalAmount(spool('a', { measure_unit: '' })), null);
  // An older item without the fields.
  const old = item('a', { quantity: 2 });
  for (const key of ['measure_unit', 'measure_full', 'measure_left']) delete old[key];
  assert.equal(totalAmount(old), null);
});

test('a measured item is priced per unit and its stock value follows the total amount', () => {
  const one = spool('a', { price_cents: 3000 });
  assert.equal(unitPriceCents(one), 3);
  assert.equal(stockValueCents(one), 300);
  assert.equal(stockValueCents(spool('a', { price_cents: 3000, quantity: 3, measure_left: 250 })), 6750);
  assert.equal(stockValueCents(spool('a', { price_cents: 3000, quantity: 3, measure_left: null })), 9000);
  assert.equal(stockValueCents(spool('a', { price_cents: 3000, quantity: 0 })), 0);
  assert.equal(stockValueCents(spool('a', { price_cents: 3000, quantity: null })), null);
  assert.equal(stockValueCents(spool('a', { price_cents: null })), null);
  // The pack count is ignored for a measured item.
  assert.equal(stockValueCents(spool('a', { price_cents: 3000, pack_quantity: 4 })), 300);
  // Multiplying first keeps the cents: 2499 cents per 1000 g, 333 g left.
  assert.equal(stockValueCents(spool('a', { price_cents: 2499, measure_left: 333 })), Math.round(333 * 2499 / 1000));
  // Counted parts keep the old rule, also with a full pack but no unit.
  assert.equal(stockValueCents(item('a', { price_cents: 999, pack_quantity: 45, quantity: 90 })), 1998);
  assert.equal(stockValueCents(item('a', { price_cents: 999, pack_quantity: 25, quantity: 6, measure_full: 1000 })), 240);
  assert.deepEqual(viewTotals([spool('a', { price_cents: 3000 }), item('b', { price_cents: 500, quantity: 2 })]),
    { count: 2, withoutCount: 0, unpriced: 0, stockValueCents: 1300, estimatedPrices: 0 });
});

test('amounts read as 1,100 g and 100 of 1000 g', () => {
  assert.match(formatAmount(1100, 'g'), /^1\D?100 g$/);
  assert.equal(formatAmount(250, 'mL'), '250 mL');
  assert.equal(openPackText(spool('a')), '100 of 1000 g');
  assert.equal(openPackText(spool('a', { measure_unit: 'mL', measure_left: 0 })), '0 of 1000 mL');
  assert.equal(openPackText(spool('a', { measure_left: null })), null);
  assert.equal(openPackText(item('a')), null);
});

test('the fraction left sorts the Open pack column, with missing last', () => {
  assert.equal(fractionLeft(spool('a')), 0.1);
  assert.equal(fractionLeft(spool('a', { measure_left: 0 })), 0);
  assert.equal(fractionLeft(spool('a', { measure_left: null })), null);
  assert.equal(fractionLeft(item('a')), null);
  assert.equal(columnDefs.open_amount.label, 'Open pack');
  assert.equal(columnDefs.open_amount.numeric, true);
  const items = [item('none'), spool('half', { measure_left: 500 }), spool('empty', { measure_left: 0 }),
    spool('tenth'), spool('small', { measure_full: 200, measure_left: 150 }), spool('unrecorded', { measure_left: null })];
  const order = direction => sortItems(items, { field: 'open_amount', direction }).map(x => x.id);
  assert.deepEqual(order('asc'), ['empty', 'tenth', 'half', 'small', 'none', 'unrecorded']);
  // Missing sorts last in both directions, as for the other columns.
  assert.deepEqual(order('desc'), ['small', 'half', 'tenth', 'empty', 'none', 'unrecorded']);
});

test('Auto shows Open pack when an item in the view has an amount left', () => {
  assert.deepEqual(autoColumns([spool('a')], 'category'), ['quantity', 'open_amount']);
  assert.deepEqual(autoColumns([spool('a', { measure_left: null })], 'category'), ['quantity']);
  assert.deepEqual(autoColumns([item('a'), spool('b', { measure_left: 0 })], 'category'), ['quantity', 'open_amount']);
  assert.deepEqual(autoColumns([spool('a', { price_cents: 100 })], 'category'), ['quantity', 'open_amount', 'price', 'stock_value']);
  assert.deepEqual(autoColumns([item('a', { quantity: 1 })], 'category'), ['quantity']);
});

test('the full pack comes from a printed net weight', () => {
  assert.equal(fullFromSpecs([spec('1', 'kg')], 'g'), 1000);
  assert.equal(fullFromSpecs([spec('750', 'g')], 'g'), 750);
  assert.equal(fullFromSpecs([spec('1.75', 'kg')], 'g'), 1750);
  assert.equal(fullFromSpecs([spec('1.1', 'kg')], 'g'), 1100);
  assert.equal(fullFromSpecs([spec('1 kg', '')], 'g'), 1000);
  assert.equal(fullFromSpecs([spec('1', 'KG')], 'g'), 1000);
  assert.equal(fullFromSpecs([spec('1,000', 'g')], 'g'), 1000);
  assert.equal(fullFromSpecs([spec('500', 'mL')], 'mL'), 500);
  assert.equal(fullFromSpecs([spec('1', 'L')], 'mL'), 1000);
  assert.equal(fullFromSpecs([spec('1', 'kg')], 'mL'), null);
  assert.equal(fullFromSpecs([spec('1', 'L')], 'g'), null);
  assert.equal(fullFromSpecs([spec('2.2', 'lb')], 'g'), null);
  assert.equal(fullFromSpecs([spec('0.5', 'g')], 'g'), null);
  assert.equal(fullFromSpecs([spec('0', 'kg')], 'g'), null);
  assert.equal(fullFromSpecs([spec('abc', 'kg')], 'g'), null);
  assert.equal(fullFromSpecs([spec('1', 'kg', 'length')], 'g'), null);
  assert.equal(fullFromSpecs([], 'g'), null);
  assert.equal(fullFromSpecs(undefined, 'g'), null);
  assert.equal(fullFromSpecs([spec('1', 'kg')], ''), null);
  // The first usable net weight wins over an earlier unusable one.
  assert.equal(fullFromSpecs([spec('2.2', 'lb'), spec('1', 'kg')], 'g'), 1000);
});

test('choosing a unit prefills the full pack from the label only when it is empty, and Pieces clears the amounts', () => {
  const form = { ...blankForm(), name: 'PLA', specs: [spec('1', 'kg')] };
  assert.equal(chooseUnit(form, 'g').measure_full, '1000');
  assert.equal(chooseUnit({ ...form, measure_full: '850' }, 'g').measure_full, '850');
  assert.equal(chooseUnit(form, 'm').measure_full, '');
  const set = { ...form, measure_unit: 'g', measure_full: '1000', measure_left: '100' };
  assert.deepEqual([chooseUnit(set, '').measure_unit, chooseUnit(set, '').measure_full, chooseUnit(set, '').measure_left], ['', '', '']);
  assert.equal(chooseUnit(set, 'mL').measure_left, '100');
});

test('a measured pack saves as whole numbers and round-trips through the form', () => {
  const form = { ...blankForm(), name: 'PLA', quantity: '1', measure_unit: 'g', measure_full: '1000', measure_left: '100' };
  const draft = saveDraft(form);
  assert.deepEqual([draft.measure_unit, draft.measure_full, draft.measure_left], ['g', 1000, 100]);
  const again = editForm(draft);
  assert.deepEqual([again.measure_unit, again.measure_full, again.measure_left], ['g', '1000', '100']);
  assert.equal(formTotalAmount(form), 100);
  assert.equal(formTotalAmount({ ...form, quantity: '3', measure_left: '250' }), 2250);
  assert.equal(formTotalAmount({ ...form, quantity: '' }), null);
  assert.equal(formTotalAmount({ ...form, measure_left: 'x' }), null);
  // Full pack only, left unrecorded.
  const unopened = saveDraft({ ...form, measure_left: '' });
  assert.deepEqual([unopened.measure_full, unopened.measure_left], [1000, null]);
  // Counted pieces carry no measure.
  const pieces = saveDraft({ ...blankForm(), name: 'Magnets', quantity: '6', pack_quantity: '25' });
  assert.deepEqual([pieces.measure_unit, pieces.measure_full, pieces.measure_left], ['', null, null]);
});

test('measured pack errors are plain and agree with the library', () => {
  const base = { ...blankForm(), name: 'PLA', measure_unit: 'g', measure_full: '1000', measure_left: '100' };
  assert.throws(() => saveDraft({ ...base, measure_left: '1001' }), /cannot be more than the full pack of 1000 g/);
  assert.doesNotThrow(() => saveDraft({ ...base, measure_left: '1000' }));
  assert.doesNotThrow(() => saveDraft({ ...base, measure_left: '0' }));
  assert.throws(() => saveDraft({ ...base, measure_unit: '' }), /Choose a unit/);
  assert.throws(() => saveDraft({ ...base, measure_unit: '', measure_left: '' }), /Choose a unit/);
  assert.throws(() => saveDraft({ ...base, measure_unit: '', measure_full: '', measure_left: '5' }), /Choose a unit/);
  assert.throws(() => saveDraft({ ...base, measure_full: '', measure_left: '5' }), /full pack amount/);
  assert.doesNotThrow(() => saveDraft({ ...base, measure_full: '', measure_left: '' }));
  assert.throws(() => saveDraft({ ...base, measure_unit: 'kg' }), /Choose Pieces, g, mL or m/);
  for (const bad of ['1.5', '-1', '1e3', '9007199254740992', 'abc']) {
    assert.throws(() => saveDraft({ ...base, measure_left: bad }), /whole number/, bad);
    assert.throws(() => saveDraft({ ...base, measure_full: bad }), /whole number/, bad);
  }
  assert.throws(() => saveDraft({ ...base, measure_full: '0' }), /from 1 to/);
});

test('a card stored by an older session has no measure fields and still loads and saves', () => {
  const old = { ...blankForm(), name: 'x', quantity: '2' };
  for (const key of ['measure_unit', 'measure_full', 'measure_left']) delete old[key];
  const draft = saveDraft(old);
  assert.deepEqual([draft.measure_unit, draft.measure_full, draft.measure_left], ['', null, null]);
  const oldDraft = { ...draft };
  for (const key of ['measure_unit', 'measure_full', 'measure_left']) delete oldDraft[key];
  const form = editForm(oldDraft);
  assert.deepEqual([form.measure_unit, form.measure_full, form.measure_left], ['', '', '']);
  assert.equal(formTotalAmount(old), null);
  assert.equal(reviewLabel(old, oldDraft).measure_unit, '');
});

test('duplicating a measured item keeps the unit and the full pack and clears the count and the opened pack', () => {
  const copy = duplicateForm(spool('a', { price_cents: 3000 }));
  assert.deepEqual([copy.measure_unit, copy.measure_full, copy.measure_left, copy.quantity], ['g', '1000', '', '']);
  assert.equal(copy.price, '30.00');
});

test('a label review takes a measured pack only when the form has none', () => {
  const label = { ...saveDraft({ ...blankForm(), name: 'PLA' }), measure_unit: 'g', measure_full: 1000, measure_left: null };
  const taken = reviewLabel(blankForm(), label);
  assert.deepEqual([taken.measure_unit, taken.measure_full, taken.measure_left], ['g', '1000', '']);
  const kept = reviewLabel({ ...blankForm(), measure_unit: 'mL', measure_full: '500' }, label);
  assert.deepEqual([kept.measure_unit, kept.measure_full], ['mL', '500']);
});

test('the price hint is per unit for a measured pack', () => {
  assert.match(pieceHint('30', '', 'CAD', { unit: 'g', full: '1000' }), /0[.,]03.* per g$/);
  assert.equal(pieceHint('30', '', 'CAD', { unit: 'g', full: '' }), '');
  assert.equal(pieceHint('30', '', 'CAD', { unit: 'g', full: '0' }), '');
  assert.match(pieceHint('10', '4', 'CAD'), /per piece$/);
  assert.match(pieceHint('10', '4', 'CAD', { unit: '', full: '1000' }), /per piece$/);
});

test('the list edit accepts a whole amount up to the full pack and a blank clears it', () => {
  assert.deepEqual(checkOpenAmount('100', 1000, 'g'), { ok: true, left: 100 });
  assert.deepEqual(checkOpenAmount(' 0 ', 1000, 'g'), { ok: true, left: 0 });
  assert.deepEqual(checkOpenAmount('1000', 1000, 'g'), { ok: true, left: 1000 });
  assert.deepEqual(checkOpenAmount('', 1000, 'g'), { ok: true, left: null });
  assert.deepEqual(checkOpenAmount('  ', 1000, 'g'), { ok: true, left: null });
  for (const bad of ['1001', '-1', '1.5', 'abc', '1e2']) {
    const check = checkOpenAmount(bad, 1000, 'g');
    assert.equal(check.ok, false, bad);
    assert.match(check.error, /0 to 1000 g/);
  }
});

test('stepping On hand down assumes the opened pack was used up; up or unchanged keeps it', () => {
  const left = row => [row.quantity, row.measure_left];
  assert.deepEqual(left(stepQuantityItem(spool('a', { quantity: 2, measure_left: 100 }), -1)), [1, null]);
  assert.equal(totalAmount(stepQuantityItem(spool('a', { quantity: 2, measure_left: 100 }), -1)), 1000);
  assert.deepEqual(left(stepQuantityItem(spool('a', { quantity: 1, measure_left: 100 }), 1)), [2, 100]);
  assert.deepEqual(left(stepQuantityItem(spool('a', { quantity: 1, measure_left: 100 }), 0)), [1, 100]);
  assert.deepEqual(left(stepQuantityItem(spool('a', { quantity: 1, measure_left: 100 }), -1)), [0, null]);
  assert.deepEqual(left(stepQuantityItem(spool('a', { quantity: null, measure_left: 100 }), 1)), [1, 100]);
  assert.deepEqual(left(stepQuantityItem(spool('a', { quantity: null, measure_left: 100 }), -1)), [0, null]);
  assert.deepEqual(left(stepQuantityItem(spool('a', { quantity: 3, measure_left: null }), -1)), [2, null]);
  assert.equal(stepQuantityItem(spool('a', { quantity: 0, measure_left: null }), -1), null);
  // Only the count and the open amount change.
  const before = spool('a', { quantity: 2, price_cents: 3000 });
  assert.deepEqual(stepQuantityItem(before, -1), { ...before, quantity: 1, measure_left: null });
});

test('a total that is not a safe integer is unknown, as in the record', () => {
  assert.equal(totalAmount(spool('a', { quantity: Number.MAX_SAFE_INTEGER, measure_full: 1000, measure_left: null })), null);
  assert.equal(totalAmount(spool('a', { quantity: Number.MAX_SAFE_INTEGER, measure_full: 1000, measure_left: 5 })), null);
  assert.equal(totalAmount(spool('a', { quantity: 9007199254740, measure_full: 1000, measure_left: null })), 9007199254740000);
});
