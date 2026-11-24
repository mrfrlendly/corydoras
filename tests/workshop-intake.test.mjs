import test from 'node:test';
import assert from 'node:assert/strict';
import {
  categoryForSave, changeCount, columnTick, defaultTicks, fieldInput, fillFromSuggestions, filterRows, hasOtherCurrency, hostOf, isGenericCategory,
  intakeBase, keywordsFromText, keywordsSummary, keywordsText, listingPrice, ownedFields, priceFill, setColumnTicks, shouldAutoFill,
  suggestionChanges, tickKey, ticksAfterReload, unambiguousListingPrice,
} from '../src/components/workshopIntake.ts';
import { blankForm, saveDraft } from '../src/components/workshopForm.ts';

const categories = ['Filament', 'Adhesives', 'Hardware', 'Other', 'Bolts'];
const touchedNone = { category: false, colour: false };

test('generic categories: empty, Other, Materials, Hardware, Consumables or a name the library does not have', () => {
  for (const name of ['', '  ', 'Other', 'other', 'Hardware', 'HARDWARE', 'Materials', 'Consumables']) {
    assert.equal(isGenericCategory(name, categories), true, name);
  }
  // Materials is generic even when the library has no such category; a name that is not in the library is generic too.
  assert.equal(isGenericCategory('Fasteners', categories), true);
  assert.equal(isGenericCategory('Filament', categories), false);
  assert.equal(isGenericCategory(' filament ', categories), false);
  assert.equal(isGenericCategory('Bolts', categories), false);
  assert.equal(isGenericCategory('Filament', []), true);
});

test('shouldAutoFill: colour only when empty, category only when generic, neither after the designer touched it', () => {
  assert.equal(shouldAutoFill('colour', '', false, categories), true);
  assert.equal(shouldAutoFill('colour', '   ', false, categories), true);
  assert.equal(shouldAutoFill('colour', 'Red', false, categories), false);
  assert.equal(shouldAutoFill('colour', '', true, categories), false);
  assert.equal(shouldAutoFill('category', 'Other', false, categories), true);
  assert.equal(shouldAutoFill('category', 'Bolts', false, categories), false);
  assert.equal(shouldAutoFill('category', 'Other', true, categories), false);
  assert.equal(shouldAutoFill('category', '', true, categories), false);
});

test('fill-only-empty: a suggestion sets only the empty colour and the generic category', () => {
  const suggestions = { category: { name: 'Filament', reason: 'name says "filament"' }, colour: 'Titan Grey' };
  const filled = fillFromSuggestions({ category: 'Other', colour: '' }, suggestions, touchedNone, categories);
  assert.deepEqual(filled.patch, { category: 'Filament', colour: 'Titan Grey' });
  assert.deepEqual(filled.categoryNote, { previous: 'Other', text: 'Category set to Filament: name says "filament"' });
  assert.deepEqual(filled.colourNote, { previous: '', text: 'Colour set to Titan Grey: from the name' });
  assert.deepEqual(filled.auto, { category: { value: 'Filament', previous: 'Other' }, colour: 'Titan Grey' });

  // A colour the designer has stays, a category that is not generic stays.
  const kept = fillFromSuggestions({ category: 'Bolts', colour: 'Red' }, suggestions, touchedNone, categories);
  assert.deepEqual(kept.patch, {});
  assert.equal(kept.categoryNote, undefined);

  // Typed or undone fields are not filled again.
  const touched = fillFromSuggestions({ category: 'Other', colour: '' }, suggestions, { category: true, colour: true }, categories);
  assert.deepEqual(touched.patch, {});

  // The same category again is no change and no note.
  const same = fillFromSuggestions({ category: 'filament', colour: '' }, { category: { name: 'Filament', reason: 'x' }, colour: null }, touchedNone, categories);
  assert.deepEqual(same.patch, {});
  assert.equal(same.categoryNote, undefined);

  // No suggestion: nothing changes. A blank colour is no suggestion.
  assert.deepEqual(fillFromSuggestions({ category: '', colour: '' }, { category: null, colour: '  ' }, touchedNone, categories).patch, {});
});

