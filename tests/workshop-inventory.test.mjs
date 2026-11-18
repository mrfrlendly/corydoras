import test from 'node:test';
import assert from 'node:assert/strict';
import { blankForm, editForm, matchingItems, reviewLabel, saveDraft } from '../src/components/workshopForm.ts';

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
