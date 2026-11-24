import test from 'node:test';
import assert from 'node:assert/strict';
import {
  batchDrafts, blankForm, duplicateForm, editForm, estimateLabel, isEstimate, matchingItems, parsePrice, priceText, reviewLabel, saveDraft,
  withPrice,
} from '../src/components/workshopForm.ts';

test('review preserves manual changes and treats a package count separately from stock', () => {
  const form = { ...blankForm(), brand: 'Brand from the user', quantity: '0' };
  const scan = { ...saveDraft({ ...blankForm(), name: 'Socket screws' }), brand: 'Model brand', quantity: 100, pack_quantity: 100 };
  const reviewed = reviewLabel(form, scan);
  assert.equal(reviewed.brand, form.brand);
  assert.equal(reviewed.name, 'Socket screws');
  assert.equal(saveDraft(reviewed).pack_quantity, 100);
  assert.equal(saveDraft(reviewed).quantity, 0);
  assert.equal(reviewLabel(blankForm(), scan).quantity, '');
  assert.equal(form.name, '');
});

test('a draft distinguishes zero and unknown and rejects quantities that lose precision', () => {
  const draft = { ...blankForm(), name: '  Washers  ', quantity: '0' };
  assert.equal(saveDraft(draft).name, 'Washers');
  assert.equal(saveDraft(draft).quantity, 0);
  assert.equal(saveDraft(draft).pack_quantity, null);
  assert.equal(editForm(saveDraft(draft)).quantity, '0');
  for (const bad of ['1.5', '-1', '1e4', '9007199254740992']) {
    assert.throws(() => saveDraft({ ...draft, quantity: bad }), /whole number/);
  }
  assert.throws(() => saveDraft(blankForm()), /name/i);
});

test('search matches words across item fields and respects the category filter', () => {
  const draft = saveDraft({ ...blankForm(), name: 'Short socket screw', brand: 'Acme', category: 'Fasteners', part_number: 'SC-02', dimensions: 'M3 × 8 mm' });
  const items = [{ ...draft, id: 'a' }, { ...draft, category: 'Tools', id: 'b' }];
  assert.deepEqual(matchingItems(items, 'acme  Sc-02 M3', 'Fasteners').map(x => x.id), ['a']);
  assert.deepEqual(matchingItems(items, 'missing', ''), []);
  assert.equal(matchingItems(items, '', '').length, 2);
});

test('supplier links only open public HTTPS hostnames', async () => {
  const { publicListingUrl } = await import('../src/components/workshopForm.ts');
  assert.equal(publicListingUrl('https://shop.example.org/products/part')?.hostname, 'shop.example.org');
  for (const url of [
    'http://shop.example.org', 'https://localhost', 'https://127.0.0.1/path', 'https://[::1]',
    'https://printer.local', 'https://a:b@shop.example.org', 'https://shop.example.org/raw path',
    'javascript:alert(0)', 'https://shop..example.org',
  ]) assert.equal(publicListingUrl(url), null, url);
});

test('the item colour survives edit, label review, save and search', () => {
  const scan = { ...blankForm(), name: 'Filament', colour: 'Translucent orange', quantity: null, pack_quantity: null };
  assert.equal(blankForm().colour, '');
  const fromLabel = reviewLabel(blankForm(), scan);
  assert.equal(fromLabel.colour, 'Translucent orange');
  const manual = { ...fromLabel, colour: '  Burnt sienna  ' };
  assert.equal(reviewLabel(manual, scan).colour, manual.colour);
  const stored = saveDraft(manual);
  assert.equal(stored.colour, 'Burnt sienna');
  assert.equal(editForm(stored).colour, stored.colour);
  assert.equal(editForm({ ...stored, colour: '' }).colour, '');
  assert.deepEqual(matchingItems([{ ...stored, id: 'a' }], 'sienna filament', '').map(item => item.id), ['a']);
});

