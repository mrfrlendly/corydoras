//! Read-only access to the workshop stock that the current library records.
//! The `item` unit counts whole instances of the exact inventory item. For a spool, one item means one spool.
use crate::inventory::{inventory_enabled, inventory_row, InventoryItem};
use crate::Library;
use anyhow::{ensure, Result};
use rusqlite::{Connection, OptionalExtension};
use serde_json::{json, Value};
use std::cmp::Reverse;

mod matching;

pub const MAX_QUERY_CHARS: usize = 256;
pub const MAX_LIMIT: usize = 50;
pub const MAX_EXACT_INTEGER: i64 = (1 << 53) - 1;

// Earlier inventory schemas lack the colour and photo columns. A design check
// must still read those libraries without a migration or a write transaction.
fn select_items(conn: &Connection) -> Result<String> {
    let (has_colour, has_photo): (bool, bool) = conn.query_row(
        "SELECT count(*) FILTER (WHERE name='colour'), count(*) FILTER (WHERE name='photo_key')
         FROM pragma_table_info('workshop_inventory')",
        [],
        |row| Ok((row.get(0)?, row.get(1)?)),
    )?;
    let colour = if has_colour { "colour" } else { "''" };
    let photo = if has_photo { "photo_key" } else { "NULL" };
    Ok(format!(
        "SELECT id, name, category, brand, part_number, dimensions, quantity, pack_quantity,
                notes, supplier_url, supplier_title, supplier_details, {colour}, {photo}
                FROM workshop_inventory"
    ))
}

fn summary(record: &InventoryItem) -> Value {
    let draft = &record.draft;
    let stock_state = match draft.quantity {
        None => "unknown",
        Some(0) => "out_of_stock",
        Some(_) => "in_stock",
    };
    json!({
        "id": record.id, "name": draft.name, "category": draft.category,
        "brand": draft.brand, "part_number": draft.part_number, "colour": draft.colour,
        "dimensions": draft.dimensions, "quantity": draft.quantity, "quantity_unit": "item",
        "pack_quantity": draft.pack_quantity,
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
    let items = stmt
        .query_map([limit, offset], inventory_row)?
        .map(|row| row.map(|record| summary(&record)))
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
    let mut matches = Vec::new();
    for result in stmt.query_map([], inventory_row)? {
        let record = result?;
        if let Some(score) = matching::score(terms, &record.draft) {
            let key = (
                Reverse(score),
                record.draft.name.to_lowercase(),
                record.id.clone(),
            );
            matches.push((key, summary(&record)));
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
    ensure!(
        matches!(id.len(), 32 | 36) && uuid::Uuid::parse_str(id).is_ok(),
        "Enter the exact inventory item ID."
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
            let status = match record.draft.quantity {
                None => "unknown",
                Some(stock) if stock < required => "insufficient",
                Some(_) => "sufficient",
            };
            json!({"required_quantity":required, "status":status})
        });
        let mut detail = summary(&record);
        detail["notes"] = json!(record.draft.notes);
        detail["supplier"] = json!(record.draft.supplier);
        detail["photo_key"] = json!(record.photo_key);
        Ok(json!({"status":"ok","item":detail,"quantity_check":check}))
    })
}

#[cfg(test)]
mod tests;
