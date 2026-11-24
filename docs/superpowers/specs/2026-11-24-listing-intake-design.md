# Listing intake: category, colour and estimated price

Status: approved by the designer on 2026-11-24 (price source chosen below). Branch `inventory-listing-intake`. Builds on 0.12.0 (columns, categories, price, open pack).

## Intent and the why

New items arrive from a label photo (Mac or phone), from a supplier listing, or typed. Today they often land in the wrong category ("Overture PETG Filament Purple" in Materials, "Gorilla Glue Sticks" in Other), have no colour even when the name says "Titan Grey", and have no price. The designer wants the intake to do the obvious work: assign the category from the words, fill the colour (with a badge), and fill an estimated price, which they will correct later if they paid less.

Findings that drive the design:
- The label reader chooses from a hard-coded list of 10 words (`CATEGORIES` in `inventory_label.rs`) and never sees the library's real categories.
- The reader fills Colour only when printed on the label; the list swatch recognises about 50 exact words.
- Supplier search reads Shopify product endpoints (`.js`) that carry a variant `price` in cents; the app discards it. `/meta.json` of each store gives its currency: canadabolts.ca CAD, 3dprintingcanada.com CAD, thepihut.com GBP.

## Goals

1. Category: the label reader is given the library's real categories, and a deterministic keyword step assigns a category from the name, brand and description when the current one is empty or generic. Rules and reasons are explicit. Per-category extra keywords can be set.
2. Colour: when Colour is empty, take it from the name or description, stored as written ("Titan Grey"); the list and editor show a badge from the shared colour dictionary, also for shades and prefixed words ("Dark Brown", "Misty Blue").
3. Price: when a supplier listing has a price in the library currency and the item has none, fill the price as an **estimate**: marked as such in the editor, the list, the footer and the MCP.
4. Existing items: a **Review suggestions** screen lists proposed category, colour and price changes with checkboxes. Nothing changes until applied.

## Non-goals

- Amazon.ca and general web price lookups (Amazon blocks automated reads and forbids scraping; a web-search API is not wanted now). Amazon stays a search link.
- Currency conversion. A listing in another currency (The Pi Hut, GBP) is shown with its price but never auto-fills the price.
- Learning from corrections, or changing categories of items the designer set on purpose.

## Anti-goals (failures even if it "works")

- Silently overriding a category or colour the designer chose. Intake only fills empty fields and replaces generic categories on new items; edits of existing items are never rewritten except through Review suggestions.
- A guessed price presented as a paid price. Estimates are always marked, and a designer edit clears the mark.
- Colour false positives that are hard to spot (every auto colour is editable and shown with its source text in Review suggestions).
- Network calls that send more than the product name, brand and part number (as today).

## Decisions

- **Generic categories** (replaced when a keyword rule fires): empty, `Other`, `Materials`, `Hardware`, `Consumables`, or a name that is not one of the library's categories. A rule never replaces any other category.
- **Category rules**: a category matches by its own name (singular and plural, case-insensitive, whole words), by the built-in words below keyed by category name, and by the designer's per-category keywords. Name matches beat description matches; the longest keyword wins; ties go to the category with more matches, then alphabetical. Built-in words (applied only when a category with that name, case-insensitively, exists in the library): Adhesives: glue, epoxy, cyanoacrylate, superglue, super glue, adhesive, cement; Filament: filament, pla, petg, abs, asa, tpu, pla+; Bolts: bolt, screw, cap screw, machine screw; Nuts: nut, lock nut, nyloc; Washers: washer; Magnets: magnet, neodymium; Tape: tape; Threaded Inserts: insert, heat-set, heat set; Tools: screwdriver, wrench, plier, pliers, tweezers; Cord & Rope: cord, rope, paracord; Electronics: arduino, raspberry pi, esp32, sensor, module, led.
- **New category names** are never invented by intake; the reader may only answer with an existing category or `Other`.
- **Colour extraction**: search the name first, then brand and description. A colour phrase is 1 to 3 contiguous words, each a base colour, alias, modifier or shade (see `shared/colours.json`), with at least one base, alias or shade; stored Title Case as written ("Titan grey" becomes "Titan Grey"). Several colours joined by `/`, `&` or `and` in the name become "Black / Red" (at most 3). Only fills an empty Colour.
- **Badge colour**: base hex; a shade uses its own hex; a modifier mixes the badge colour with black or white by its amount. Clear is shown as the existing transparent swatch. A colour text with no known word gets no badge (as today).
- **Price source**: `Draft.price_source` is `""` (entered by the designer) or `listing:<host>` (an estimate from a supplier listing). Saving a draft whose price differs from the one that came with the estimate is the designer's edit and sets `price_source` to `""`; the UI clears it when the price field is edited. The MCP adds `price_is_estimate`.
- **Which price fills**: only from listings whose currency equals the library currency. A variant whose SKU equals the item's part number wins; otherwise, when all returned variants of the top product have the same price, use it; otherwise no auto-fill (the listing cards still show prices, and "Use listing" fills the chosen listing's price when the item has none). Price per pack as bought: the listing price is for that variant, so the editor says "check the pack size".
- **Where intake runs**: (a) in `inventory_label::read_labels` for every draft (Mac, phone); (b) when a new item is saved with an empty category (core, `id.is_none()`); (c) in the editor when the designer selects a listing or a search finishes (fill-only-empty, through the command below); (d) in Review suggestions for existing items.

