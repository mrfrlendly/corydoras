// The colour words of the workshop inventory. shared/colours.json is the single source: Rust (colour suggestions)
// reads the same file. The import works with tsc, vite and `node --experimental-strip-types`.
import dictionary from '../../shared/colours.json' with { type: 'json' };

interface Modifier { mix: string; amount: number }
interface Shade { base: string; hex: string }
interface Dictionary {
  base: Record<string, string>;
  aliases: Record<string, string>;
  modifiers: Record<string, Modifier>;
  shades: Record<string, Shade>;
  /** Two-word colour names with their own badge colour, such as "olive green". */
  compounds: Record<string, string>;
}

const words = dictionary as unknown as Dictionary;
const has = (table: object, word: string) => Object.prototype.hasOwnProperty.call(table, word);

/** What a modifier mixes the colour with: the dictionary names black or white. */
const mixColours: Record<string, string> = { black: '#000000', white: '#ffffff' };

/** The longest colour phrase of one colour: modifiers and colour words together. */
const maxPhraseWords = 4;
/** The most colours of one text, joined by "/". */
const maxColours = 3;

function parseHex(hex: string): [number, number, number] | null {
  const found = /^#([a-f\d]{3}|[a-f\d]{6})$/i.exec(hex.trim());
  if (!found) return null;
  const digits = found[1].length === 3 ? [...found[1]].map(c => c + c).join('') : found[1];
  return [0, 2, 4].map(i => parseInt(digits.slice(i, i + 2), 16)) as [number, number, number];
}

/**
 * Mixes `hex` with `other` (black, white or a hex colour): amount 0 keeps `hex`, 1 gives `other`.
 * Returns `hex` unchanged when either is not a hex colour.
 */
export function mixHex(hex: string, other: string, amount: number): string {
  const from = parseHex(hex);
  const to = parseHex(mixColours[other] ?? other);
  if (!from || !to) return hex;
  const share = Math.min(1, Math.max(0, amount));
  const channels = from.map((value, i) => Math.round(value + (to[i] - value) * share));
  return `#${channels.map(value => value.toString(16).padStart(2, '0')).join('')}`;
}

/** A colour phrase of one colour: its words, the badge colour and the base colour it counts as. */
export interface ColourPhrase {
  /** The phrase words, lower case. */
  words: string[];
  /** The base colour word: "grey" for "Titan Grey". */
  base: string;
  /** The modifiers of the phrase, in order. */
  modifiers: string[];
  /** The badge colour: a hex colour, or 'transparent' for clear. */
  hex: string;
}

/**
 * Reads one colour: modifiers and colour words. The colour is, in order of reading:
 * a compound from the dictionary ("Olive Green", "Blue Green"); a base colour or alias ("gray"); a shade, alone or
 * followed by its own base word ("Titan Grey"); or a run of shades of one base colour, with or without that base word
 * or an alias of it ("Prairie Rose", "Ash Titan Grey"), drawn with the last shade. Modifiers ("Dark", "Matte") may
 * come before any of these and mix the badge colour in the order written. Clear next to another colour is a finish:
 * "Translucent Red" is red. null when a word is unknown, the words name more than one colour ("Titan Rose"), or the
 * phrase has more than 4 words. Case does not matter.
 */
export function parseColourPhrase(text: string): ColourPhrase | null {
  const parts = text.trim().toLowerCase().split(/\s+/).filter(Boolean);
  if (parts.length === 0 || parts.length > maxPhraseWords) return null;
  const modifiers: string[] = [];
  const colours: string[] = [];
  for (const part of parts) {
    if (has(words.modifiers, part)) modifiers.push(part);
    else if (has(words.shades, part) || has(words.base, part)) colours.push(part);
    else if (has(words.aliases, part)) colours.push(words.aliases[part]);
    else colours.push(part); // unknown: readColours rejects it unless a compound uses it
  }
  const reading = readColours(parts, modifiers, colours);
  if (!reading) return null;
  let { hex } = reading;
  if (hex !== 'transparent') {
    for (const modifier of modifiers) {
      const { mix, amount } = words.modifiers[modifier];
      hex = mixHex(hex, mix, amount);
    }
  }
  return { words: parts, base: reading.base, modifiers, hex };
}

/** The base colour word and the colour before the modifiers, or null when the colour words are not one colour. */
function readColours(parts: string[], modifiers: string[], colours: string[]): { base: string; hex: string } | null {
  // A compound is read before any other way: its words may be colours too ("blue green") or not ("army green").
  const joined = compound(parts, modifiers);
  if (joined) return joined;
  if (!colours.every(word => has(words.shades, word) || has(words.base, word))) return null;
  // Clear is a finish when another colour word is present.
  const named = colours.filter(word => word !== 'clear');
  const colourWords = named.length > 0 ? named : colours;
  if (colourWords.length === 0) return null;
  let run = 0;
  while (run < colourWords.length && has(words.shades, colourWords[run])) run += 1;
  const shades = colourWords.slice(0, run).map(word => words.shades[word]);
  const tail = colourWords.slice(run);
  if (run === 0) return tail.length === 1 ? { base: tail[0], hex: words.base[tail[0]] } : null;
  if (tail.length > 1 || shades.some(shade => shade.base !== shades[0].base)) return null;
  if (tail.length === 1 && tail[0] !== shades[0].base) return null;
  const last = shades[shades.length - 1];
  return { base: last.base, hex: last.hex };
}

/** A compound of the dictionary made of all the words that are not modifiers, or null. */
function compound(parts: string[], modifiers: string[]): { base: string; hex: string } | null {
  const rest = parts.filter(part => !modifiers.includes(part));
  const key = rest.join(' ');
  if (!has(words.compounds, key)) return null;
  const last = rest[rest.length - 1];
  return { base: has(words.base, last) ? last : rest[0], hex: words.compounds[key] };
}

/**
 * The CSS colours for a colour text, one for each part joined by "/", up to 3: ["#8d9094"] for "Titan Grey",
 * ["transparent"] for "Clear". [] when any part is not a known colour or the text has more than 3 parts.
 */
export function colourSwatches(text: string): string[] {
  const parts = text.trim().split(/\s*\/\s*/);
  if (parts.length > maxColours) return [];
  const colours: string[] = [];
  for (const part of parts) {
    const phrase = parseColourPhrase(part);
    if (!phrase) return [];
    colours.push(phrase.hex);
  }
  return colours;
}
