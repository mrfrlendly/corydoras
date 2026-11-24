//! Read-only access to the workshop stock that the current library records.
//! The `item` unit counts whole instances of the exact inventory item. For a spool, one item means one spool.
//! For hardware sold in packs, one item is one piece; `pack_quantity` says how many pieces a pack held.
use crate::inventory::{inventory_enabled, inventory_row, InventoryDraft, InventoryItem, MEASURE_UNITS};
use crate::Library;
use anyhow::{ensure, Result};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::cmp::Reverse;

mod matching;

pub const MAX_QUERY_CHARS: usize = 256;
pub const MAX_LIMIT: usize = 50;
pub const MAX_EXACT_INTEGER: i64 = (1 << 53) - 1;

// Earlier inventory schemas lack the colour, photo, specs, price and measured-pack columns. A
// design check must still read those libraries without a migration or a write transaction.
fn select_items(conn: &Connection) -> Result<String> {
    let have = conn
        .prepare("SELECT name FROM pragma_table_info('workshop_inventory')")?
        .query_map([], |row| row.get::<_, String>(0))?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let column = |name: &str, missing: &'static str| -> String {
        if have.iter().any(|have| have == name) {
            name.to_owned()
        } else {
            missing.to_owned()
        }
    };
    Ok(format!(
        "SELECT id, name, category, brand, part_number, dimensions, quantity, pack_quantity,
                notes, supplier_url, supplier_title, supplier_details, {}, {}, {},
                {}, {}, {}, {}
                FROM workshop_inventory",
        column("colour", "''"),
        column("photo_key", "NULL"),
        column("specs", "'[]'"),
        column("price_cents", "NULL"),
        column("measure_unit", "''"),
        column("measure_full", "NULL"),
        column("measure_left", "NULL"),
    ))
}

const DEFAULT_CURRENCY: &str = "CAD";

// The library currency, read in the same transaction. A missing or malformed setting means CAD.
fn currency(conn: &Connection) -> Result<String> {
    let code: Option<String> = conn
        .query_row(
            "SELECT value FROM settings WHERE key = 'workshop_currency'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    Ok(code
        .filter(|code| code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase()))
        .unwrap_or_else(|| DEFAULT_CURRENCY.to_owned()))
}

/// The price of one piece as a decimal string of the library currency with up to four places:
/// the pack price divided by the pack count when the pack holds at least one piece, else the price.
/// A measured pack (see `price_divisor`) divides by its full amount, so the price is per gram,
/// per millilitre or per metre.
/// Trailing zeros go, but two places stay ("5.00", "0.222").
fn unit_price(price_cents: Option<i64>, pack_quantity: Option<i64>) -> Option<String> {
    let cents = i128::from(price_cents?);
    let pack = i128::from(pack_quantity.filter(|pack| *pack >= 1).unwrap_or(1));
    let tenth_mills = (cents * 100 + pack / 2) / pack;
    let text = format!("{}.{:04}", tenth_mills / 10_000, tenth_mills % 10_000);
    let kept = text.trim_end_matches('0').len().max(text.len() - 2);
    Some(text[..kept].to_owned())
}

/// What the pack price divides by: the full amount of a measured pack, else the pack count.
fn price_divisor(draft: &InventoryDraft) -> Option<i64> {
    match draft.measure_full {
        Some(full) if !draft.measure_unit.is_empty() => Some(full),
        _ => draft.pack_quantity,
    }
}

fn summary(record: &InventoryItem, currency: &str) -> Value {
    let draft = &record.draft;
    let stock_state = match draft.quantity {
        None => "unknown",
        Some(0) => "out_of_stock",
        Some(_) => "in_stock",
    };
    json!({
        "id": record.id, "name": draft.name, "category": draft.category,
        "brand": draft.brand, "part_number": draft.part_number, "colour": draft.colour,
        "dimensions": draft.dimensions, "specs": draft.specs,
        "quantity": draft.quantity, "quantity_unit": "item",
        "pack_quantity": draft.pack_quantity,
        "price_cents": draft.price_cents, "currency": currency,
        "unit_price": unit_price(draft.price_cents, price_divisor(draft)),
        "measure_unit": draft.measure_unit, "measure_full": draft.measure_full,
        "measure_left": draft.measure_left, "total_amount": draft.total_amount(),
        "stock_state": stock_state, "has_photo": record.photo_key.is_some()
    })
}

fn with_inventory(lib: &Library, read: impl FnOnce(&Connection) -> Result<Value>) -> Result<Value> {
    let mut conn = lib.c();
    let tx = conn.transaction()?;
    let result = if inventory_enabled(&tx)? {
        read(&tx)?
    } else {
        json!({"status":"disabled"})
    };
    tx.commit()?;
    Ok(result)
}

fn browse(
    conn: &Connection,
    sql: &str,
    limit: usize,
    offset: usize,
) -> Result<(usize, Vec<Value>)> {
    let total = conn.query_row("SELECT count(*) FROM workshop_inventory", [], |row| {
        row.get(0)
    })?;
    let mut stmt = conn.prepare(&format!(
        "{sql} ORDER BY name COLLATE NOCASE, id LIMIT ?1 OFFSET ?2"
    ))?;
    let currency = currency(conn)?;
    let items = stmt
        .query_map([limit, offset], inventory_row)?
        .map(|row| row.map(|record| summary(&record, &currency)))
        .collect::<rusqlite::Result<_>>()?;
    Ok((total, items))
}