test('photo selection checks actual file format and size before reading or attaching the photo', async () => {
  const { validateItemPhoto } = await import('../src/components/workshopForm.ts');
  const file = bytes => new File([Uint8Array.from(bytes)], 'item.jpg', { type: 'image/jpeg' });
  for (const header of [
    [0xff, 0xd8, 0xff, 0xe0],
    [0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a],
    [...Buffer.from('RIFF1234WEBP')],
  ]) await validateItemPhoto(file(header));
  await assert.rejects(validateItemPhoto(file([1, 2, 3])), /JPEG, PNG, or WebP/);
  await assert.rejects(validateItemPhoto(file([])), /12 MiB/);
  await assert.rejects(validateItemPhoto({ size: 12 * 1024 * 1024 + 1 }), /12 MiB/);
});

const spec = (role, value, unit = '', source = 'label', extra = {}) =>
  ({ role, name: '', value, unit, tolerance: '', source, evidence: 'printed', ...extra });

test('specSummary matches the summary the database stores', async () => {
  const { specSummary } = await import('../src/components/workshopForm.ts');
  const specs = [spec('thread', 'M3'), spec('pitch', '0.5', 'mm', 'standard'), spec('length', '5.7', 'mm')];
  assert.equal(specSummary(specs), 'M3 thread; pitch 0.5 mm (standard); length 5.7 mm');
  assert.equal(specSummary([]), '');
  assert.equal(specSummary([spec('other', 'Hex', '', 'entered', { name: 'Drive' })]), 'Drive Hex');
  assert.equal(specSummary([spec('diameter', '1.75', 'mm', 'label', { tolerance: '0.03' })]), 'diameter 1.75 ±0.03 mm');
});

test('editing a printed or table value turns it into an entered one', async () => {
  const { editSpec, addSpec, removeSpec } = await import('../src/components/workshopForm.ts');
  const specs = [spec('thread', 'M3', '', 'notation'), spec('pitch', '0.5', 'mm', 'standard')];
  const edited = editSpec(specs, 1, { value: '0.6' });
  assert.equal(edited[1].value, '0.6');
  assert.equal(edited[1].source, 'entered');
  assert.equal(edited[1].evidence, 'printed');
  assert.equal(edited[0], specs[0]);
  assert.equal(specs[1].source, 'standard');
  assert.equal(editSpec(specs, 0, { value: 'M3' })[0].source, 'notation');
  assert.equal(editSpec(specs, 0, { evidence: 'forged' })[0].evidence, 'printed');
  const added = addSpec(edited);
  assert.equal(added.length, 3);
  assert.equal(added[2].source, 'entered');
  assert.deepEqual(removeSpec(added, 0).map(s => s.role), ['pitch', 'other']);
});

test('a bought pack adds its count to the stock', async () => {
  const { addPack } = await import('../src/components/workshopForm.ts');
  const form = { ...blankForm(), name: 'M3 insert', pack_quantity: '50' };
  assert.equal(addPack(form).quantity, '50');
  assert.equal(addPack({ ...form, quantity: '20' }).quantity, '70');
  assert.equal(addPack({ ...form, quantity: '0' }).quantity, '50');
  assert.throws(() => addPack({ ...form, pack_quantity: '' }), /quantity per pack/);
});

test('a batch names the item that cannot be saved', async () => {
  const { batchDrafts } = await import('../src/components/workshopForm.ts');
  const good = { ...blankForm(), name: 'M3 insert' };
  const drafts = batchDrafts([good, { ...good, name: 'M4 insert', quantity: '7' }]);
  assert.deepEqual(drafts.map(d => d.quantity), [null, 7]);
  assert.throws(() => batchDrafts([good, { ...good, name: ' ' }]), /^Error: Item 2: Enter an item name/);
  assert.throws(() => batchDrafts([good, { ...good, quantity: '1.5' }]), /Item 2: .*whole number/);
});

test('a label review keeps the specs the user already has and builds the Dimensions text from them', () => {
  const scan = { ...saveDraft({ ...blankForm(), name: 'Insert' }), dimensions: 'old', specs: [spec('thread', 'M3')] };
  const fresh = reviewLabel(blankForm(), scan);
  assert.equal(fresh.specs.length, 1);
  assert.equal(fresh.dimensions, 'M3 thread');
  const own = { ...blankForm(), specs: [spec('length', '5', 'mm', 'entered')] };
  assert.deepEqual(reviewLabel(own, scan).specs, own.specs);
  assert.equal(blankForm().specs.length, 0);
});