test('the note says only the category when the rule gave no reason', () => {
  const result = fillFromSuggestions({ category: '', colour: '' }, { category: { name: 'Bolts', reason: '' }, colour: null }, touchedNone, categories);
  assert.equal(result.categoryNote.text, 'Category set to Bolts');
  assert.equal(result.categoryNote.previous, '');
});

test('fieldInput sends the name, brand, notes and dimensions', () => {
  const form = { name: ' Gorilla Glue Sticks ', brand: 'Gorilla', notes: 'hot glue', dimensions: 'Length 20 mm', category: 'Other', colour: '' };
  assert.deepEqual(fieldInput(form), {
    name: 'Gorilla Glue Sticks', brand: 'Gorilla', description: 'hot glue\nLength 20 mm', category: 'Other', colour: '',
  });
});

test('a chosen listing sends only its title and the item brand, never the supplier text or the item notes', () => {
  const form = { name: 'Gorilla Glue Sticks', brand: 'Gorilla', notes: 'my notes', dimensions: 'Length 20 mm', category: 'Other', colour: '' };
  const input = fieldInput(form, { title: ' Glue sticks 12 pack ', details: 'Supplier text about tape and red' });
  assert.deepEqual(input, { name: 'Glue sticks 12 pack', brand: 'Gorilla', description: '', category: 'Other', colour: '' });
  assert.ok(!JSON.stringify(input).includes('Supplier text'));
  assert.ok(fieldInput(form, { title: 'x'.repeat(5000) }).name.length <= 512);
});

test('hostOf reads the host of a URL and nothing else', () => {
  assert.equal(hostOf('https://CanadaBolts.ca/products/m3?variant=4'), 'canadabolts.ca');
  assert.equal(hostOf('https://www.thepihut.com/products/x'), 'www.thepihut.com');
  assert.equal(hostOf('not a url'), '');
  assert.equal(hostOf(''), '');
});

const listing = (url, cents, currency) => ({ title: 't', url, details: '', price_cents: cents, currency });

test('a listing price fills only in the library currency', () => {
  assert.deepEqual(listingPrice(listing('https://canadabolts.ca/products/a', 1299, 'CAD'), 'CAD'),
    { price: '12.99', price_source: 'listing:canadabolts.ca' });
  assert.deepEqual(listingPrice(listing('https://canadabolts.ca/products/a', 1299, 'cad'), 'CAD'),
    { price: '12.99', price_source: 'listing:canadabolts.ca' });
  assert.equal(listingPrice(listing('https://thepihut.com/products/a', 1299, 'GBP'), 'CAD'), null);
  // An unknown currency never fills.
  assert.equal(listingPrice(listing('https://x.ca/products/a', 1299, ''), 'CAD'), null);
  assert.equal(listingPrice({ title: 't', url: 'https://x.ca/p', details: '' }, 'CAD'), null);
  assert.equal(listingPrice(listing('https://x.ca/p', null, 'CAD'), 'CAD'), null);
  assert.equal(listingPrice(listing('https://x.ca/p', 5, 'CAD'), 'CAD').price, '0.05');
  // A price of 0 or less is not a price: it never fills.
  assert.equal(listingPrice(listing('https://x.ca/p', 0, 'CAD'), 'CAD'), null);
  assert.equal(listingPrice(listing('https://x.ca/p', -100, 'CAD'), 'CAD'), null);
  assert.equal(listingPrice(listing('not a url', 100, 'CAD'), 'CAD'), null);
});

test('a price in another or unknown currency is flagged for the card note', () => {
  assert.equal(hasOtherCurrency(listing('https://a/p', 100, 'GBP'), 'CAD'), true);
  assert.equal(hasOtherCurrency(listing('https://a/p', 100, ''), 'CAD'), true);
  assert.equal(hasOtherCurrency(listing('https://a/p', 100, 'CAD'), 'CAD'), false);
  assert.equal(hasOtherCurrency(listing('https://a/p', null, 'GBP'), 'CAD'), false);
});

