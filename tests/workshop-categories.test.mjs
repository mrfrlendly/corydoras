import test from 'node:test';
import assert from 'node:assert/strict';
import {
  canDecrease, categoryChips, checkCurrency, findCategory, mergeTarget, mergeText, movedText, newCategoryError, pieceHint,
  overlaySelector, pickerSelector, renameError, shortcutFor, stepQuantity,
} from '../src/components/workshopActions.ts';

test('the stepper never goes below zero and treats an unknown count as 0', () => {
  assert.equal(stepQuantity(5, 1), 6);
  assert.equal(stepQuantity(5, -1), 4);
  assert.equal(stepQuantity(1, -1), 0);
  assert.equal(stepQuantity(0, -1), null);
  assert.equal(stepQuantity(0, 1), 1);
  assert.equal(stepQuantity(null, 1), 1);
  assert.equal(stepQuantity(undefined, 1), 1);
  assert.equal(stepQuantity(null, -1), 0);
  assert.equal(stepQuantity(3, -10), null);
  assert.equal(stepQuantity(Number.MAX_SAFE_INTEGER, 1), null);
});

test('the minus button is off at 0 but not at an unknown count', () => {
  assert.equal(canDecrease(0), false);
  assert.equal(canDecrease(1), true);
  assert.equal(canDecrease(null), true);
  assert.equal(canDecrease(undefined), true);
});

const key = (value, extra = {}) => ({ key: value, target: { tagName: 'BODY' }, ...extra });

test('/ and n are shortcuts only when nothing else owns the keys', () => {
  assert.equal(shortcutFor(key('/'), false), 'search');
  assert.equal(shortcutFor(key('n'), false), 'add');
  assert.equal(shortcutFor(key('x'), false), null);
  assert.equal(shortcutFor(key('N'), false), null);
  assert.equal(shortcutFor({ key: 'n' }, false), 'add');
});

test('shortcuts are ignored while typing, with modifiers, and when the list is not the view', () => {
  for (const tagName of ['INPUT', 'TEXTAREA', 'SELECT', 'input']) {
    assert.equal(shortcutFor(key('/', { target: { tagName } }), false), null, tagName);
  }
  assert.equal(shortcutFor(key('n', { target: { tagName: 'DIV', isContentEditable: true } }), false), null);
  assert.equal(shortcutFor(key('n', { target: { tagName: 'BUTTON' } }), false), 'add');
  for (const modifier of ['ctrlKey', 'metaKey', 'altKey']) {
    assert.equal(shortcutFor(key('n', { [modifier]: true }), false), null, modifier);
  }
  assert.equal(shortcutFor(key('n'), true), null);
  assert.equal(shortcutFor(key('/'), true), null);
  assert.equal(shortcutFor(key('n', { defaultPrevented: true }), false), null);
  assert.equal(shortcutFor(key('n', { isComposing: true }), false), null);
});

test('a currency code is three letters and is stored in upper case', () => {
  assert.deepEqual(checkCurrency('CAD'), { ok: true, code: 'CAD' });
  assert.deepEqual(checkCurrency(' usd '), { ok: true, code: 'USD' });
  for (const bad of ['', 'CA', 'CADX', 'C4D', '€', 'C D', '123', 'ÄÖÜ']) {
    const result = checkCurrency(bad);
    assert.equal(result.ok, false, bad);
    assert.match(result.error, /three-letter/);
  }
});

const categories = [{ name: 'Bolts', count: 3, columns: null, keywords: [] }, { name: 'Filament', count: 0, columns: ['colour'], keywords: [] }];

test('category names are found and checked without regard to letter case', () => {
  assert.equal(findCategory(categories, 'bolts')?.name, 'Bolts');
  assert.equal(findCategory(categories, ' FILAMENT ')?.name, 'Filament');
  assert.equal(findCategory(categories, 'Nuts'), undefined);
  assert.equal(newCategoryError(categories, 'Nuts'), '');
  assert.match(newCategoryError(categories, 'bolts'), /Bolts already exists/);
  assert.match(newCategoryError(categories, '   '), /Enter a category name/);
  assert.match(newCategoryError(categories, 'x'.repeat(121)), /120/);
});