fn find(
    conn: &Connection,
    sql: &str,
    terms: &[String],
    limit: usize,
    offset: usize,
) -> Result<(usize, Vec<Value>)> {
    let mut stmt = conn.prepare(sql)?;
    let currency = currency(conn)?;
    let mut matches = Vec::new();
    for result in stmt.query_map([], inventory_row)? {
        let record = result?;
        if let Some(score) = matching::score(terms, &record.draft) {
            let key = (
                Reverse(score),
                record.draft.name.to_lowercase(),
                record.id.clone(),
            );
            matches.push((key, summary(&record, &currency)));
        }
    }
    matches.sort_unstable_by(|a, b| a.0.cmp(&b.0));
    let total = matches.len();
    let page = matches
        .into_iter()
        .skip(offset)
        .take(limit)
        .map(|(_, item)| item)
        .collect();
    Ok((total, page))
}

/// Search the library that supplies the Inventory tab. A blank query lists all records.
pub fn search(lib: &Library, query: &str, limit: usize, offset: usize) -> Result<Value> {
    ensure!(
        query.chars().count() <= MAX_QUERY_CHARS && !query.chars().any(char::is_control),
        "Enter a search query of at most {MAX_QUERY_CHARS} characters on one line."
    );
    ensure!(
        (1..=MAX_LIMIT).contains(&limit),
        "Enter a limit from 1 to {MAX_LIMIT}."
    );
    ensure!(
        offset as u64 <= MAX_EXACT_INTEGER as u64,
        "The offset exceeds {MAX_EXACT_INTEGER}."
    );
    let terms = matching::words(query);
    ensure!(
        query.trim().is_empty() || !terms.is_empty(),
        "Include a word or a number in the search query."
    );
    with_inventory(lib, |conn| {
        let sql = select_items(conn)?;
        let (total, items) = if terms.is_empty() {
            browse(conn, &sql, limit, offset)?
        } else {
            find(conn, &sql, &terms, limit, offset)?
        };
        let end = offset + items.len();
        Ok(json!({
            "status": if total == 0 { "no_match" } else { "ok" },
            "total": total, "offset": offset, "limit": limit, "next_offset": (end < total).then_some(end),
            "items": items
        }))
    })
}

/// Read one stock record by its stored ID. Only the current quantity counts toward a requirement.
pub fn item(lib: &Library, id: &str, required_quantity: Option<i64>) -> Result<Value> {
    item_in_unit(lib, id, required_quantity, "item")
}

/// The check of an amount in the item's own measure unit against `total_amount`. The amount is what
/// the designer recorded, never a weighing, so a pass says so. An item that records no total, or
/// records it in another unit, is unconfirmed.
fn amount_check(draft: &InventoryDraft, required: i64, unit: &str) -> Value {
    let total = if draft.measure_unit == unit { draft.total_amount() } else { None };
    let (status, note) = match total {
        None if draft.measure_unit != unit => (
            "unknown",
            format!(
                "This item records its amount {}, not in {unit}. Report the stock as unconfirmed.",
                if draft.measure_unit.is_empty() {
                    "as a count".to_owned()
                } else {
                    format!("in {}", draft.measure_unit)
                }
            ),
        ),
        None => (
            "unknown",
            "No total amount is recorded for this item. Report the stock as unconfirmed.".to_owned(),
        ),
        Some(total) if total < required => (
            "insufficient",
            "The recorded amount is lower than the requirement. Report the shortage.".to_owned(),
        ),
        Some(_) => (
            "sufficient",
            "Recorded by the designer, not verified by weight.".to_owned(),
        ),
    };
    json!({"required_quantity":required, "required_unit":unit, "status":status,
        "total_amount":total, "note":note})
}

/// Like `item`, with the unit of the requirement: "item" counts whole items, and the item's own
/// measure unit ("g", "mL" or "m") compares the amount with `total_amount`.
pub fn item_in_unit(
    lib: &Library,
    id: &str,
    required_quantity: Option<i64>,
    required_unit: &str,
) -> Result<Value> {
    ensure!(
        matches!(id.len(), 32 | 36) && uuid::Uuid::parse_str(id).is_ok(),
        "Enter the exact inventory item ID."
    );
    ensure!(
        required_unit == "item" || MEASURE_UNITS.contains(&required_unit),
        "Set the required unit to item, g, mL or m."
    );
    if let Some(required) = required_quantity {
        ensure!(
            (1..=MAX_EXACT_INTEGER).contains(&required),
            "Enter a required quantity from 1 to {MAX_EXACT_INTEGER}."
        );
    }
    with_inventory(lib, |conn| {
        let sql = format!("{} WHERE id=?1", select_items(conn)?);
        let Some(record) = conn.query_row(&sql, [id], inventory_row).optional()? else {
            return Ok(json!({"status":"not_found"}));
        };
        let check = required_quantity.map(|required| {
            if required_unit != "item" {
                return amount_check(&record.draft, required, required_unit);
            }
            let status = match record.draft.quantity {
                None => "unknown",
                Some(stock) if stock < required => "insufficient",
                Some(_) => "sufficient",
            };
            json!({"required_quantity":required, "status":status})
        });
        let mut detail = summary(&record, &currency(conn)?);
        detail["notes"] = json!(record.draft.notes);
        detail["supplier"] = json!(record.draft.supplier);
        detail["photo_key"] = json!(record.photo_key);
        Ok(json!({"status":"ok","item":detail,"quantity_check":check}))
    })
}

#[cfg(test)]
mod tests;
