# Multi-item label reading for the workshop inventory

Status: draft for approval, 2026-11-18. Nothing is built yet.

## Intent and the why

The designer buys loose hardware (threaded inserts, screws, nuts, washers) and filament, mostly in multipacks and assortment kits. Today one photo makes one inventory draft, so a kit with a chart of ten sizes means ten photos or ten hand-typed records.

The goal is **accurate dimensions that CAD agents can read**. Agents reach the inventory through the Corydoras MCP server (`search_workshop_inventory`, `get_workshop_item`). If the inventory carries trustworthy sizes, the designer stops measuring the same insert on every project.

So: give the app a photo of a kit label, a size chart, several products, or several filament spools. It returns one **unsaved draft per size or product**, each with its specs and the printed evidence for them. The designer reviews and saves.

## Goals

1. One photo can produce many drafts (up to 40), each checked against its own row of the label.
2. Each item stores **structured specs** (role, value, unit, tolerance, source, evidence) that the MCP server returns to agents. Agents can tell a printed number from a derived or typed one.
3. The same reader handles hardware labels and filament spool labels.
4. A review screen lets the designer edit, discard or save each draft, or save them all in one step.
5. Stock is never inferred. Quantity on hand is entered by the designer.

## Non-goals

- Backfilling structured specs for the 104 items imported on 2026-11-14.
- Kit parent and child records (the old catalogue's grouping). It can be added later on top of this design.
- Tracking remaining filament mass. Spools are counted as whole spools. A "remaining grams" field is a separate later change.
- A supplier search for each draft in batch mode. The existing single-item supplier search stays as it is.
- Imperial threads (`#6-32`, `1/4-20`). The imported catalogue holds none. Metric only.
- Saving anything without the designer's review.

## Anti-goals (failures even if the feature "works")

- An agent is told a derived or typed number is a confirmed measurement.
- A stock count is raised by inference from a pack count.
- An item gets a value printed on a different row (for example the M4 length on the M3 row).
- A reply cut off mid-list parses as a complete list.

## Approaches considered

| | What it is | Trade-off |
|---|---|---|
| A | Several drafts; dimensions stay free text, provenance written into the text | Smallest change. Agents must parse sentences, which is the problem being solved. |
| **B (chosen)** | Several drafts plus a structured `specs` list stored with each item | One additive column and a new review screen. Agents get clean data with provenance. |
| C | B plus kit parent and child records | More schema and UI that agents do not read. Can be added on B later. |

The designer chose B on 2026-11-18, and asked for filament spools to be covered by the same design.

## The design

### Stock rule (what `quantity` counts)

`quantity` counts **the unit the item is used in**:

- **Loose hardware: pieces.** `pack_quantity` is pieces per pack. "I bought one pack" adds `pack_quantity` to `quantity`.
- **Filament: spools.** `pack_quantity` stays empty. One spool is one item.

This is what a CAD agent needs ("I need 8 screws"). The MCP tool descriptions and the server's instructions string both say "whole items" or "whole units of this exact item"; each gets one added sentence: for hardware sold in packs, one item is one piece. In the designer's library (checked 2026-11-18, read-only), 101 items are recorded, 78 have a pack quantity, and only 2 have a quantity on hand. So few existing records could be counting packs, and this design does not rewrite them. The designer should check those two.

### Specs

Each item has a list of specs:

```
{ role, value, unit, tolerance, source, evidence }
```

- **Roles:** `thread`, `pitch`, `length`, `outer_diameter`, `hole_diameter` (the installation hole of an insert), `clearance_hole` and `tap_drill` (for screws and bolts, from the table), `diameter`, `net_weight`, `material`, `nozzle_temp`, `bed_temp`, `drying`, and `other` (with a short name). Metric units only.
- **Sources:**
  - `label`: the value and its role are both printed, for example "Length 8 mm". `evidence` is the printed phrase.
  - `notation`: the **number is printed but its role is read from a size notation**, for example `M2x2x3.2` read as thread, length and outer diameter. `evidence` is the marking. This is the common case for inserts and nuts. The review screen shows it as "printed, role read from notation", and the MCP descriptions explain it: the digits are printed, the meaning is a convention. Agents should confirm a critical fit.
  - `standard`: filled by the app from a built-in ISO metric table, never by the model. `evidence` names the table row. Not measured.
  - `entered`: typed or edited by the designer in the review screen. It outranks `standard`.
- **Evidence rule:** for `label` and `notation`, the spec's value must appear inside its own evidence phrase, and that phrase must appear in the item's row (see Reading a photo).
- A printed value always beats the table (for example `M4-.70`, or a printed fine pitch).
- **Editing:** a designer edit to a `label` or `notation` value saves the spec as `entered`. When the thread of an item is edited or removed, its `standard` rows are recomputed from the new thread, or dropped if there is none. Table rows for screws and bolts depend on the item's product type, which is not stored. The table runs only when drafting, and later edits leave existing `standard` rows alone unless the thread changes.

**Standards table, metric only.** Coarse pitch for each printed `M` thread from M1.6 to M12 when no pitch is printed. Clearance hole and tap drill, for screws and bolts only. **Nothing is derived for threaded inserts**: their hole and outer diameter depend on the product. They stay blank unless the label prints them or the designer enters them once. A measured value saved as `entered` is the intended way to close that gap.

**Source of truth.** When an item has specs, the existing `dimensions` text is generated from them on every save (for example `M3 thread; pitch 0.5 mm; length 5.7 mm`). The editor shows it read-only. Search keeps matching on that text. An item with no specs (older or hand-typed records) keeps the editable text field exactly as today. The two cannot disagree.

### Reading a photo

- One new reader replaces the single-label reader. It returns a list of drafts.
- Reply shape: `shared_text` (text common to the whole photo) and `items[]`. `shared_text` may support only brand, material, colour and finish. Numeric specs and pack counts need evidence in the item's own `row_text`, so a reply cannot put the whole label in `shared_text` to defeat the per-row check. Each item has `row_text` (its own printed row), name, category, `product_type` (screw, bolt, nut, washer, insert, standoff, filament, other), brand, part number, colour, pack quantity with `pack_evidence`, notes, and specs with evidence.
- **Verification is per item.** Each field and each spec's evidence must appear in that item's `row_text` plus `shared_text`, never in another item's row. This is a consistency check against the model's own transcript, not ground truth, so the review screen shows the evidence beside each value.
- Existing rules carry over: colour only from printed text (the label's ink or swatch is ignored, and translucent PETG is not recorded as clear unless printed); pack counts need a printed quantity phrase; unknown fields stay empty; image text is data, not instructions.
- **Limits (my starting numbers, to be tuned on the test set):** at most 40 items per photo; image long side 3072 px (was 2048); reply allowance about 10,000 tokens (was 1,600), about 250 per item for 40 items with their evidence; accepted reply size 128 KiB (was 32 KiB). If the numbers stop agreeing on the test set, lower the item cap rather than shrinking the evidence. The provider layer may halve the output cap after a refusal, and models have their own output limits.
- **A cut-off reply is an error**, as today. The message says to try a closer photo or one section of the label at a time. No partial list is shown as complete.
- Budget, vision check and usage logging (`inventory_label`) are unchanged.

### Saving

- A new command saves **one photo and N drafts in one transaction**. All drafts share the stored photo (photos are already stored once by hash). All succeed or none are saved.
- The request frame carries a metadata block (limit raised from 128 KiB to cover 40 drafts) plus one photo. One decode slot protects against concurrent saves as the single-item command does.
- Quantity on hand is blank on every draft until the designer enters it or presses "I bought one pack".

### Storage and the MCP server

- New column `specs TEXT NOT NULL DEFAULT '[]'` on `workshop_inventory`, added with the existing missing-column pattern in `db.rs`. The agent read path (`select_items` in `agent_inventory.rs`) tolerates its absence in older libraries, as it does for `colour` and `photo_key`. `inventory_row` indexes are updated together with every SELECT.
- `get_workshop_item` and `search_workshop_inventory` return `specs`, and keep `dimensions`. The tool descriptions explain the sources: `label` is printed; `entered` is the designer's; `standard` is a table value that is not measured and must not be used as confirmed fit data.
- The spool rule stays: a spool count does not establish remaining mass.

### Screens

- New button **Add from photo** in the inventory header. It opens a review list, one card per draft: fields, a specs table with a source badge and the printed evidence, **Discard**, **I bought one pack**, and a running **Save N items**.
- The single-item editor keeps its **Choose label photo** button and uses the same reader. If the photo contains several items, it offers to open them in the review list. Specs are edited as rows in the editor.

## Acceptance set

The 29 photos in `~/Corydoras/imports/workshop-catalogue-2026-11-14/photos`, checked against the catalogue records that reference each one (35 threaded inserts, 14 filament spools, 17 bolts, the 1,760-piece kit and its chart rows):

- The number of drafts per photo matches the number of catalogue records that reference it, or the difference is explained.
- No spec's evidence comes from another row.
- No quantity on hand is set.
- Filament drafts carry material, diameter, net weight and the printed colour, and a translucent spool is not recorded as clear.
- Threaded-insert drafts show hole and outer diameter only where printed.
- The catalogue's own insert roles are `convention` values, so they are a comparison, not ground truth.
- Provider tests use the existing mock pattern in `inventory_label/tests.rs`, plus cases for cross-row evidence, `shared_text` misuse, a cut-off list and 41 items.
- Running the 29 photos uses the live provider and the designer's monthly token budget. It runs only with the designer's say-so, and the designer decides which photos to include.

## Left to the builder

- Exact prompt wording and the JSON schema field names.
- Final limit numbers, after running the acceptance set.
- The standards table values, using ISO 261 pitches and ISO 273 medium clearance holes.
- Layout and focus handling of the review list, following the existing workshop editor.
- Whether the one reader fully replaces `read_label` internally (the design requires one reader, not two prompts).

## Calls made without the designer (review these)

1. **A threaded insert's hole size stays blank unless printed or entered.** The designer picked option 2, which was worded as "label facts plus standard values, marked as such". The table gives pitch for threads and clearance and tap drill for screws and bolts. For inserts it gives nothing, because the hole depends on the product and a wrong table value would be worse than a blank.
2. `quantity` counts pieces for hardware and spools for filament (see Stock rule).
3. A fourth source, `notation`, marks numbers printed in a size marking whose role is a convention. The designer asked for label, standard and entered only.
4. The source for typed values is called `entered`, not `measured`: it records who typed it, not a verification.
5. The editor shows the Dimensions text read-only whenever specs exist.
6. Standard values come from a code table, not the model, and cover pitch, plus clearance and tap drill for screws and bolts only.
7. All numbers in "Limits" are starting values.
8. No commit or branch is made. `docs/` and `CLAUDE.md` are untracked on `main`.