## The contract

TypeScript (already in `src/inventoryApi.ts`): `InventoryDraft.price_source`, `SupplierListing.price_cents`/`currency`, `CategoryInfo.keywords`, `FieldInput`, `FieldSuggestions`, `ItemSuggestion`, `SuggestionChange` and the wrappers `suggestFields`, `reviewSuggestions`, `applySuggestions`, `setCategoryKeywords`.

Rust (`corydoras-core`):

```rust
// inventory.rs: InventoryDraft gains `pub price_source: String` (serde default, "" or "listing:<host>", max 120 chars, no control chars);
// SupplierListing gains `pub price_cents: Option<i64>` and `pub currency: String` (serde default).
// inventory_categories.rs: CategoryInfo gains `pub keywords: Vec<String>`; `set_category_keywords(name, Vec<String>)` (each 1..=40 chars, at most 20, trimmed, case-insensitive unique, no control chars);
//   schema: workshop_categories.keywords TEXT (JSON array, NULL = none), additive migration.
// New module inventory_intake.rs (re-exported in lib.rs):
pub struct FieldInput { pub name: String, pub brand: String, pub description: String, pub category: String, pub colour: String }
pub struct CategorySuggestion { pub name: String, pub reason: String }   // reason like: name says "filament"
pub struct FieldSuggestions { pub category: Option<CategorySuggestion>, pub colour: Option<String> }
pub struct ItemSuggestion { pub id: String, pub name: String, pub category_from: String, pub category_to: Option<String>, pub category_reason: String,
                            pub colour_to: Option<String>, pub colour_reason: String }
pub struct SuggestionChange { pub id: String, pub category: Option<String>, pub colour: Option<String> }
impl Library {
  pub fn suggest_fields(&self, input: &FieldInput) -> Result<FieldSuggestions>;      // pure rules + this library's categories and keywords
  pub fn review_suggestions(&self) -> Result<Vec<ItemSuggestion>>;                   // items in generic categories with a rule match; items with empty colour and a colour in name/description/notes
  pub fn apply_suggestions(&self, changes: &[SuggestionChange]) -> Result<i64>;       // one transaction; each field is applied only if the item's current value still equals the one reviewed (category_from, or empty colour); returns items changed
}
pub fn suggest_category(...) and pub fn suggest_colour(text_fields...) are pure functions with unit tests.
```

Tauri commands: `suggest_inventory_fields(input)`, `review_inventory_suggestions()`, `apply_inventory_suggestions(changes)`, `set_inventory_category_keywords(name, keywords)`. `search_inventory_listings` keeps its shape; its listings gain `price_cents` and `currency`.

MCP: records gain `price_is_estimate` (bool).

## Left to the builder

Exact word-boundary matching, test fixtures, copy for empty states, layout of the Review suggestions screen (reuse the list table styles), and how the TypeScript side loads `shared/colours.json` (both `tsc` and `node --experimental-strip-types` must work; if a JSON import is not possible in both, generate a `.ts` module from the JSON with a script and a test that fails when they differ).
