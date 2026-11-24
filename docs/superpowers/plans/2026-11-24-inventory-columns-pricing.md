# Inventory columns, categories and price: plan

Spec: `docs/superpowers/specs/2026-11-24-inventory-columns-pricing-design.md` (the contract lives there and in `src/inventoryApi.ts`). Branch `inventory-columns-pricing`. Agents do not commit; the lead commits after review.

## Rules for every task

- Never open, write or test against `~/Corydoras/library.sqlite`. Tests use temp libraries; live checks use a copy.
- Only edit the files your task owns. Report anything outside them that needs a change instead of making it.
- Run before reporting: `npm run test:workshop`, `npx tsc -b` (TypeScript tasks) and `cargo test -p corydoras-core -p corydoras-mcp` (Rust tasks; `cargo test -p corydoras` for `src-tauri` if the package name differs, check `Cargo.toml`). Report the exact results.
- Follow the repo's tone for user-facing text: plain, sentence case, no exclamation marks.

## Waves

| Wave | Task | Model | Owns |
| --- | --- | --- | --- |
| 1 | T1 Rust data model: price, categories, view settings, quantity, migration, tests | sonnet | `crates/corydoras-core/src/{db,inventory,inventory_categories,lib,inventory_search,agent_inventory,inventory_label}.rs` (compile fixes only in the last three), `crates/corydoras-core/tests/workshop_inventory.rs` |
| 1 | T3 TypeScript column engine and form plumbing | sonnet | `src/components/workshopColumns.ts`, `workshopList.ts`, `workshopForm.ts`, `src/phone/*` typing only, `tests/workshop-*.test.mjs` |
| 2 | T2 Tauri commands, MCP price, ipc tests | sonnet | `src-tauri/src/{inventory,main}.rs`, `src-tauri/src/ipc_tests*`, `crates/corydoras-mcp/**` |
| 2 | T4 List UI: columns, picker, chips, footer | sonnet | `WorkshopInventoryTable.tsx`, `WorkshopInventory.tsx`, new `WorkshopColumnPicker.tsx`, `workshop.css` |
| 3 | T5 Category manager, editor price field, stepper, duplicate, shortcuts | sonnet | new `WorkshopCategories.tsx`, `WorkshopItemEditor.tsx`, `WorkshopInventory.tsx`, `workshop.css` |
| 3 | T6 Docs and phone Price field | haiku | `README.md`, `src/phone/PhoneReview.tsx` |
| 4 | Review, then live check on a library copy | sonnet, then lead | none (report only) |

Wave 3 waits for wave 2 so that T5 does not share `WorkshopInventory.tsx` or `workshop.css` with T4.
