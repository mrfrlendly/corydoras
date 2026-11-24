import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { colourSwatches, mixHex, parseColourPhrase } from '../src/components/workshopColours.ts';

const dictionary = JSON.parse(readFileSync(new URL('../shared/colours.json', import.meta.url), 'utf8'));

test('a base colour word gives its badge colour from the shared file', () => {
  for (const [word, hex] of Object.entries(dictionary.base)) assert.deepEqual(colourSwatches(word), [hex], word);
  assert.deepEqual(colourSwatches('Red'), ['#d32f2f']);
});

test('an alias counts as its base word and gray is grey', () => {
  assert.deepEqual(colourSwatches('gray'), colourSwatches('grey'));
  assert.deepEqual(colourSwatches('Gray'), ['#808285']);
  assert.deepEqual(colourSwatches('lavender'), colourSwatches('violet'));
  assert.deepEqual(colourSwatches('Burgundy'), colourSwatches('maroon'));
  assert.deepEqual(colourSwatches('Crimson'), colourSwatches('red'));
});

test('a shade uses its own colour, with or without its base word', () => {
  assert.deepEqual(colourSwatches('Titan'), ['#8d9094']);
  assert.deepEqual(colourSwatches('Titan Grey'), ['#8d9094']);
  assert.deepEqual(colourSwatches('Titan Gray'), ['#8d9094']);
  assert.deepEqual(colourSwatches('Space Grey'), ['#53565a']);
  assert.deepEqual(colourSwatches('Misty Blue'), ['#9db8cc']);
  assert.deepEqual(colourSwatches('Midnight'), ['#1b2340']);
  for (const [word, shade] of Object.entries(dictionary.shades)) {
    assert.deepEqual(colourSwatches(word), [shade.hex], word);
    assert.deepEqual(colourSwatches(`${word} ${shade.base}`), [shade.hex], word);
  }
});

test('shades of the same base colour are one colour, drawn with the last shade', () => {
  assert.deepEqual(colourSwatches('Prairie Rose'), [dictionary.shades.rose.hex]);
  assert.deepEqual(colourSwatches('Rose Prairie'), [dictionary.shades.prairie.hex]);
  assert.deepEqual(colourSwatches('Dark Prairie Rose'), [mixHex(dictionary.shades.rose.hex, 'black', 0.3)]);
  assert.equal(parseColourPhrase('Prairie Rose').base, 'pink');
  assert.deepEqual(colourSwatches('Misty Blue'), ['#9db8cc']);
  assert.deepEqual(colourSwatches('Titan Rose'), []);
  assert.deepEqual(colourSwatches('Prairie Rose Titan'), []);
  const shades = Object.entries(dictionary.shades);
  let pairs = 0;
  for (const [a, first] of shades) {
    for (const [b, second] of shades) {
      if (a === b || first.base !== second.base) continue;
      pairs += 1;
      assert.deepEqual(colourSwatches(`${a} ${b}`), [second.hex], `${a} ${b}`);
    }
  }
  assert.ok(pairs > 0);
});

test('a shade does not take another colour as its base', () => {
  assert.deepEqual(colourSwatches('Misty Grey'), []);
  assert.deepEqual(colourSwatches('Titan Blue'), []);
});

// Modifiers apply in the order written.
test('a modifier mixes the badge colour with black or white by its amount', () => {
  assert.deepEqual(colourSwatches('Dark Brown'), [mixHex('#6d4c41', 'black', 0.3)]);
  assert.deepEqual(colourSwatches('Dark Brown'), ['#4c352e']);
  assert.deepEqual(colourSwatches('Light Blue'), ['#75ade4']);
  assert.deepEqual(colourSwatches('Pastel Pink'), [mixHex('#ec7fa5', 'white', 0.55)]);
  assert.deepEqual(colourSwatches('Dark Titan Grey'), [mixHex('#8d9094', 'black', 0.3)]);
  // Finish words with amount 0 leave the colour as it is.
  assert.deepEqual(colourSwatches('Matte Black'), ['#1f1f1f']);
  assert.deepEqual(colourSwatches('Silk Gold'), ['#c9a227']);
  assert.deepEqual(colourSwatches('Dark Light Blue'), [mixHex(mixHex('#1976d2', 'black', 0.3), 'white', 0.4)]);
});