test('changing a role away from Other drops the name that only Other may carry', async () => {
  const { editSpec } = await import('../src/components/workshopForm.ts');
  const specs = [spec('other', 'Hex', '', 'entered', { name: 'Head height' })];
  assert.equal(editSpec(specs, 0, { role: 'length' })[0].name, '');
  assert.equal(editSpec(specs, 0, { value: 'Torx' })[0].name, 'Head height');
});

test('adding the first spec keeps the typed Dimensions text in the notes', async () => {
  const { withSpecs } = await import('../src/components/workshopForm.ts');
  const form = { ...blankForm(), name: 'Imported', dimensions: 'M3 x 5.7, OD 4.6', notes: 'Bin 4' };
  const one = withSpecs(form, [spec('hole_diameter', '4.2', 'mm', 'entered')]);
  assert.equal(one.dimensions, 'hole diameter 4.2 mm');
  assert.equal(one.notes, 'Bin 4\nEarlier dimensions: M3 x 5.7, OD 4.6');
  const two = withSpecs(one, [...one.specs, spec('length', '5.7', 'mm', 'entered')]);
  assert.equal(two.notes, one.notes);
  assert.equal(withSpecs({ ...blankForm(), dimensions: '' }, [spec('length', '5', 'mm', 'entered')]).notes, '');
});

test('a price is typed in dollars and saved as whole cents', () => {
  assert.equal(parsePrice(''), null);
  assert.equal(parsePrice('  '), null);
  assert.equal(parsePrice('9.99'), 999);
  assert.equal(parsePrice('$9.99'), 999);
  assert.equal(parsePrice('$ 9.99'), 999);
  assert.equal(parsePrice('1,234.50'), 123450);
  assert.equal(parsePrice('12'), 1200);
  assert.equal(parsePrice('0'), 0);
  assert.equal(parsePrice('0.5'), 50);
  assert.equal(parsePrice('.5'), 50);
  assert.equal(parsePrice('19.90'), 1990);
  // 1.15 * 100 and 0.29 * 100 lose cents in floating point; the text is converted without them.
  assert.equal(parsePrice('1.15'), 115);
  assert.equal(parsePrice('0.29'), 29);
  assert.equal(parsePrice('4.35'), 435);
  for (const bad of ['abc', '9.99x', '1.2.3', '1,23', '12,34.5', '9.', '.', '$', '$$5', '1e3']) {
    assert.throws(() => parsePrice(bad), /price in dollars/, bad);
  }
  assert.throws(() => parsePrice('-1'), /0 or more/);
  assert.throws(() => parsePrice('$-1.00'), /0 or more/);
  assert.throws(() => parsePrice('9.999'), /at most two decimals/);
  assert.throws(() => parsePrice('99999999999'), /or less/);
});

test('price text for editing round-trips cents', () => {
  assert.equal(priceText(null), '');
  assert.equal(priceText(undefined), '');
  assert.equal(priceText(999), '9.99');
  assert.equal(priceText(0), '0.00');
  assert.equal(priceText(5), '0.05');
  assert.equal(priceText(123450), '1234.50');
  for (const cents of [0, 5, 115, 999, 123450, 1_000_000_000_000]) assert.equal(parsePrice(priceText(cents)), cents);
});

test('the price survives the form, a save, a label review and an old stored form', () => {
  const draft = saveDraft({ ...blankForm(), name: 'PLA', price: '$24.99' });
  assert.equal(draft.price_cents, 2499);
  assert.equal('price' in draft, false);
  assert.equal(editForm(draft).price, '24.99');
  assert.equal(saveDraft({ ...blankForm(), name: 'x' }).price_cents, null);
  assert.throws(() => saveDraft({ ...blankForm(), name: 'x', price: '9.999' }), /two decimals/);
  // A card stored by an older phone session has no price field.
  const old = { ...blankForm(), name: 'x' };
  delete old.price;
  assert.equal(saveDraft(old).price_cents, null);
  const label = { ...saveDraft({ ...blankForm(), name: 'PLA' }), price_cents: 1250 };
  assert.equal(reviewLabel(blankForm(), label).price, '12.50');
  assert.equal(reviewLabel({ ...blankForm(), price: '3.00' }, label).price, '3.00');
  assert.equal(reviewLabel(blankForm(), { ...label, price_cents: null }).price, '');
});

