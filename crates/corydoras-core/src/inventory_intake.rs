//! Intake rules of the workshop inventory: a category from the words of an item, and a colour from
//! its name or description. The rules are deterministic and every suggestion carries its reason.
//! Nothing here invents a category: a suggestion is always a category of this library.

use crate::inventory::{inventory_row, SELECT_ITEMS};
use crate::inventory::{normalized_colour, InventoryDraft};
use crate::inventory_categories::{categories_in, CategoryInfo};
use crate::Library;
use anyhow::Result;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashSet;
use std::sync::OnceLock;

/// The most characters of one field that the rules read.
const READ_LIMIT: usize = 8192;

/// A category that a keyword rule replaces: these hold what no rule could place.
const GENERIC_CATEGORIES: &[&str] = &["Other", "Materials", "Hardware", "Consumables"];

/// Built-in words of the common categories, keyed by category name. A row applies only when the
/// library has a category of that name (ignoring case).
const BUILT_IN_WORDS: &[(&str, &[&str])] = &[
    (
        "Adhesives",
        &[
            "glue",
            "epoxy",
            "cyanoacrylate",
            "superglue",
            "super glue",
            "adhesive",
            "cement",
        ],
    ),
    (
        "Filament",
        &["filament", "pla", "petg", "abs", "asa", "tpu", "pla+"],
    ),
    ("Bolts", &["bolt", "screw", "cap screw", "machine screw"]),
    ("Nuts", &["nut", "lock nut", "nyloc"]),
    ("Washers", &["washer"]),
    ("Magnets", &["magnet", "neodymium"]),
    ("Tape", &["tape"]),
    ("Threaded Inserts", &["insert", "heat-set", "heat set"]),
    (
        "Tools",
        &["screwdriver", "wrench", "plier", "pliers", "tweezers"],
    ),
    ("Cord & Rope", &["cord", "rope", "paracord"]),
    (
        "Electronics",
        &[
            "arduino",
            "raspberry pi",
            "esp32",
            "sensor",
            "module",
            "led",
        ],
    ),
];

/// What intake knows about an item.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct FieldInput {
    pub name: String,
    pub brand: String,
    pub description: String,
    pub category: String,
    pub colour: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CategorySuggestion {
    pub name: String,
    /// Why, for example: name says "filament"
    pub reason: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct FieldSuggestions {
    pub category: Option<CategorySuggestion>,
    pub colour: Option<String>,
}

/// One proposed change to an existing item, for the Review suggestions screen.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ItemSuggestion {
    pub id: String,
    pub name: String,
    pub category_from: String,
    pub category_to: Option<String>,
    pub category_reason: String,
    pub colour_to: Option<String>,
    pub colour_reason: String,
}

/// What the designer ticked. A field left out is not changed.
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct SuggestionChange {
    pub id: String,
    pub category: Option<String>,
    pub colour: Option<String>,
}

/// A colour found in the text, with the words it came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ColourMatch {
    pub colour: String,
    pub reason: String,
}

/// A library category with every word that puts an item in it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CategoryRule {
    pub name: String,
    pub keywords: Vec<String>,
}

// ---------------------------------------------------------------------------------------------
// Words
// ---------------------------------------------------------------------------------------------

/// Lower case, letters and digits only, one space between words. A plus that ends a word stays
/// ("pla+"); apostrophes join the word ("Gorilla's" is one word). The result has no outer spaces.
fn normalize(text: &str) -> String {
    let mut out = String::new();
    let mut pending_space = false;
    let mut last_alnum_or_plus = false;
    for c in text.chars().take(READ_LIMIT) {
        if c.is_alphanumeric() {
            if pending_space && !out.is_empty() {
                out.push(' ');
            }
            pending_space = false;
            out.extend(c.to_lowercase());
            last_alnum_or_plus = true;
        } else if c == '+' && last_alnum_or_plus {
            out.push('+');
        } else if c == '\'' || c == '\u{2019}' {
            // joins the word
        } else {
            pending_space = true;
            last_alnum_or_plus = false;
        }
    }
    out
}

