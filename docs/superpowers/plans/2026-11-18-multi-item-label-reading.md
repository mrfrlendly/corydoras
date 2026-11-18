# Multi-item label reading Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** One photo of a kit label, size chart, several products or several filament spools produces one reviewed, unsaved inventory draft per item, each with structured specs that the MCP server returns to agents.

**Architecture:** A new `inventory_specs` module in `corydoras-core` holds the spec model and the ISO metric standards table. `InventoryDraft` gains `specs`; a new `specs` column stores them as JSON. The single-label reader becomes one multi-item reader with per-row verification. A batch save stores N drafts and one shared photo in one transaction. A review list in the React UI shows the drafts, and the MCP tools return the specs.

**Tech Stack:** Rust (workspace: `corydoras-core`, `corydoras-mcp`, `src-tauri` crate `corydoras`), SQLite via rusqlite, Tauri 2 IPC, React 19 + TypeScript, `node --test` for UI helpers.

**Spec:** `docs/superpowers/specs/2026-11-18-multi-item-label-reading-design.md`. Read it first. It holds the reasons behind every rule below.

## Global Constraints

- Work on a new branch `feature/multi-item-labels` cut from `main`. The first commit also adds the untracked `docs/superpowers/` files. Never name a commit "Version 0.x.0" (that is the release convention). Do not push.
- Metric only. Spec units are one of `""`, `mm`, `g`, `kg`, `°C`. No imperial threads.
- Roles: `thread`, `pitch`, `length`, `outer_diameter`, `hole_diameter` (installation hole of an insert), `clearance_hole`, `tap_drill`, `diameter`, `net_weight`, `material`, `nozzle_temp`, `bed_temp`, `drying`, `other` (with a short `name`).
- Sources: `label`, `notation`, `standard`, `entered`. `standard` rows come only from the code table, never from the model. A printed value always beats the table. Nothing is derived for threaded inserts.
- Evidence: for `label` and `notation`, the spec's value appears inside its own evidence, and that evidence appears in the item's own row. `shared_text` supports only brand, material, colour and finish.
- Limits: at most 40 items per photo, image long side 3072 px, reply allowance 10,000 tokens, accepted reply 128 KiB, batch metadata limit 512 KiB. A cut-off reply is an error.
- `quantity` counts pieces for hardware and spools for filament. Quantity on hand is never inferred and is blank on every draft.
- Standard rows: ISO 261 coarse pitch for M1.6–M12 when no pitch is printed; ISO 273 medium clearance and tap drill for screws and bolts only.
- Existing libraries open without a write from the MCP read path: a missing `specs` column reads as no specs.
- The global pre-commit hook blocks any staged line that pairs the word "token" with a long quoted string, so test data and docs must avoid that shape.

## Review Focus

Each line below has a test in the task named in brackets.

1. Two rows with identical text (two spools of the same filament) stay two separate drafts. [Task 4]
2. A printed pitch (`M8x1`) beats the table's coarse pitch, and no standard pitch is added. [Task 1]
3. A batch with one invalid draft saves nothing and names the draft number. [Task 5]
4. A library made before this change reads through the MCP tools with `specs: []` and no write. [Task 3]
5. A comma decimal on the label (`5,7 mm`) still verifies a value written `5.7`. [Task 4]

---

### Task 1: Spec model and standards table

**Files:**
- Create: `crates/corydoras-core/src/inventory_specs.rs`
- Modify: `crates/corydoras-core/src/lib.rs` (add `pub mod inventory_specs;` next to `inventory`)
- Test: `crates/corydoras-core/src/inventory_specs/tests.rs` (use `#[cfg(test)] mod tests;` like `inventory_label.rs`)

**Interfaces:**
- Produces, in `inventory_specs`:
  - `pub struct Spec { pub role: SpecRole, pub name: String, pub value: String, pub unit: String, pub tolerance: String, pub source: SpecSource, pub evidence: String }` deriving `Clone, Debug, PartialEq, Eq, Deserialize, Serialize`. `name` is empty unless the role is `Other`.
  - `pub enum SpecRole`, `pub enum SpecSource`, `pub enum ProductType { Screw, Bolt, Nut, Washer, Insert, Standoff, Filament, Other }`, all `#[serde(rename_all = "snake_case")]`.
  - `pub fn summary(specs: &[Spec]) -> String`
  - `pub fn with_standard(specs: Vec<Spec>, product: ProductType) -> Vec<Spec>` (drafting)
  - `pub fn refresh_standard(specs: Vec<Spec>) -> Vec<Spec>` (every save)
  - `pub fn normalized(specs: &[Spec]) -> anyhow::Result<Vec<Spec>>`

