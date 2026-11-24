# Inventory columns, categories and price (the QOL pass)

Status: approved by the designer on 2026-11-24 (price meaning and dimensions option chosen below). Branch `inventory-columns-pricing`.

## Intent and the why

The workshop inventory list shows five fixed columns. The designer looks through one category at a time and needs different columns in each: Filament wants Colour and has no use for Per pack (nearly every spool is one 1 kg item), Bolts want Dimensions, Electronics want Brand. The designer also wants material cost in the inventory, starting with the retail price of an item, so a design can be costed. This pass is also the quality-of-life update: anything that makes the list quicker to use belongs here.

## Goals

1. The designer chooses and orders the columns of each category view and of the "All categories" view. The choice persists in the library.
2. Views start on **Auto**: show only columns that have a value for at least one item in the view.
3. Categories are manageable: create, rename, merge, delete (only when empty), and set columns for each.
4. Items get an optional **price**. The list can show Price, Unit price and Stock value, and totals say how many items are unpriced.
5. The MCP server returns price to agents.
6. QOL: category chips with counts, `-`/`+` on On hand, duplicate item, filters for "no price" and "no count", remembered last view, keyboard shortcuts.

## Non-goals

- Price history, multiple currencies per item, supplier price scraping, cost of a whole design (a later pass; the MCP price makes it possible).
- Converting `Dimensions` text into structured specs and per-spec columns (Thread, Length). Chosen later as its own previewed tool. 51 bolts and inserts carry thread and length only as text today, so this pass shows `Dimensions` as one text column.
- Changing how the phone captures labels beyond passing `price_cents` through.

## Anti-goals (failures even if it "works")

- A hidden column hides data from search or from agents. Columns are a view setting only.
- Stock value that quietly treats a missing price as zero.
- A rename or merge that loses items, leaves an old name on some items, or breaks the label reader or MCP.
- Any change to `~/Corydoras/library.sqlite` during development or tests.
- An older Corydoras opening a library written by this version and failing (additive migration only).

## Decisions

- **Price meaning (decided by designer):** price is the retail price of one pack as bought. `unit_price = price / pack_quantity` when `pack_quantity >= 1`, else `price`. `stock_value = quantity x unit_price`, rounded to whole cents; null when `quantity` or `price` is null. A filament spool with no pack count is simply its spool price.
- **Stored as** `price_cents INTEGER` (nullable, `>= 0`), because `InventoryDraft` derives `Eq`. One currency per library, setting `workshop_currency`, default `CAD`.
- **Columns are stored in the library**, not localStorage, because libraries can be switched, moved and snapshotted.
- **Dimensions (decided):** option A. One text column, ellipsis with the full text as a tooltip.
- **Item column** is always shown and never in the picker. When Brand or Part number is a chosen column, the item caption stops repeating it.
- **Sort when the sorted column is hidden in the current view:** the view sorts by name ascending for display; the stored sort preference is kept, not cleared.
- **Category identity** is case-insensitive for create, rename and merge ("bolts" merges into "Bolts"). Display keeps the stored casing of the target.

## The contract

Column ids, the same list in Rust (`COLUMN_IDS`) and TypeScript (`columnIds`). `name` (Item) is implicit and never listed.

`category, brand, part_number, colour, dimensions, quantity, pack_quantity, price, unit_price, stock_value, notes, supplier`

A column config is `string[] | null`. `null` means Auto. A config may not contain an unknown or repeated id.

Auto resolution (TypeScript): in this order, include `category` only in the All view; `colour`, `dimensions` and `pack_quantity` when any item in the view has a value; `quantity` always; `price` when any item has a price; `stock_value` when any item has a price and a count. `brand`, `part_number`, `notes` and `supplier` are never auto, because the caption already shows brand and part number.

### Rust core (`corydoras-core`)

```rust
// inventory.rs
pub struct InventoryDraft { /* existing fields */ pub price_cents: Option<i64> } // serde default, 0..=MAX_PRICE_CENTS (10^12)
// New module inventory_categories.rs, re-exported from lib.rs
pub const COLUMN_IDS: &[&str];
pub struct CategoryInfo { pub name: String, pub count: i64, pub columns: Option<Vec<String>> }
pub struct ViewSettings { pub currency: String, pub all_columns: Option<Vec<String>> }
impl Library {
  pub fn list_categories(&self) -> Result<Vec<CategoryInfo>>;        // item categories (with counts) united with stored rows (count 0), sorted by name
  pub fn create_category(&self, name: &str) -> Result<()>;           // error if it exists (case-insensitive)
  pub fn rename_category(&self, from: &str, to: &str) -> Result<i64>; // updates items; returns items changed; if `to` exists it is a merge and the target's columns win
  pub fn delete_category(&self, name: &str) -> Result<()>;           // error if any item uses it
  pub fn set_category_columns(&self, name: &str, columns: Option<Vec<String>>) -> Result<()>; // creates the row if absent
  pub fn view_settings(&self) -> Result<ViewSettings>;
  pub fn set_all_columns(&self, columns: Option<Vec<String>>) -> Result<()>;
  pub fn set_currency(&self, code: &str) -> Result<()>;              // three uppercase ASCII letters
  pub fn set_inventory_quantity(&self, id: &str, quantity: i64) -> Result<InventoryItem>;
}
```