/// The words of a category name in the singular and the plural of its last word:
/// "Threaded Inserts" gives "threaded inserts" and "threaded insert".
fn name_forms(name: &str) -> Vec<String> {
    let phrase = normalize(name);
    let (head, last) = match phrase.rsplit_once(' ') {
        Some((head, last)) => (format!("{head} "), last.to_owned()),
        None => (String::new(), phrase.clone()),
    };
    if last.chars().count() < 2 {
        return vec![];
    }
    let sibilant = |w: &str| ["s", "x", "z", "ch", "sh"].iter().any(|e| w.ends_with(e));
    let mut forms = vec![last.clone()];
    if let Some(stem) = last.strip_suffix("ies") {
        forms.push(format!("{stem}y"));
    } else if let Some(stem) = last.strip_suffix("es").filter(|s| sibilant(s)) {
        forms.push(stem.to_owned());
    } else if let Some(stem) = last.strip_suffix('s').filter(|_| !last.ends_with("ss")) {
        forms.push(stem.to_owned());
    } else if let Some(stem) = last
        .strip_suffix('y')
        .filter(|s| !s.ends_with(['a', 'e', 'i', 'o', 'u']))
    {
        forms.push(format!("{stem}ies"));
    } else if sibilant(&last) {
        forms.push(format!("{last}es"));
    } else {
        forms.push(format!("{last}s"));
    }
    forms.into_iter().map(|f| format!("{head}{f}")).collect()
}

fn is_generic_name(name: &str) -> bool {
    GENERIC_CATEGORIES
        .iter()
        .any(|g| g.eq_ignore_ascii_case(name.trim()))
}

/// The rules of the categories of a library: the category's own name (not for the generic
/// categories), the built-in words that belong to its name, and the designer's keywords.
pub fn category_rules(categories: &[CategoryInfo]) -> Vec<CategoryRule> {
    categories
        .iter()
        .map(|category| {
            let mut keywords: Vec<String> = Vec::new();
            if !is_generic_name(&category.name) {
                keywords.extend(name_forms(&category.name));
            }
            if let Some((_, words)) = BUILT_IN_WORDS
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(category.name.trim()))
            {
                keywords.extend(words.iter().map(|w| w.to_string()));
            }
            keywords.extend(category.keywords.iter().cloned());
            let mut seen = HashSet::new();
            let keywords = keywords
                .iter()
                .map(|k| normalize(k))
                .filter(|k| !k.is_empty() && seen.insert(k.clone()))
                .collect();
            CategoryRule {
                name: category.name.clone(),
                keywords,
            }
        })
        .collect()
}

/// Whether a category is one that a rule may replace: empty, a generic one, or not a category of the library.
pub fn is_generic(category: &str, rules: &[CategoryRule]) -> bool {
    let category = category.trim();
    category.is_empty()
        || is_generic_name(category)
        || !rules.iter().any(|r| r.name.eq_ignore_ascii_case(category))
}

/// The words of `keyword` that stand whole in `text` (both normalized), also with a plural s or
/// as "pla+" for "pla". Returns the words as matched.
fn whole_word_match(text: &str, keyword: &str) -> Option<String> {
    let padded = format!(" {text} ");
    let mut forms = vec![keyword.to_owned()];
    if !keyword.ends_with(['s', '+']) {
        forms.push(format!("{keyword}s"));
    }
    if !keyword.ends_with('+') {
        forms.push(format!("{keyword}+"));
    }
    let last_word = keyword.rsplit(' ').next().unwrap_or(keyword);
    forms.into_iter().find(|form| {
        let needle = format!(" {form} ");
        let mut from = 0;
        while let Some(at) = padded[from..].find(&needle) {
            let end = from + at + needle.len();
            // The word right after the match; the text ends with a space, so the last word has none.
            let next = padded[end..].split(' ').next().unwrap_or("");
            if !stopped_by(last_word, next) {
                return true;
            }
            from += at + 1;
        }
        false
    })
}

/// Words that, right after a matched keyword, turn it into another product: "Tape measure" is
/// not tape, "Hot glue gun" is not glue.
const NEXT_WORD_STOPLIST: &[&str] = &[
    "gun",
    "guns",
    "cutter",
    "cutters",
    "driver",
    "drivers",
    "dryer",
    "dryers",
    "measure",
    "measuring",
    "tool",
    "tools",
    "stripper",
    "puller",
    "holder",
    "holders",
    "dispenser",
    "remover",
    "applicator",
];

/// The same for single keywords, keyed by the last word of the keyword.
const WORD_STOPLIST: &[(&str, &[&str])] = &[("pla", &["plate", "plates"])];

fn stopped_by(keyword_word: &str, next: &str) -> bool {
    NEXT_WORD_STOPLIST.contains(&next)
        || WORD_STOPLIST
            .iter()
            .any(|(word, stop)| *word == keyword_word && stop.contains(&next))
}