test('after a search the price fills only when the first product has exactly one price in the library currency', () => {
  const a = 'https://canadabolts.ca/products/m3-screw';
  // One product, three variants with one price.
  assert.deepEqual(unambiguousListingPrice([listing(`${a}?variant=1`, 899, 'CAD'), listing(`${a}?variant=2`, 899, 'CAD'),
    listing(`${a}?variant=3`, 899, 'CAD')], 'CAD'), { price: '8.99', price_source: 'listing:canadabolts.ca' });
  // Variants with different prices are ambiguous.
  assert.equal(unambiguousListingPrice([listing(`${a}?variant=1`, 899, 'CAD'), listing(`${a}?variant=2`, 1099, 'CAD')], 'CAD'), null);
  // A second product with another price does not matter.
  assert.equal(unambiguousListingPrice([listing(a, 899, 'CAD'), listing('https://canadabolts.ca/products/other', 2500, 'CAD')], 'CAD')?.price, '8.99');
  // A first product in another currency gives nothing, and the next product is not used instead.
  assert.equal(unambiguousListingPrice([listing('https://thepihut.com/products/x', 599, 'GBP'), listing('https://canadabolts.ca/products/y', 500, 'CAD')], 'CAD'), null);
  // No listing, no price.
  assert.equal(unambiguousListingPrice([], 'CAD'), null);
  assert.equal(unambiguousListingPrice([{ title: 't', url: a, details: '' }], 'CAD'), null);
  // A variant without a price does not make the others ambiguous.
  assert.equal(unambiguousListingPrice([listing(`${a}?variant=1`, 899, 'CAD'), { title: 't', url: `${a}?variant=2`, details: '' }], 'CAD')?.price, '8.99');
});

test('a price is filled only when the price field is empty', () => {
  const price = { price: '8.99', price_source: 'listing:canadabolts.ca' };
  assert.deepEqual(priceFill({ price: '' }, price), price);
  assert.deepEqual(priceFill({ price: '  ' }, price), price);
  assert.equal(priceFill({ price: '7.50' }, price), null);
  assert.equal(priceFill({ price: '' }, null), null);
});

const rows = [
  { id: 'a', name: 'Gorilla Glue', category_from: 'Other', category_to: 'Adhesives', category_reason: 'name says "glue"', colour_to: null, colour_reason: '' },
  { id: 'b', name: 'Overture PETG Titan Grey', category_from: 'Materials', category_to: 'Filament', category_reason: 'name says "petg"', colour_to: 'Titan Grey', colour_reason: 'name says "Titan Grey"' },
  { id: 'c', name: 'Blue tape', category_from: 'Tape', category_to: null, category_reason: '', colour_to: 'Blue', colour_reason: 'name says "Blue"' },
];

test('Review suggestions starts with the category changes ticked and no colour', () => {
  assert.deepEqual([...defaultTicks(rows)].sort(), ['category:a', 'category:b']);
});

test('changes hold exactly the ticked fields and leave out the rest', () => {
  const ticked = new Set([tickKey('category', 'a'), tickKey('colour', 'b'), tickKey('colour', 'c')]);
  assert.deepEqual(suggestionChanges(rows, ticked), [
    { id: 'a', category: 'Adhesives' },
    { id: 'b', colour: 'Titan Grey' },
    { id: 'c', colour: 'Blue' },
  ]);
  const a = suggestionChanges(rows, ticked)[0];
  assert.equal('colour' in a, false);
  // A row with nothing ticked is not sent. A ticked box of a field the row does not offer is ignored.
  assert.deepEqual(suggestionChanges(rows, new Set([tickKey('category', 'c')])), []);
  assert.deepEqual(suggestionChanges(rows, new Set()), []);
});