- [ ] **Step 1: Write failing tests** in `tests.rs`:
  - `summary_lists_specs_in_order`: thread `M3`, pitch `0.5 mm`, length `5.7 mm` gives exactly `"M3 thread; pitch 0.5 mm; length 5.7 mm"`. An empty list gives `""`.
  - `a_screw_gets_pitch_clearance_and_tap_drill_from_the_table`: `with_standard([thread M3 label], Screw)` adds `pitch 0.5 mm`, `clearance_hole 3.4 mm`, `tap_drill 2.5 mm`, all `source: standard`. M4 gives `0.7`, `4.5`, `3.3`.
  - `an_insert_gets_pitch_only`: `with_standard([thread M3], Insert)` adds only `pitch 0.5 mm`. Filament with no thread gets nothing.
  - `a_printed_pitch_beats_the_table` (Review Focus 2): `[thread M8, pitch 1 mm source label]` with `Screw` gets no `standard` pitch, but still gets the clearance and tap rows.
  - `refresh_follows_the_thread`: standard rows `[M3 pitch, clearance, tap]` with the thread edited to `M4` and source `entered` become `0.7, 4.5, 3.3`. With the thread removed, every `standard` row is dropped. A `standard` pitch alone never grows clearance or tap rows.
  - `normalized_rejects_imperial_units_and_overlong_values`: unit `in`, a value over 120 chars, more than 40 specs, or a control character in any text field each fail. Roles other than `Other` with a non-empty `name` fail.
- [ ] **Step 2:** Run `cargo test -p corydoras-core inventory_specs`. Expected: compile error or FAIL.
- [ ] **Step 3: Implement** the items above. The table is one `const` array of `(thread, pitch, clearance, tap)` for M1.6, M2, M2.5, M3, M4, M5, M6, M8, M10, M12 (pitches 0.35, 0.4, 0.45, 0.5, 0.7, 0.8, 1.0, 1.25, 1.5, 1.75). Evidence for a standard row names the table, for example `ISO 261 coarse pitch, M3`. `summary` renders thread as `"{value} thread"`, other roles as `"{role words} {value} {unit}"`.
- [ ] **Step 4:** Run the same command. Expected: PASS.
- [ ] **Step 5: Commit** `feat: add inventory spec model and ISO metric table` (include `docs/superpowers/`).

### Task 2: Store specs with each item

**Files:**
- Modify: `crates/corydoras-core/src/inventory.rs` (`InventoryDraft`, `normalized`, `normalized_for_review`, `inventory_row`, `list_inventory`, `save_inventory_with_photo`)
- Modify: `crates/corydoras-core/src/db.rs` (`CREATE TABLE workshop_inventory`, `migrate` steps)
- Modify: `crates/corydoras-core/src/inventory_import_media.rs` (every SELECT that uses `inventory_row`)
- Test: `crates/corydoras-core/tests/workshop_inventory.rs`

**Interfaces:**
- Consumes: `inventory_specs::{Spec, summary, refresh_standard, normalized}`.
- Produces: `InventoryDraft.specs: Vec<Spec>` (`#[serde(default)]`). `inventory_row` reads `specs` from column index 14, after `photo_key`.

- [ ] **Step 1: Write failing tests** in `tests/workshop_inventory.rs`:
  - `specs_persist_and_generate_the_dimensions_text`: save a draft with `dimensions: "ignored"` and specs `[thread M3, length 5.7 mm]`. The returned item and `list_inventory()` both have `dimensions == "M3 thread; length 5.7 mm"` and equal specs.
  - `an_item_without_specs_keeps_its_typed_dimensions`: `specs` empty and `dimensions: "Length 20 mm"` stay as typed.
  - `saving_refreshes_standard_rows`: save thread `M3` with a `standard` pitch, then update with the thread `M4` as `entered`. The stored pitch is `0.7`.
  - `a_library_without_the_specs_column_migrates`: build a library, drop the column with `ALTER TABLE ... DROP COLUMN specs`, reopen with `Library::open`, and confirm `list_inventory` works and returns `specs == []`.