const luminance = hex => {
  const [r, g, b] = [1, 3, 5].map(i => parseInt(hex.slice(i, i + 2), 16));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};

test('a compound is one badge with its own colour, in any case and inside a list', () => {
  assert.deepEqual(colourSwatches('Olive Green'), ['#6b8e23']);
  assert.deepEqual(colourSwatches('olive  GREEN'), ['#6b8e23']);
  assert.deepEqual(colourSwatches('Rose Gold'), ['#b76e79']);
  assert.deepEqual(colourSwatches('Blue Green'), ['#2a9d8f']);
  assert.deepEqual(colourSwatches('Navy Blue'), ['#1a237e']);
  assert.deepEqual(colourSwatches('Olive Green / Black'), ['#6b8e23', '#1f1f1f']);
  assert.deepEqual(colourSwatches('Black / Rose Gold / Blue Green'), ['#1f1f1f', '#b76e79', '#2a9d8f']);
  assert.equal(parseColourPhrase('Olive Green').hex, '#6b8e23');
});

test('every compound of the shared file gives one badge with its hex', () => {
  const entries = Object.entries(dictionary.compounds);
  assert.ok(entries.length > 0);
  for (const [phrase, hex] of entries) {
    assert.deepEqual(colourSwatches(phrase), [hex], phrase);
    assert.deepEqual(colourSwatches(phrase.toUpperCase()), [hex], phrase);
    assert.deepEqual(colourSwatches(`${phrase} / Black`), [hex, '#1f1f1f'], phrase);
  }
});

test('Dark Grey is computed from grey and is darker than Grey', () => {
  const [grey] = colourSwatches('Grey');
  const [dark] = colourSwatches('Dark Grey');
  assert.equal(dark, mixHex('#808285', 'black', 0.3));
  assert.ok(luminance(dark) < luminance(grey));
  assert.deepEqual(colourSwatches('Dark Gray'), [dark]);
  assert.deepEqual(colourSwatches('dark grey / light blue'), [dark, '#75ade4']);
});

test('a modifier alone is not a colour', () => {
  for (const text of ['Dark', 'Matte', 'Light Pale']) assert.deepEqual(colourSwatches(text), [], text);
});

test('clear is the transparent swatch', () => {
  assert.deepEqual(colourSwatches('Clear'), ['transparent']);
  assert.deepEqual(colourSwatches('Transparent'), ['transparent']);
  assert.deepEqual(colourSwatches('Translucent'), ['transparent']);
  assert.deepEqual(colourSwatches('Matte Clear'), ['transparent']);
  // Next to another colour, clear is a finish and the colour shows.
  assert.deepEqual(colourSwatches('Translucent Red'), ['#d32f2f']);
});

test('several colours joined by a slash get one swatch each, up to three', () => {
  assert.deepEqual(colourSwatches('Black / Red'), ['#1f1f1f', '#d32f2f']);
  assert.deepEqual(colourSwatches('black/red/Titan Grey'), ['#1f1f1f', '#d32f2f', '#8d9094']);
  assert.deepEqual(colourSwatches('Black / Red / Blue / Green'), []);
  assert.deepEqual(colourSwatches('Black / Mystery'), []);
  assert.deepEqual(colourSwatches('Black /'), []);
});

test('case and extra spaces do not matter', () => {
  assert.deepEqual(colourSwatches('  TITAN   grey '), ['#8d9094']);
  assert.deepEqual(colourSwatches('dark BROWN'), colourSwatches('Dark Brown'));
});

test('unknown text, four words and script-like text get no badge', () => {
  for (const text of ['', ' ', 'Mystery', 'Blue Mystery', 'Dark Dark Dark Dark Blue', 'Red Blue', 'Green Blue', 'url(https://example.com)',
    'var(--bg)', 'inherit', '__proto__', 'constructor', 'toString', 'hasOwnProperty', '#fff']) {
    assert.deepEqual(colourSwatches(text), [], text);
  }
});