test('the filter chip limits what Apply sends to what the designer sees', () => {
  const ticked = new Set([tickKey('category', 'a'), tickKey('category', 'b'), tickKey('colour', 'b'), tickKey('colour', 'c')]);
  assert.deepEqual(suggestionChanges(rows, ticked, 'category'), [{ id: 'a', category: 'Adhesives' }, { id: 'b', category: 'Filament' }]);
  assert.deepEqual(suggestionChanges(rows, ticked, 'colour'), [{ id: 'b', colour: 'Titan Grey' }, { id: 'c', colour: 'Blue' }]);
  assert.deepEqual(filterRows(rows, 'colour').map(row => row.id), ['b', 'c']);
  assert.deepEqual(filterRows(rows, 'category').map(row => row.id), ['a', 'b']);
  assert.equal(filterRows(rows, 'all').length, 3);
  assert.equal(changeCount(suggestionChanges(rows, ticked)), 4);
  assert.equal(changeCount([]), 0);
});

test('Select all and none work on one column of the rows in view', () => {
  const none = new Set();
  assert.equal(columnTick(rows, none, 'category'), 'none');
  const all = setColumnTicks(rows, none, 'category', true);
  assert.deepEqual([...all].sort(), ['category:a', 'category:b']);
  assert.equal(columnTick(rows, all, 'category'), 'all');
  assert.equal(columnTick(rows, new Set(['category:a']), 'category'), 'some');
  // Select none leaves the other column and rows out of view alone.
  const mixed = new Set(['category:a', 'category:b', 'colour:b']);
  assert.deepEqual([...setColumnTicks(rows.slice(0, 1), mixed, 'category', false)].sort(), ['category:b', 'colour:b']);
  assert.equal(columnTick([], mixed, 'colour'), 'none');
});

test('after a reload the ticks of rows that stay are kept and new rows start as default', () => {
  const next = rows.filter(row => row.id !== 'a').concat([{ ...rows[0], id: 'd', colour_to: 'Red' }]);
  const ticked = new Set(['category:a', 'colour:b']);
  assert.deepEqual([...ticksAfterReload(rows, next, ticked)].sort(), ['category:d', 'colour:b']);
});

test('words: split at commas and line breaks, trimmed, empty ones dropped, each word once', () => {
  assert.deepEqual(keywordsFromText('glue, epoxy ,  Super   glue,,\nEpoxy,  GLUE '), ['glue', 'epoxy', 'Super glue']);
  assert.deepEqual(keywordsFromText(''), []);
  assert.deepEqual(keywordsFromText(' , ,'), []);
  assert.deepEqual(keywordsFromText('heat-set'), ['heat-set']);
  assert.equal(keywordsText(['a', 'b']), 'a, b');
  assert.equal(keywordsSummary([]), '');
  assert.equal(keywordsSummary(['glue']), 'glue');
  assert.equal(keywordsSummary(['glue', 'epoxy']), 'glue, epoxy');
  assert.equal(keywordsSummary(['glue', 'epoxy', 'cement', 'resin']), 'glue, epoxy +2 more');
});

test('a listing of price 0 never fills, alone or among variants', () => {
  const a = 'https://canadabolts.ca/products/m3';
  assert.equal(unambiguousListingPrice([listing(`${a}?variant=1`, 0, 'CAD')], 'CAD'), null);
  assert.equal(unambiguousListingPrice([listing(`${a}?variant=1`, 0, 'CAD'), listing(`${a}?variant=2`, 899, 'CAD')], 'CAD')?.price, '8.99');
});