test('a rename needs a different name, and a merge is a rename to another existing category', () => {
  assert.equal(renameError('Bolts', 'Screws'), '');
  assert.equal(renameError('Bolts', 'bolts'), '');
  assert.match(renameError('Bolts', 'Bolts'), /different name/);
  assert.match(renameError('Bolts', ' '), /Enter a category name/);
  assert.equal(mergeTarget(categories, 'Bolts', 'filament')?.name, 'Filament');
  assert.equal(mergeTarget(categories, 'Bolts', 'Screws'), undefined);
  // The same category in other letters is a change of spelling, not a merge.
  assert.equal(mergeTarget(categories, 'Bolts', 'bolts'), undefined);
});

test('the merge confirmation says how many items move and which column choice wins', () => {
  const text = mergeText('Screws', 3, 'Bolts');
  assert.match(text, /3 items will move from Screws to Bolts/);
  assert.match(text, /Bolts keeps its own column choice\. If Bolts is on Auto, it takes the column choice of Screws\./);
  assert.match(mergeText('Screws', 1, 'Bolts'), /1 item will move/);
  assert.match(mergeText('Screws', 0, 'Bolts'), /no items to move/);
  assert.equal(movedText(3, 'Bolts'), '3 items moved to Bolts.');
  assert.equal(movedText(1, 'Bolts'), '1 item moved to Bolts.');
  assert.equal(movedText(0, 'Bolts'), 'Renamed to Bolts.');
});

test('the price field shows the price of one piece when the price and the pack count are known', () => {
  assert.match(pieceHint('9.99', '45', 'CAD'), /^About \D*0[.,]222 per piece$/);
  assert.match(pieceHint('9.99', '1', 'CAD'), /^About \D*9[.,]99 per piece$/);
  assert.match(pieceHint('0', '10', 'CAD'), /^About \D*0[.,]00 per piece$/);
  assert.equal(pieceHint('', '45', 'CAD'), '');
  assert.equal(pieceHint('9.99', '', 'CAD'), '');
  assert.equal(pieceHint('9.99', '0', 'CAD'), '');
  assert.equal(pieceHint('abc', '45', 'CAD'), '');
  assert.equal(pieceHint('9.999', '45', 'CAD'), '');
});

test('shortcuts stay out of open dialogs and popovers and off the buttons of the column picker', () => {
  // A fake element: closest(selector) answers for the selectors that this element sits inside.
  const inside = (tagName, ...selectors) => ({ tagName, closest: selector => selectors.includes(selector) ? {} : null });
  assert.equal(shortcutFor(key('n', { target: inside('BUTTON', overlaySelector) }), false), null);
  assert.equal(shortcutFor(key('/', { target: inside('DIV', overlaySelector) }), false), null);
  for (const tagName of ['BUTTON', 'A', 'SUMMARY']) {
    assert.equal(shortcutFor(key('n', { target: inside(tagName, pickerSelector) }), false), null, tagName);
  }
  // Elsewhere the same elements keep the shortcut.
  assert.equal(shortcutFor(key('n', { target: inside('BUTTON') }), false), 'add');
  assert.equal(shortcutFor(key('/', { target: inside('A') }), false), 'search');
  assert.equal(shortcutFor(key('n', { target: inside('DIV', pickerSelector) }), false), 'add');
});

test('category chips merge spellings in any letter case and show the stored name', () => {
  const item = category => ({ category });
  const items = [item('bolts'), item('Bolts'), item('BOLTS'), item('Tape'), item('tape'), item('Filament')];
  // The stored name wins over the spellings that items use, and empty stored categories get a chip.
  assert.deepEqual(categoryChips(items, [{ name: 'Bolts' }, { name: 'Washers' }]), [
    { name: 'Bolts', count: 3 }, { name: 'Filament', count: 1 }, { name: 'Tape', count: 2 }, { name: 'Washers', count: 0 },
  ]);
  // Without a stored name the first spelling seen is shown.
  assert.deepEqual(categoryChips([item('tape'), item('Tape')], []), [{ name: 'tape', count: 2 }]);
  assert.deepEqual(categoryChips([], []), []);
});