test('a phrase reports its base colour and modifiers', () => {
  assert.deepEqual(parseColourPhrase('Titan Grey'), { words: ['titan', 'grey'], base: 'grey', modifiers: [], hex: '#8d9094' });
  assert.deepEqual(parseColourPhrase('Dark Brown'), { words: ['dark', 'brown'], base: 'brown', modifiers: ['dark'], hex: '#4c352e' });
  assert.equal(parseColourPhrase('Gray').base, 'grey');
  assert.equal(parseColourPhrase('Purple Dark Dark Dark Dark'), null);
  assert.equal(parseColourPhrase('Nothing'), null);
});

test('mixing keeps the hex form and clamps the amount', () => {
  assert.equal(mixHex('#808080', 'black', 0), '#808080');
  assert.equal(mixHex('#808080', 'black', 1), '#000000');
  assert.equal(mixHex('#808080', 'white', 1), '#ffffff');
  assert.equal(mixHex('#808080', 'white', 5), '#ffffff');
  assert.equal(mixHex('#808080', 'black', -1), '#808080');
  assert.equal(mixHex('#fff', 'black', 0.5), '#808080');
  assert.equal(mixHex('#102030', '#ffffff', 0.5), '#889098');
  assert.equal(mixHex('transparent', 'black', 0.5), 'transparent');
});

const aliasesOf = base => Object.entries(dictionary.aliases).filter(([, target]) => target === base).map(([alias]) => alias);

test('a modifier before a compound mixes the compound colour, also in a list', () => {
  assert.deepEqual(colourSwatches('Dark Blue Green'), [mixHex('#2a9d8f', 'black', 0.3)]);
  assert.deepEqual(colourSwatches('Bright Olive Green'), [mixHex('#6b8e23', 'white', 0.1)]);
  assert.deepEqual(colourSwatches('Matte Rose Gold'), ['#b76e79']);
  assert.deepEqual(colourSwatches('Matte Blue Green / Green'), ['#2a9d8f', '#388e3c']);
  assert.deepEqual(colourSwatches('Dark Light Navy Blue'), [mixHex(mixHex('#1a237e', 'black', 0.3), 'white', 0.4)]);
});

test('every compound with every modifier gives exactly one swatch', () => {
  for (const [phrase, hex] of Object.entries(dictionary.compounds)) {
    for (const [modifier, { mix, amount }] of Object.entries(dictionary.modifiers)) {
      assert.deepEqual(colourSwatches(`${modifier} ${phrase}`), [mixHex(hex, mix, amount)], `${modifier} ${phrase}`);
      assert.equal(colourSwatches(`${modifier.toUpperCase()} ${phrase} / Black`).length, 2, `${modifier} ${phrase}`);
    }
  }
});

test('a run of same-base shades, with the base word, an alias of it or neither, is one colour', () => {
  assert.deepEqual(colourSwatches('Ash Titan Grey'), ['#8d9094']);
  assert.deepEqual(colourSwatches('Amber Peach Orange'), ['#f4b183']);
  assert.deepEqual(colourSwatches('Azure Cobalt Blue'), ['#0047ab']);
  assert.deepEqual(colourSwatches('Ash Ash Grey'), ['#b2b4b2']);
  assert.deepEqual(colourSwatches('Ash Charcoal Gray'), ['#36454f']);
  assert.deepEqual(colourSwatches('Dark Ash Titan Grey'), [mixHex('#8d9094', 'black', 0.3)]);
  assert.deepEqual(colourSwatches('Ash Titan Charcoal'), ['#36454f']);
  assert.deepEqual(colourSwatches('Ash Titan Rose Grey'), []);
  assert.deepEqual(colourSwatches('Ash Titan Blue'), []);
  assert.deepEqual(colourSwatches('Ash Rose Grey'), []);
});

test('every ordered pair of same-base shades, with none, the base word or an alias, gives one swatch', () => {
  let count = 0;
  for (const [a, first] of Object.entries(dictionary.shades)) {
    for (const [b, second] of Object.entries(dictionary.shades)) {
      if (first.base !== second.base) continue;
      for (const tail of ['', first.base, ...aliasesOf(first.base)]) {
        const text = `${a} ${b} ${tail}`.trim();
        count += 1;
        assert.deepEqual(colourSwatches(text), [second.hex], text);
        assert.deepEqual(colourSwatches(`Dark ${text} / Black`).length, 2, text);
      }
    }
  }
  assert.ok(count > 0);
  // Different bases stay without a badge.
  assert.deepEqual(colourSwatches('Titan Rose Grey'), []);
});