test('an automatic fill follows later words until the designer types in the field', () => {
  const none = { category: false, colour: false };
  // "Nut" fills Nuts and "Red" fills Red.
  const first = fillFromSuggestions({ category: '', colour: '' }, { category: { name: 'Nuts', reason: 'name says "nut"' }, colour: 'Red' }, none, categories);
  assert.deepEqual(first.patch, { category: 'Nuts', colour: 'Red' });
  // The designer kept typing: "Nutsert tool" is Tools and "Reducer" has no colour. The fill is replaced and cleared.
  let form = { category: 'Nuts', colour: 'Red' };
  const second = fillFromSuggestions(form, { category: { name: 'Tools', reason: 'name says "tool"' }, colour: null }, none, categories, first.auto);
  assert.deepEqual(second.patch, { category: 'Tools', colour: '' });
  assert.equal(second.categoryNote.previous, '');
  assert.equal(second.categoryNote.text, 'Category set to Tools: name says "tool"');
  assert.equal(second.colourNote, null);
  assert.deepEqual(second.auto, { category: { value: 'Tools', previous: '' }, colour: null });
  // No category any more: the field goes back to what it was before the first fill.
  form = { category: 'Tools', colour: '' };
  const third = fillFromSuggestions(form, { category: null, colour: null }, none, categories, second.auto);
  assert.deepEqual(third.patch, { category: '' });
  assert.equal(third.categoryNote, null);
  assert.equal(third.auto.category, null);
  // The same answer changes nothing and keeps the note.
  const same = fillFromSuggestions({ category: 'Nuts', colour: 'Red' }, { category: { name: 'nuts', reason: 'r' }, colour: 'red' }, none, categories, first.auto);
  assert.deepEqual(same.patch, {});
  assert.equal(same.categoryNote, undefined);
  assert.equal(same.colourNote, undefined);
});

test('a field the designer typed in or undid, or changed from the fill, is never replaced', () => {
  const auto = { category: { value: 'Nuts', previous: 'Other' }, colour: 'Red' };
  const suggestions = { category: { name: 'Tools', reason: 'r' }, colour: 'Blue' };
  // Typed or undone: touched.
  assert.deepEqual(fillFromSuggestions({ category: 'Nuts', colour: 'Red' }, suggestions, { category: true, colour: true }, categories, auto).patch, {});
  // A value that is no longer the automatic one is the designer's, whatever the flags say.
  assert.deepEqual(fillFromSuggestions({ category: 'Bolts', colour: 'Green' }, suggestions, { category: false, colour: false }, categories, auto).patch, {});
});

test('the request sees an untouched automatic fill as empty so that the rules can answer again', () => {
  const none = { category: false, colour: false };
  const auto = { category: { value: 'Nuts', previous: 'Other' }, colour: 'Red' };
  const form = { name: 'Reducer', brand: '', notes: '', dimensions: '', category: 'Nuts', colour: 'Red' };
  assert.deepEqual(ownedFields(form, auto, none), { category: true, colour: true });
  const base = intakeBase(form, auto, none);
  assert.equal(base.category, 'Other');
  assert.equal(base.colour, '');
  // Typed in: the field is the designer's and goes as it is.
  assert.equal(intakeBase(form, auto, { category: true, colour: true }).category, 'Nuts');
  assert.equal(intakeBase({ ...form, colour: 'Green' }, auto, none).colour, 'Green');
});

test('saving keeps an empty category empty so that the library runs its intake', () => {
  assert.equal(saveDraft({ ...blankForm(), name: 'Gorilla Glue Sticks' }).category, '');
  assert.equal(saveDraft({ ...blankForm(), name: 'x', category: '  Bolts ' }).category, 'Bolts');
});

test('save-time category: an empty one the designer decided on goes as Other, an untouched empty one stays empty for intake', () => {
  // Undo, or clearing the field by hand after an automatic fill: the designer's choice stands.
  assert.equal(categoryForSave({ category: '' }, true), 'Other');
  assert.equal(categoryForSave({ category: '   ' }, true), 'Other');
  // Never touched: the library may assign.
  assert.equal(categoryForSave({ category: '' }, false), '');
  assert.equal(categoryForSave({ category: '  ' }, false), '');
  // A category is sent as typed, locked or not.
  assert.equal(categoryForSave({ category: ' Filament ' }, false), 'Filament');
  assert.equal(categoryForSave({ category: 'Bolts' }, true), 'Bolts');
});
