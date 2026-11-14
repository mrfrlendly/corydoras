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