- [ ] **Step 2:** Run `cargo test -p corydoras-core --test workshop_inventory`. Expected: FAIL.
- [ ] **Step 3: Implement.** Add `specs TEXT NOT NULL DEFAULT '[]'` to `CREATE TABLE` and as a `migrate` step with the same `has_column` pattern. Store specs as JSON text. `normalized()` (the save path) applies `refresh_standard`, then sets `dimensions = summary(specs)` when specs are non-empty. `normalized_for_review()` only validates (`inventory_specs::normalized`). Update every SELECT that feeds `inventory_row` (`grep -rn "inventory_row" crates`), the INSERT and UPDATE, and check `snapshot.rs` and `storage.rs` for hard-coded column lists.
- [ ] **Step 4:** Run `cargo test -p corydoras-core`. Expected: PASS, including the existing inventory tests.
- [ ] **Step 5: Commit** `feat: store structured specs with inventory items`.

### Task 3: Agents see the specs

**Files:**
- Modify: `crates/corydoras-core/src/agent_inventory.rs` (`select_items`, `summary`, `item`, the module doc comment on line 2)
- Modify: `crates/corydoras-mcp/src/workshop.rs` (the `required_quantity` description at line 36 and both tool descriptions)
- Modify: `crates/corydoras-core/src/agent.rs` (the instruction text at line 13) and `README.md` (line 125 area)
- Test: `crates/corydoras-core/src/agent_inventory/tests.rs`, `crates/corydoras-mcp/tests/workshop.rs`

**Interfaces:**
- Consumes: `InventoryDraft.specs` and the `specs` column (index 14 in `select_items`).
- Produces: each item JSON from `search` and `item` has `"specs": [{role, name, value, unit, tolerance, source, evidence}]` and keeps `"dimensions"`.

- [ ] **Step 1: Write failing tests:**
  - `search_and_item_return_specs_with_sources`: save an item with a `label` spec and an `entered` spec. Both `search` and `item` return `specs[0].source == "label"` and `specs[1].source == "entered"`, plus the unchanged `dimensions` text.
  - `an_older_library_returns_empty_specs_without_a_write` (Review Focus 4): create the schema without the `specs` column, insert an item row by SQL, run `item` and `search`, and assert `specs == []` and the database file's `PRAGMA data_version` or modified time did not change.
  - In the MCP crate: `descriptions_explain_the_sources_and_the_piece_rule`: the `get_workshop_item` description mentions `notation`, `standard`, `entered`, and that for hardware sold in packs one item is one piece.