test('duplicating an item keeps its fields, clears the count and copies no photo', () => {
  const spec = { role: 'thread', name: '', value: 'M3', unit: '', tolerance: '', source: 'label', evidence: 'M3' };
  const supplier = { title: 'Shop', url: 'https://shop.example.com/m3', details: '' };
  const item = { ...saveDraft({ ...blankForm(), name: 'M3 insert', category: 'Fasteners', brand: 'Acme', part_number: 'I-3',
    colour: 'brass', quantity: '40', pack_quantity: '50', price: '9.99', notes: 'for PLA', specs: [spec], supplier }),
    id: 'abc', photo_key: 'key-1' };
  const copy = duplicateForm(item);
  assert.equal(copy.quantity, '');
  assert.equal(copy.name, 'M3 insert');
  assert.deepEqual({ ...copy, quantity: '40' }, editForm(item));
  assert.equal(copy.price, '9.99');
  assert.equal('photo_key' in copy, false);
  assert.equal('id' in copy, false);
  assert.notEqual(copy.specs[0], item.specs[0]);
  assert.notEqual(copy.supplier, item.supplier);
  assert.equal(saveDraft(copy).quantity, null);
});

test('the list can filter to items without a price or without a count', () => {
  const base = saveDraft({ ...blankForm(), name: 'Part' });
  const items = [
    { ...base, id: 'a', price_cents: 100, quantity: 2 },
    { ...base, id: 'b', price_cents: null, quantity: 2 },
    { ...base, id: 'c', price_cents: 100, quantity: null },
    { ...base, id: 'd', price_cents: 0, quantity: 0 },
    { ...base, id: 'e', price_cents: null, quantity: null, category: 'Tools' },
  ];
  const ids = (...args) => matchingItems(items, ...args).map(x => x.id);
  assert.deepEqual(ids('', '', { noPrice: true }), ['b', 'e']);
  assert.deepEqual(ids('', '', { noCount: true }), ['c', 'e']);
  assert.deepEqual(ids('', '', { noPrice: true, noCount: true }), ['e']);
  assert.deepEqual(ids('', 'Tools', { noPrice: true }), ['e']);
  assert.deepEqual(ids('', '', {}), ['a', 'b', 'c', 'd', 'e']);
  assert.deepEqual(ids('', ''), ['a', 'b', 'c', 'd', 'e']);
});

test('the category filter treats letter case as one category, as the library does', () => {
  const base = saveDraft({ ...blankForm(), name: 'Part' });
  const items = [
    { ...base, id: 'a', category: 'Bolts' },
    { ...base, id: 'b', category: 'bolts' },
    { ...base, id: 'c', category: 'Tape' },
  ];
  const ids = category => matchingItems(items, '', category).map(x => x.id);
  assert.deepEqual(ids('Bolts'), ['a', 'b']);
  assert.deepEqual(ids('BOLTS'), ['a', 'b']);
  assert.deepEqual(ids('bolts'), ['a', 'b']);
  assert.deepEqual(ids('Tape'), ['c']);
  assert.deepEqual(ids('Nuts'), []);
  assert.deepEqual(ids(''), ['a', 'b', 'c']);
});

const listing = 'listing:canadabolts.ca';

test('the price source is carried by the form, saved with the draft and defaults to the designer', () => {
  assert.equal(blankForm().price_source, '');
  const estimate = { ...blankForm(), name: 'M3 bolt', price: '4.50', price_source: listing };
  const draft = saveDraft(estimate);
  assert.equal(draft.price_cents, 450);
  assert.equal(draft.price_source, listing);
  assert.equal(editForm(draft).price_source, listing);
  assert.equal(editForm(draft).price, '4.50');
  assert.equal('price_source' in saveDraft({ ...blankForm(), name: 'x' }), true);
  assert.equal(saveDraft({ ...blankForm(), name: 'x' }).price_source, '');
  // A source with no price is not an estimate.
  assert.equal(saveDraft({ ...estimate, price: '' }).price_source, '');
  assert.deepEqual(batchDrafts([estimate, { ...estimate, price: '' }]).map(d => d.price_source), [listing, '']);
});

