import test from 'node:test';
import assert from 'node:assert/strict';
import { DISCONNECTED, OFFLINE, messageFor, tokenFromHash } from '../src/phone/phoneApi.ts';
import { fitWithin } from '../src/phone/photo.ts';
import { frameDrafts } from '../src/inventoryFrame.ts';
import { blankForm, saveDraft } from '../src/components/workshopForm.ts';

test('the token comes only from a #t= fragment', () => {
  assert.equal(tokenFromHash('#t=abc'), 'abc');
  for (const hash of ['#t=', '#x=1', '', '#', 't=abc']) assert.equal(tokenFromHash(hash), null, hash);
});

test('a phone photo shrinks to 3072 px on its long side and never grows', () => {
  assert.deepEqual(fitWithin(4032, 3024, 3072), { width: 3072, height: 2304 });
  assert.deepEqual(fitWithin(3024, 4032, 3072), { width: 2304, height: 3072 });
  assert.deepEqual(fitWithin(8064, 6048, 3072), { width: 3072, height: 2304 });
  assert.deepEqual(fitWithin(1000, 800, 3072), { width: 1000, height: 800 });
});

test('a batch frame is the JSON length, the JSON and the photo', async () => {
  const draft = saveDraft({ ...blankForm(), name: 'M3 insert' });
  const frame = await frameDrafts([draft], new Blob([new Uint8Array([7, 8, 9])]));
  const length = new DataView(frame.buffer, frame.byteOffset).getUint32(0, false);
  const json = JSON.parse(new TextDecoder().decode(frame.slice(4, 4 + length)));
  assert.deepEqual(json, { drafts: [draft] });
  assert.deepEqual([...frame.slice(4 + length)], [7, 8, 9]);
});

test('errors read as the designer should see them', () => {
  assert.equal(messageFor('offline'), OFFLINE);
  assert.equal(OFFLINE, 'Cannot reach Corydoras. Check that the Mac is awake, on the same Wi-Fi, and that Phone is on in Settings.');
  assert.equal(messageFor(401), DISCONNECTED);
  assert.equal(DISCONNECTED, 'This phone is no longer connected. Open Corydoras on your Mac, go to Settings › Phone, and scan the code.');
  assert.equal(messageFor(400, 'Select an image analysis provider in Settings.'), 'Select an image analysis provider in Settings.');
  assert.equal(messageFor(500), 'Corydoras answered with an error (500).');
});

test('removing the last spec on the phone also removes its generated Dimensions text', async () => {
  const { removeSpecRow, editForm, withSpecs, blankForm: blank } = await import('../src/components/workshopForm.ts');
  const spec = (role, value) => ({ role, name: '', value, unit: 'mm', tolerance: '', source: 'label', evidence: value });
  const form = withSpecs(blank(), [spec('length', '5.7'), spec('outer_diameter', '4.6')]);
  const one = removeSpecRow(form, 0);
  assert.equal(one.specs.length, 1);
  assert.equal(one.dimensions, 'outer diameter 4.6 mm');
  const none = removeSpecRow(one, 0);
  assert.equal(none.specs.length, 0);
  assert.equal(none.dimensions, '');
  // Text the designer typed is not generated text, so it stays.
  const typed = { ...editForm({ ...form, dimensions: 'measured 5.6 mm' }), specs: [spec('length', '5.7')] };
  assert.equal(removeSpecRow(typed, 0).dimensions, 'measured 5.6 mm');
});

test('only a read that never reached the Mac stays pending for a reload; an answered failure is not paid again', async () => {
  const { keepsReadPending, PhoneError, OFFLINE: offline } = await import('../src/phone/phoneApi.ts');
  assert.equal(keepsReadPending(new PhoneError(offline, 'offline')), true);
  assert.equal(keepsReadPending(new PhoneError('The provider returned an incomplete list.', 400)), false);
  assert.equal(keepsReadPending(new PhoneError('Corydoras answered with an error (500).', 500)), false);
});

test('each photo gets its own batch id for an idempotent save', async () => {
  const { newBatchId } = await import('../src/phone/phoneApi.ts');
  const a = newBatchId();
  assert.match(a, /^[0-9a-f]{32}$/);
  assert.notEqual(a, newBatchId());
});