- [ ] **Step 2:** Run `cargo test -p corydoras-core agent_inventory` and `cargo test -p corydoras-mcp`. Expected: FAIL.
- [ ] **Step 3: Implement.** Extend `select_items` with the same missing-column tolerance as `colour` (`'[]'` when absent). The `required_quantity` description gains: "For hardware sold in packs, one item is one piece." Describe the sources as in the spec: `label` printed, `notation` digits printed but role read from a size notation (confirm a critical fit), `entered` typed by the designer, `standard` a table value that is not measured and must not be used as confirmed fit data. Update README and `agent.rs` wording to match.
- [ ] **Step 4:** Run `cargo test -p corydoras-core -p corydoras-mcp`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: return structured specs through the MCP inventory tools`.

### Task 4: Multi-item reader with per-row verification

**Files:**
- Modify: `crates/corydoras-core/src/inventory_label.rs`
- Test: `crates/corydoras-core/src/inventory_label/tests.rs` (port the existing single-label cases to the new reply shape; keep their intent)

**Interfaces:**
- Consumes: `inventory_specs::{Spec, SpecRole, SpecSource, ProductType, with_standard}`, existing `printed_phrase`, `printed_count`, `folded`.
- Produces:
  - `pub const MAX_ITEMS: usize = 40;`
  - `fn parse_labels(reply: &str) -> Result<Vec<InventoryDraft>>`
  - `pub async fn read_labels(library: &Library, cfg: &AiConfig, bytes: &[u8]) -> Result<Vec<InventoryDraft>>` (replaces `read_label`; `request_label` becomes `request_labels`).
  - Reply JSON (all structs `deny_unknown_fields`): `{ shared_text, items: [ { row_text, name, category, product_type, brand, part_number, colour, pack_quantity, pack_evidence, notes: [..], specs: [ { role, name, value, unit, tolerance, basis: "stated" | "notation", evidence } ] } ] }`.

- [ ] **Step 1: Write failing tests** (table-style, like the existing ones):
  - `each_row_is_checked_against_its_own_text`: two items, rows `"M3 x 8 mm 50 pcs"` and `"M4 x 10 mm 40 pcs"`. If the M3 item claims length evidence `"10 mm"`, that spec is dropped while the same item's `"8 mm"` spec stays. The M4 item keeps `"10 mm"`.
  - `shared_text_supports_only_brand_material_colour_and_finish`: a length, a pack count and a thread whose evidence appears only in `shared_text` are dropped. Brand `Acme` and material `PETG` printed only in `shared_text` are kept.
  - `value_must_appear_inside_its_evidence`: value `"8"` with evidence `"M3 x 12"` is dropped. A comma decimal case (Review Focus 5): evidence `"5,7 mm"` verifies value `"5.7"`.
  - `notation_specs_are_marked_notation`: row `"M2x2x3.2"` with specs thread `M2`, length `2`, outer_diameter `3.2`, basis `notation`, evidence `"M2x2x3.2"` come out as `source: notation` and each value contained in its evidence. Basis `stated` comes out as `label`.
  - `a_screw_row_gets_standard_rows_and_an_insert_row_gets_pitch_only`: through `with_standard`, using the product type from the reply.
  - `identical_rows_stay_separate_drafts` (Review Focus 1): two filament items with the same row text give two drafts.
  - `a_filament_spool_keeps_printed_colour_only`: colour `"Translucent Orange"` printed → kept; colour `"Clear"` not printed → empty. Material, diameter `1.75 mm`, `net_weight 1 kg` verify against the row.
  - `item_count_and_reply_size_are_limited`: 41 items, a reply over 128 KiB, an empty `items` list, and a reply whose rows are all unreadable each fail with a message that names the fix (closer photo or one section at a time).
  - `quantity_is_never_set`: every draft has `quantity == None`.
  - Async, mock server (existing `serve` pattern): `a_cut_off_reply_is_an_error`; `the_request_asks_for_10000_tokens_and_a_3072_px_image`; `usage_is_recorded_as_inventory_label`; budget and non-vision failures happen before any request (port the existing test).
- [ ] **Step 2:** Run `cargo test -p corydoras-core inventory_label`. Expected: FAIL.
- [ ] **Step 3: Implement.** Verification is per item: allowed text for brand, colour and material specs is `row_text + shared_text`; everything else is `row_text` only. Failing fields and specs are dropped, as the single-label reader does today. Normalize `,` to `.` between digits before the value-in-evidence check. Drop an item whose `row_text` is empty or that has no name, brand, spec or pack count left; error if no item remains. Rewrite `SYSTEM` and `SCHEMA` for the reply shape: say that the model must return `row_text` per item, mark `notation` only for digits whose meaning comes from a notation, and leave `hole_diameter` empty unless printed. Set the image long side to 3072 and the allowance to 10,000 tokens. Keep `cut_off()` and the budget and vision checks.
- [ ] **Step 4:** Run `cargo test -p corydoras-core inventory_label`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: read several items from one label photo`.

### Task 5: Batch save

**Files:**
- Modify: `crates/corydoras-core/src/inventory.rs` (add `save_inventory_batch`)
- Modify: `src-tauri/src/inventory.rs` (new command, `read_inventory_label` returns a list, frame limit)
- Modify: `src-tauri/src/main.rs` (register `inventory::save_inventory_batch`, next to line 1231) and `src-tauri/src/ipc_tests.rs` (the handler list at line 134)
- Test: `crates/corydoras-core/tests/workshop_photo.rs`, `src-tauri/src/ipc_tests.rs`