test('an older phone card or draft without a price source still loads and saves', () => {
  const old = { ...blankForm(), name: 'x', price: '2.00' };
  delete old.price_source;
  assert.equal(saveDraft(old).price_source, '');
  const draft = { ...saveDraft({ ...blankForm(), name: 'x' }) };
  delete draft.price_source;
  assert.equal(editForm(draft).price_source, '');
  assert.equal(withPrice(old, '3.00').price_source, '');
  assert.equal(isEstimate(old), false);
});

test('a duplicate keeps the estimate mark with the price', () => {
  const item = { ...saveDraft({ ...blankForm(), name: 'Bolt', price: '4.50', price_source: listing, quantity: '3' }), id: 'a', photo_key: null };
  const copy = duplicateForm(item);
  assert.equal(copy.price_source, listing);
  assert.equal(copy.price, '4.50');
  assert.deepEqual({ ...copy, quantity: '3' }, editForm(item));
});

test('a label review keeps the mark of the price that stays', () => {
  const label = { ...saveDraft({ ...blankForm(), name: 'PLA', price: '12.50', price_source: listing }) };
  assert.equal(reviewLabel(blankForm(), label).price_source, listing);
  assert.equal(reviewLabel({ ...blankForm(), price: '3.00' }, label).price_source, '');
  const mine = { ...blankForm(), price: '3.00', price_source: 'listing:thepihut.com' };
  assert.equal(reviewLabel(mine, label).price_source, 'listing:thepihut.com');
  assert.equal(reviewLabel(blankForm(), { ...label, price_cents: null }).price_source, '');
});

test('typing a different price clears the estimate mark and the same price keeps it', () => {
  const estimate = { ...blankForm(), name: 'Bolt', price: '24.90', price_source: listing };
  assert.equal(withPrice(estimate, '24.90').price_source, listing);
  // The same price in another spelling is no edit.
  assert.equal(withPrice(estimate, '24.9').price_source, listing);
  assert.equal(withPrice(estimate, '$24.90').price_source, listing);
  assert.equal(withPrice(estimate, '24.99').price_source, '');
  assert.equal(withPrice(estimate, '').price_source, '');
  assert.equal(withPrice(estimate, '24.').price_source, '');
  assert.equal(withPrice(estimate, '24.99').price, '24.99');
  // Retyping the old price afterwards does not bring the mark back.
  assert.equal(withPrice(withPrice(estimate, '24.99'), '24.90').price_source, '');
  // A price the designer entered stays theirs.
  assert.equal(withPrice({ ...blankForm(), price: '1.00' }, '2.00').price_source, '');
  // The form is not changed in place.
  assert.equal(estimate.price, '24.90');
  assert.equal(estimate.price_source, listing);
  assert.equal(saveDraft(withPrice(estimate, '30')).price_source, '');
});

test('isEstimate tells an estimate from a price the designer entered, for a form or an item', () => {
  assert.equal(isEstimate({ price_source: listing }), true);
  assert.equal(isEstimate({ price_source: '' }), false);
  assert.equal(isEstimate({ price_source: '   ' }), false);
  assert.equal(isEstimate({}), false);
  assert.equal(isEstimate({ price_source: null }), false);
  assert.equal(isEstimate({ ...blankForm(), price_source: listing }), true);
  assert.equal(isEstimate(saveDraft({ ...blankForm(), name: 'x', price: '1', price_source: listing })), true);
});

test('the estimate label names the supplier host', () => {
  assert.equal(estimateLabel('listing:canadabolts.ca'), 'Estimated from canadabolts.ca');
  assert.equal(estimateLabel('listing:3dprintingcanada.com'), 'Estimated from 3dprintingcanada.com');
  assert.equal(estimateLabel(''), '');
  assert.equal(estimateLabel(undefined), '');
  assert.equal(estimateLabel(null), '');
  assert.equal(estimateLabel('listing:'), 'Estimated');
  assert.equal(estimateLabel('other'), 'Estimated');
});