/// The category whose words best match the item. Name beats brand beats description; within
/// the first field that matches anything, the longest word wins, then the category with more
/// matching words, then the alphabetically first. Only a generic category is replaced, and a
/// suggestion is never the category the item already has, and never "Other".
pub fn suggest_category(fields: &FieldInput, rules: &[CategoryRule]) -> Option<CategorySuggestion> {
    if !is_generic(&fields.category, rules) {
        return None;
    }
    for (label, text) in [
        ("name", &fields.name),
        ("brand", &fields.brand),
        ("description", &fields.description),
    ] {
        let text = normalize(text);
        if text.is_empty() {
            continue;
        }
        // (longest word length, number of words, category, matched words)
        let mut best: Option<(usize, usize, &CategoryRule, String)> = None;
        for rule in rules
            .iter()
            .filter(|r| !r.name.eq_ignore_ascii_case("other"))
        {
            let mut longest: Option<(usize, String)> = None;
            let mut count = 0;
            for keyword in &rule.keywords {
                if let Some(found) = whole_word_match(&text, keyword) {
                    count += 1;
                    let length = keyword.chars().count();
                    if longest.as_ref().is_none_or(|(l, _)| length > *l) {
                        longest = Some((length, found));
                    }
                }
            }
            let Some((length, found)) = longest else {
                continue;
            };
            let better = match &best {
                None => true,
                Some((l, c, current, _)) => {
                    (length, count) > (*l, *c)
                        || ((length, count) == (*l, *c)
                            && rule.name.to_lowercase() < current.name.to_lowercase())
                }
            };
            if better {
                best = Some((length, count, rule, found));
            }
        }
        if let Some((_, _, rule, found)) = best {
            // The item may already be in this category (a generic one, such as Hardware, with its own words).
            if rule.name.eq_ignore_ascii_case(fields.category.trim()) {
                return None;
            }
            return Some(CategorySuggestion {
                name: rule.name.clone(),
                reason: format!("{label} says \"{found}\""),
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// Colours
// ---------------------------------------------------------------------------------------------

#[derive(Deserialize)]
struct Shade {
    base: String,
    /// Whether the shade word may be taken from a name without its base colour next to it.
    #[serde(default)]
    extract_alone: bool,
}

#[derive(Deserialize)]
struct Dictionary {
    base: std::collections::HashMap<String, String>,
    aliases: std::collections::HashMap<String, String>,
    modifiers: std::collections::HashMap<String, Value>,
    shades: std::collections::HashMap<String, Shade>,
    /// Words that are valid for a badge but never taken from a product name.
    #[serde(default)]
    extract_skip: Vec<String>,
    /// Two-word colour names with their own badge colour: "olive green".
    #[serde(default)]
    compounds: std::collections::HashMap<String, String>,
}

/// The colour words of the workshop, shared with the list badges (`src/components/workshopColours.ts`).
fn dictionary() -> &'static Dictionary {
    static DICTIONARY: OnceLock<Dictionary> = OnceLock::new();
    DICTIONARY.get_or_init(|| {
        serde_json::from_str(include_str!("../../../shared/colours.json"))
            .expect("shared/colours.json is a colour dictionary")
    })
}

/// A word of a colour phrase.
enum Word {
    Modifier,
    /// A base colour, or the base an alias stands for.
    Base(String),
    Shade(String),
}

fn colour_word(word: &str) -> Option<Word> {
    let dictionary = dictionary();
    if dictionary.modifiers.contains_key(word) {
        Some(Word::Modifier)
    } else if dictionary.shades.contains_key(word) {
        Some(Word::Shade(word.to_owned()))
    } else if dictionary.base.contains_key(word) {
        Some(Word::Base(word.to_owned()))
    } else {
        dictionary
            .aliases
            .get(word)
            .map(|base| Word::Base(base.clone()))
    }
}

fn title_case(word: &str) -> String {
    let mut chars = word.chars();
    match chars.next() {
        Some(first) => first
            .to_uppercase()
            .chain(chars.flat_map(char::to_lowercase))
            .collect(),
        None => String::new(),
    }
}

/// Reads 1 to 3 words as one colour, the way the list badge does (`parseColourPhrase`): modifiers
/// plus one base colour, alias or shade, where a shade may be followed by its own base ("Titan
/// Grey"). Two other colour words are two colours, not one phrase. Clear next to another colour is
/// a finish and is left out. Returns the words to store, in Title Case and in the order written.
fn parse_phrase(words: &[&str]) -> Option<Vec<String>> {
    if words.is_empty() || words.len() > 3 {
        return None;
    }
    if words
        .iter()
        .any(|w| dictionary().extract_skip.iter().any(|s| s == w))
    {
        return None;
    }
    // A compound ("olive green") is one colour, with at most modifiers before it.
    if words.len() >= 2 {
        let (head, pair) = words.split_at(words.len() - 2);
        if dictionary()
            .compounds
            .contains_key(&format!("{} {}", pair[0], pair[1]))
            && head
                .iter()
                .all(|w| matches!(colour_word(w), Some(Word::Modifier)))
        {
            return Some(words.iter().map(|w| title_case(w)).collect());
        }
    }
    let kinds: Vec<Word> = words
        .iter()
        .map(|w| colour_word(w))
        .collect::<Option<_>>()?;
    let is_clear = |kind: &Word| matches!(kind, Word::Base(base) if base == "clear");
    let colour_at: Vec<usize> = (0..words.len())
        .filter(|&i| !matches!(kinds[i], Word::Modifier))
        .collect();
    let named: Vec<usize> = colour_at
        .iter()
        .copied()
        .filter(|&i| !is_clear(&kinds[i]))
        .collect();
    let used = if named.is_empty() {
        colour_at.clone()
    } else {
        named
    };
    match used.as_slice() {
        [] => return None,
        [only] => {
            // "Stainless Steel Bolts": an ambiguous shade word is no colour without its base next to it.
            if let Word::Shade(shade) = &kinds[*only] {
                if !dictionary().shades.get(shade)?.extract_alone {
                    return None;
                }
            }
        }
        // Several shades of one base colour are one phrase ("Prairie Rose"), and so is a shade
        // run that ends with that base ("Titan Grey"). Shades of different bases are not.
        many => {
            let shade_base = |i: usize| match &kinds[i] {
                Word::Shade(shade) => dictionary().shades.get(shade).map(|s| s.base.as_str()),
                _ => None,
            };
            let (shades, tail) = match many.split_last() {
                Some((&last, rest)) if matches!(kinds[last], Word::Base(_)) => (rest, Some(last)),
                _ => (many, None),
            };
            let Some(&head) = shades.first() else {
                return None;
            };
            let base = shade_base(head)?;
            let same = shades.iter().all(|&i| shade_base(i) == Some(base));
            let ends_with_base =
                tail.is_none_or(|i| matches!(&kinds[i], Word::Base(w) if w == base));
            if !same || !ends_with_base {
                return None;
            }
        }
    }
    let alone_clear = used.iter().all(|&i| is_clear(&kinds[i]));
    Some(
        (0..words.len())
            .filter(|i| matches!(kinds[*i], Word::Modifier) || used.contains(i))
            .map(|i| {
                if alone_clear && !matches!(kinds[i], Word::Modifier) {
                    "Clear".to_owned()
                } else {
                    title_case(words[i])
                }
            })
            .collect(),
    )
}

/// A word of the original text with its place.
struct Token {
    start: usize,
    end: usize,
    lower: String,
}

fn tokens(text: &str) -> Vec<Token> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in text
        .char_indices()
        .chain(std::iter::once((text.len(), ' ')))
    {
        if c.is_alphanumeric() {
            start.get_or_insert(i);
        } else if let Some(s) = start.take() {
            out.push(Token {
                start: s,
                end: i,
                lower: text[s..i].to_lowercase(),
            });
        }
    }
    out
}

/// The text between two words is only spaces or hyphens: the words are one phrase.
fn contiguous(gap: &str) -> bool {
    gap.chars()
        .all(|c| c.is_whitespace() || matches!(c, '-' | '\u{2010}'..='\u{2014}'))
}

/// The longest colour phrase that starts at token `i`: its words to store and the index after it.
fn phrase_at(tokens: &[Token], text: &str, i: usize) -> Option<(Vec<String>, usize)> {
    let mut reach = 1;
    while reach < 3
        && i + reach < tokens.len()
        && contiguous(&text[tokens[i + reach - 1].end..tokens[i + reach].start])
    {
        reach += 1;
    }
    (1..=reach).rev().find_map(|len| {
        let words: Vec<&str> = tokens[i..i + len]
            .iter()
            .map(|t| t.lower.as_str())
            .collect();
        parse_phrase(&words).map(|stored| (stored, i + len))
    })
}

/// The colour of one text and where it stands: the first colour phrase, plus up to two more
/// joined to it by "/", "&" or "and" ("Black / Red").
fn colour_in(text: &str) -> Option<(String, (usize, usize))> {
    let tokens = tokens(text);
    let (first, words, mut next) = (0..tokens.len())
        .find_map(|i| phrase_at(&tokens, text, i).map(|(words, next)| (i, words, next)))?;
    let start = tokens[first].start;
    let mut end = tokens[next - 1].end;
    let mut phrases = vec![words.join(" ")];
    while phrases.len() < 3 && next < tokens.len() {
        let gap = &text[end..tokens[next].start];
        let from = if matches!(gap.trim(), "/" | "&") {
            next
        } else if gap.chars().all(char::is_whitespace)
            && !gap.is_empty()
            && tokens[next].lower == "and"
            && next + 1 < tokens.len()
            && text[tokens[next].end..tokens[next + 1].start]
                .chars()
                .all(char::is_whitespace)
        {
            next + 1
        } else {
            break;
        };
        let Some((words, upto)) = phrase_at(&tokens, text, from) else {
            break;
        };
        phrases.push(words.join(" "));
        next = upto;
        end = tokens[next - 1].end;
    }
    Some((phrases.join(" / "), (start, end)))
}

/// A colour for an item whose colour is empty: the first of name, brand and description that holds
/// one. The colour is stored as written in Title Case ("Titan grey" becomes "Titan Grey").
pub fn suggest_colour(fields: &FieldInput) -> Option<ColourMatch> {
    if !fields.colour.trim().is_empty() {
        return None;
    }
    for (label, text) in [
        ("name", &fields.name),
        ("brand", &fields.brand),
        ("description", &fields.description),
    ] {
        let text: String = text.chars().take(READ_LIMIT).collect();
        if let Some((colour, (start, end))) = colour_in(&text) {
            return Some(ColourMatch {
                colour,
                reason: format!("{label} says \"{}\"", &text[start..end]),
            });
        }
    }
    None
}

// ---------------------------------------------------------------------------------------------
// The library
// ---------------------------------------------------------------------------------------------

/// What intake reads of a saved or unsaved item. The description is the dimensions and the notes.
fn field_input(draft: &InventoryDraft) -> FieldInput {
    FieldInput {
        name: draft.name.clone(),
        brand: draft.brand.clone(),
        description: format!("{} {}", draft.dimensions, draft.notes),
        category: draft.category.clone(),
        colour: draft.colour.clone(),
    }
}

/// The lines of a text that state a colour: they start with "Colour" or "Color". Everything else
/// in notes and dimensions of an existing item is imported noise (listings, breadcrumbs, kits).
fn colour_lines(text: &str) -> String {
    text.split(['\n', '\r', ';'])
        .map(str::trim)
        .filter(|line| {
            let lower = line.to_lowercase();
            lower.starts_with("colour") || lower.starts_with("color")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// What review and apply read of an item that is already saved: the category comes from the name
/// and the brand only, the colour also from lines of the notes and dimensions that state a colour.
fn existing_inputs(draft: &InventoryDraft) -> (FieldInput, FieldInput) {
    let for_category = FieldInput {
        name: draft.name.clone(),
        brand: draft.brand.clone(),
        description: String::new(),
        category: draft.category.clone(),
        colour: draft.colour.clone(),
    };
    let for_colour = FieldInput {
        description: format!(
            "{}\n{}",
            colour_lines(&draft.dimensions),
            colour_lines(&draft.notes)
        ),
        ..for_category.clone()
    };
    (for_category, for_colour)
}

pub(crate) fn rules_in(conn: &Connection) -> Result<Vec<CategoryRule>> {
    Ok(category_rules(&categories_in(conn)?))
}

/// Fills the category of a new item that came without one. The colour is left as it is: the editor
/// and the label reader fill it before the save, and a colour the designer removed stays removed.
pub(crate) fn fill_new_draft(draft: &mut InventoryDraft, rules: &[CategoryRule]) {
    let fields = FieldInput {
        category: String::new(),
        ..field_input(draft)
    };
    if let Some(suggestion) = suggest_category(&fields, rules) {
        draft.category = suggestion.name;
    }
}

impl Library {
    pub(crate) fn category_rules(&self) -> Result<Vec<CategoryRule>> {
        rules_in(&self.c())
    }

    /// Intake for one new item that has no category yet.
    pub(crate) fn intake_new_draft(&self, draft: &mut InventoryDraft) -> Result<()> {
        let rules = self.category_rules()?;
        fill_new_draft(draft, &rules);
        Ok(())
    }

    /// The label reader's step after parsing: an empty colour is read from the name and description,
    /// and a generic category is replaced by a category of the library when a rule matches.
    pub(crate) fn intake_label_drafts(&self, drafts: &mut [InventoryDraft]) -> Result<()> {
        let rules = self.category_rules()?;
        for draft in drafts {
            let fields = field_input(draft);
            if let Some(suggestion) = suggest_category(&fields, &rules) {
                draft.category = suggestion.name;
            }
            if let Some(found) = suggest_colour(&fields) {
                if let Ok(colour) = normalized_colour(&found.colour) {
                    draft.colour = colour;
                }
            }
        }
        Ok(())
    }

    /// Suggestions for an item that is being added or edited, from this library's categories and keywords.
    pub fn suggest_fields(&self, input: &FieldInput) -> Result<FieldSuggestions> {
        let rules = self.category_rules()?;
        Ok(FieldSuggestions {
            category: suggest_category(input, &rules),
            colour: suggest_colour(input).map(|found| found.colour),
        })
    }

    /// Items in a generic category that a rule would move, and items with no colour that name one.
    /// Nothing changes until `apply_suggestions`.
    pub fn review_suggestions(&self) -> Result<Vec<ItemSuggestion>> {
        let mut conn = self.c();
        let tx = conn.transaction()?;
        let rules = rules_in(&tx)?;
        let items = tx
            .prepare(&format!("{SELECT_ITEMS} ORDER BY name COLLATE NOCASE, id"))?
            .query_map([], inventory_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        tx.commit()?;
        let mut out = Vec::new();
        for item in items {
            let (for_category, for_colour) = existing_inputs(&item.draft);
            let category = suggest_category(&for_category, &rules);
            let colour = suggest_colour(&for_colour);
            if category.is_none() && colour.is_none() {
                continue;
            }
            out.push(ItemSuggestion {
                id: item.id,
                name: item.draft.name,
                category_from: item.draft.category,
                category_reason: category
                    .as_ref()
                    .map(|c| c.reason.clone())
                    .unwrap_or_default(),
                category_to: category.map(|c| c.name),
                colour_reason: colour
                    .as_ref()
                    .map(|c| c.reason.clone())
                    .unwrap_or_default(),
                colour_to: colour.map(|c| c.colour),
            });
        }
        Ok(out)
    }

    /// Applies the ticked changes in one transaction and returns the number of items changed. The
    /// change carries only the new value, so the rule runs again on the item as it is now: a
    /// category is set only while the item is still in a generic category and the rule still gives
    /// the same target, and a colour only while the colour is still empty. Unknown ids are skipped.
    pub fn apply_suggestions(&self, changes: &[SuggestionChange]) -> Result<i64> {
        let mut conn = self.c();
        let tx = conn.transaction()?;
        let rules = rules_in(&tx)?;
        let mut changed: HashSet<&str> = HashSet::new();
        for change in changes {
            let item = tx
                .query_row(
                    &format!("{SELECT_ITEMS} WHERE id = ?1"),
                    [&change.id],
                    inventory_row,
                )
                .optional()?;
            let Some(item) = item else { continue };
            if let Some(target) = change
                .category
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
            {
                let now = suggest_category(&existing_inputs(&item.draft).0, &rules);
                if let Some(now) = now.filter(|n| n.name.eq_ignore_ascii_case(target)) {
                    tx.execute(
                        "UPDATE workshop_inventory SET category = ?2 WHERE id = ?1",
                        rusqlite::params![item.id, now.name],
                    )?;
                    changed.insert(&change.id);
                }
            }
            if let Some(colour) = change.colour.as_deref() {
                let colour = normalized_colour(colour)?;
                if !colour.is_empty() && item.draft.colour.trim().is_empty() {
                    tx.execute(
                        "UPDATE workshop_inventory SET colour = ?2 WHERE id = ?1 AND colour = ''",
                        rusqlite::params![item.id, colour],
                    )?;
                    changed.insert(&change.id);
                }
            }
        }
        tx.commit()?;
        Ok(changed.len() as i64)
    }
}

#[cfg(test)]
mod tests;