**Interfaces:**
- Consumes: `inventory_label::read_labels`, `InventoryDraft`.
- Produces:
  - `pub const MAX_BATCH: usize = 40;`
  - `Library::save_inventory_batch(&self, drafts: &[InventoryDraft], photo: &[u8]) -> Result<Vec<InventoryItem>>`: new items only, one transaction, one stored photo shared by all rows.
  - Tauri `save_inventory_batch(state, request: tauri::ipc::Request) -> CmdResult<Vec<InventoryItem>>`, frame: u32 big-endian length, JSON `{"drafts": [...]}`, then photo bytes. `unpack_photo`'s `MAX_FORM` becomes 512 KiB.
  - Tauri `read_inventory_label` now returns `CmdResult<Vec<InventoryDraft>>`.

- [ ] **Step 1: Write failing tests:**
  - `a_batch_shares_one_photo`: 3 drafts and one PNG give 3 rows with the same `photo_key` and exactly 1 row in `workshop_photos`.
  - `a_batch_with_a_bad_draft_saves_nothing` (Review Focus 3): the second draft has an empty name. The call errors with a message containing `Item 2`, and `list_inventory()` is empty.
  - `a_batch_is_limited`: 0 drafts and 41 drafts both error.
  - IPC: `save_inventory_batch` with an empty body errors; `read_inventory_label` with an empty body still errors (the existing check).