Schema (additive, in `db.rs` `migrate`): `workshop_inventory.price_cents INTEGER CHECK (price_cents >= 0)`; table `workshop_categories (name TEXT PRIMARY KEY COLLATE NOCASE, columns TEXT)` where `columns` is NULL (Auto) or a JSON array; settings keys `workshop_currency` and `workshop_all_columns`.

`inventory_row` indices, the list `SELECT`, the save `UPDATE`/`INSERT` and the batch `INSERT` all change together. Normalization: the default category stays `Other`.

### Tauri commands (`src-tauri/src/inventory.rs`, registered in `main.rs`)

`list_inventory_categories() -> CategoryInfo[]`, `create_inventory_category(name)`, `rename_inventory_category(from, to) -> number`, `delete_inventory_category(name)`, `set_inventory_category_columns(name, columns)`, `get_inventory_view_settings() -> ViewSettings`, `set_inventory_all_columns(columns)`, `set_inventory_currency(code)`, `set_inventory_quantity(id, quantity) -> InventoryItem`. All go through `enabled_library`.

### MCP (`corydoras-mcp/src/workshop.rs`)

Item records gain `price_cents` (integer or null), `currency`, and `unit_price` (decimal string with up to 4 places, or null). The search and item tools' descriptions say that price is per pack as bought. Price never counts as stock confirmation.

### TypeScript

`src/inventoryApi.ts` carries the types and `invoke` wrappers (written in step 0). `src/components/workshopColumns.ts` holds the column definitions, Auto resolution, `unitPriceCents`, `stockValueCents`, `formatMoney`, `viewTotals`. `workshopList.ts` keeps its exports and widens `SortField` to a column id.

## Approaches considered

1. **Columns as a per-category table in the library (chosen).** Survives library switch and snapshot; one source of truth with categories; additive schema.
2. Columns in localStorage. Simplest, but is per machine and lost on a library switch; rejected.
3. Make categories a foreign-keyed table and migrate items to ids. Cleaner in theory, but every reader of `category` text (label reader, MCP, phone, search) would change; rejected for this pass. The text stays the identity and the new table only holds config and empty categories.

## Left to the builder

Visual details of the picker popover and manager (use existing `workshop.css` patterns), copy for empty states, the exact money formatting for sub-dollar unit prices (three or four decimals), and test fixtures.

## Addendum, 2026-11-24: open pack (partial stock)

Requested by the designer after the first pass. Chosen: a **separate open-pack field**, not a unit on On hand. A filament spool is 1000 g full; the designer wants to say "I have 1 spool and 100 g is left"; a magnet pack of 25 with 6 left already works as On hand 6 with Per pack 25 (pieces) and needs no change.

Fields on `InventoryDraft` (all optional, additive migration, serde default):
- `measure_unit: String` one of `""` (counted pieces, the default), `g`, `mL`, `m`.
- `measure_full: Option<i64>` amount in one full pack, in `measure_unit`, `1..=MAX_QUANTITY`.
- `measure_left: Option<i64>` amount in the one opened pack, `0..=measure_full`.

Rules:
- `quantity` still counts whole packs and **includes** the opened one. Total amount = `(quantity - 1) * measure_full + measure_left` when `measure_left` is set and `quantity >= 1`; `quantity * measure_full` when only `measure_full` is set; otherwise unknown.
- A unit with `measure_left` requires `measure_full`; `measure_full` or `measure_left` without a unit is rejected. A `measure_left` larger than `measure_full` is rejected.
- `quantity == 0` clears `measure_left` (in save normalization and in `set_inventory_quantity`). An unknown quantity (`None`) keeps it.
- A **decrease** of the quantity through `set_inventory_quantity` (the `-` stepper) also clears `measure_left`: the opened pack is assumed to be the one used up, so the remaining packs count as full (2 spools with 100 g left, press `-`: 1 spool, no open amount, 1000 g). An increase, an unchanged count or an unknown stored count keeps it. `save_inventory` (the editor's full-form save) does not apply this rule and keeps what the designer typed. The optimistic stepper uses the same rule (`stepQuantityItem`).
- Pricing: when `measure_unit` is set and `measure_full` exists, `unit_price = price / measure_full` per `measure_unit` (for example per gram) and `stock_value = round(total_amount x unit_price)`. Otherwise the rules above apply unchanged.
- New column id `open_amount` (label "Open pack"), shown as "100 of 1000 g" with a thin gauge. Auto resolution includes it when any item in the view has `measure_left`. It sorts by the fraction left. `COLUMN_IDS`/`columnIds` gain it.
- New command `set_inventory_open_amount(id, left: Option<i64>) -> InventoryItem` (the list's quick edit). Core: `Library::set_open_amount`.
- Editor: choosing a unit shows "Full pack" and "Left in the opened pack". "Full pack" is prefilled from a `net_weight` spec when there is one (1 kg gives 1000 g). Existing items are never converted automatically.
- MCP: records gain `measure_unit`, `measure_full`, `measure_left`, `total_amount` (integer or null). `required_unit` also accepts the item's own measure unit, comparing the requested amount with `total_amount`; the descriptions say the amount is what the designer recorded, not a weighing, so a shortage is reported and a pass is "recorded, not verified by weight".
- Non-goals: several opened packs, kilograms (use grams), decimal amounts, conversion between units.