- [ ] **Step 2:** Run `cargo test -p corydoras-core --test workshop_photo` and `cargo test -p corydoras ipc`. Expected: FAIL.
- [ ] **Step 3: Implement.** Decode the photo (`prepare_photo`) and normalize every draft before taking the lock, prefixing normalize errors with `Item {n}: `. Use one `transaction()`, `store_photo`, then an INSERT per draft. The Tauri command takes the same single-slot semaphore as `save_inventory_photo` and runs on `spawn_blocking`.
- [ ] **Step 4:** Run `cargo test -p corydoras-core -p corydoras`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: save several inventory drafts with one photo`.

### Task 6: Frontend types and form helpers

**Files:**
- Modify: `src/inventoryApi.ts`, `src/components/workshopForm.ts`
- Test: `tests/workshop-inventory.test.mjs`

**Interfaces:**
- Produces, in `inventoryApi.ts`: `type SpecSource = 'label' | 'notation' | 'standard' | 'entered'`; `interface Spec { role: string; name: string; value: string; unit: string; tolerance: string; source: SpecSource; evidence: string }`; `InventoryDraft.specs: Spec[]`; `inventoryApi.readLabels(file: File): Promise<InventoryDraft[]>` (replaces `readLabel`); `inventoryApi.saveBatch(drafts: InventoryDraft[], photo: File): Promise<InventoryItem[]>` (same frame as Task 5).
- Produces, in `workshopForm.ts`: `WorkshopForm.specs: Spec[]`; `specSummary(specs: Spec[]): string`; `editSpec(specs: Spec[], index: number, patch: Partial<Spec>): Spec[]`; `removeSpec(specs: Spec[], index: number): Spec[]`; `addPack(form: WorkshopForm): WorkshopForm`; `batchDrafts(forms: WorkshopForm[]): InventoryDraft[]`.

- [ ] **Step 1: Write failing tests** in `tests/workshop-inventory.test.mjs`:
  - `specSummary` of thread `M3`, pitch `0.5 mm`, length `5.7 mm` equals `"M3 thread; pitch 0.5 mm; length 5.7 mm"` (same vector as Task 1).
  - `editSpec` changing the value of a `label` or `notation` or `standard` spec returns it with `source: 'entered'` and keeps the evidence; editing only `evidence` is not allowed (ignored).
  - `addPack`: `quantity ''` and `pack_quantity '50'` gives `'50'`; `quantity '20'` gives `'70'`; blank `pack_quantity` throws `Enter the quantity per pack first.`
  - `batchDrafts` fails with a message starting `Item 2:` when the second form has no name, and sets `quantity: null` when blank.
  - `reviewLabel` keeps the form's specs when it already has some, and takes the label's specs when it has none.
- [ ] **Step 2:** Run `npm run test:workshop`. Expected: FAIL.
- [ ] **Step 3: Implement** the helpers. `blankForm`, `editForm` and `saveDraft` carry `specs`. `specSummary` is display only; the backend result is authoritative on save.
- [ ] **Step 4:** Run `npm run test:workshop`. Expected: PASS.
- [ ] **Step 5: Commit** `feat: add spec helpers and batch API to the inventory UI`.

### Task 7: Review list and editor integration

**Files:**
- Create: `src/components/WorkshopPhotoReview.tsx`, `src/components/WorkshopSpecsTable.tsx`
- Modify: `src/components/WorkshopInventory.tsx` (header button **Add from photo**, review state, focus return like `closeEditor`), `src/components/WorkshopItemEditor.tsx` (`readLabels`, specs table, read-only dimensions when specs exist), `src/components/workshop.css`

**Interfaces:**
- Consumes: Task 6 helpers. `WorkshopPhotoReview` props: `{ onSaved(items: InventoryItem[]): void; onCancel(): void; categories: string[] }`. `WorkshopSpecsTable` props: `{ specs: Spec[]; onChange(specs: Spec[]): void; disabled: boolean }`.

- [ ] **Step 1: Build the review list.** Flow: choose a photo (same formats and 12 MiB check as the editor), `readLabels`, then one card per draft with the normal fields, the specs table, **Discard**, **I bought one pack** and a **Save N items** button that calls `saveBatch` with the kept cards and the photo. The review shows `row` evidence beside each spec and the source badges: "printed", "printed, role read from notation", "standard (not measured)", "entered". Cancel with unsaved cards asks for confirmation. Saving returns the new items to `WorkshopInventory` (`setItems`, toast `N items saved.`).
- [ ] **Step 2: Wire the editor.** **Choose label photo** calls `readLabels`. With one item it fills the form as today; with several it shows "This photo shows N items" and an **Open them in the review list** button, handing the file and drafts over without a second provider call. The Dimensions field is read-only and shows `specSummary` whenever the form has specs.
- [ ] **Step 3:** Run `npm run build` (runs `tsc -b`) and `npm run test:workshop`. Expected: both succeed.
- [ ] **Step 4: Live check** on a copy of the library with a scratch bundle id and `CORYDORAS_CONFIG_DIR`, not the designer's library: use the mock-provider recipe from the project's live-check notes. Confirm: three drafts appear for a three-row reply; discard one and save two; the two share one photo thumbnail; a spec edit turns its badge into "entered"; Cancel with unsaved cards asks first; the single-item editor still fills one form.
- [ ] **Step 5: Commit** `feat: review and save several items from one label photo`.

### Task 8: Acceptance run (needs the designer's go-ahead)

**Files:**
- Create: `crates/corydoras-core/examples/read_label_photos.rs`
- Modify: `docs/superpowers/specs/2026-11-18-multi-item-label-reading-design.md` (record tuned limits and results)

**Interfaces:**
- Consumes: `read_labels`. The example takes a photos directory, an output JSON path and optional photo names, and uses the library's configured analysis provider.

- [ ] **Step 1:** Write the example. It prints, per photo: item count, each item's name and specs with sources, and flags quantities (must be none).
- [ ] **Step 2: Ask the designer** which of the 29 photos in `~/Corydoras/imports/workshop-catalogue-2026-11-14/photos` to run. The run spends their token budget. Do not run it without that answer.
- [ ] **Step 3:** Run it on the chosen photos and compare with the catalogue records that reference each photo (the catalogue's `convention` roles are a comparison, not ground truth). Check each acceptance line in the spec.
- [ ] **Step 4:** If a limit does not hold, change the constants in `inventory_label.rs` (lower the item cap before shrinking evidence), update the spec's Limits line, and re-run the Task 4 tests.
- [ ] **Step 5: Commit** `test: add a live label-photo acceptance runner`.

---

## Self-review

- **Spec coverage:** goals 1–5 map to Tasks 4, 1–3, 4, 5–7 and 4–5. Stock rule: Task 3 wording and Task 6 `addPack`. Specs, sources, standards table, editing rules: Tasks 1, 2, 6. Reading, limits, per-row checks, cut-off: Task 4. Batch save: Task 5. Storage and MCP: Tasks 2–3. Screens: Task 7. Acceptance set: Task 8. Non-goals have no task by design.
- **Type consistency:** `Spec`, `SpecRole`, `SpecSource` (Task 1) are used unchanged in Tasks 2–4; the TS `Spec` (Task 6) mirrors the serde field names; `specSummary` and `summary` share one test vector.
- **Known soft spot:** the ISO table values in Task 1 should be checked against ISO 261 and ISO 273 by the implementer before commit.
