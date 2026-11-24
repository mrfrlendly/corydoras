//! Categories and column choices of the workshop inventory list.
//! The category text on an item stays its identity. A row in `workshop_categories` only holds the
//! column choice of a category and keeps a category that has no items yet. Columns are a view
//! setting: they never hide data from search or from agents.

use crate::inventory::{inventory_row, InventoryItem, MAX_QUANTITY, SELECT_ITEMS};
use crate::Library;
use anyhow::{anyhow, ensure, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The columns a list view can show. The item name is always shown and is not one of them.
/// The same ids live in `src/components/workshopColumns.ts`.
pub const COLUMN_IDS: &[&str] = &[
    "category",
    "brand",
    "part_number",
    "colour",
    "dimensions",
    "quantity",
    "pack_quantity",
    "open_amount",
    "price",
    "unit_price",
    "stock_value",
    "notes",
    "supplier",
];

const MAX_CATEGORY_CHARS: usize = 120;
/// The most keywords of one category, and the length of each.
pub const MAX_KEYWORDS: usize = 20;
pub const MAX_KEYWORD_CHARS: usize = 40;
const DEFAULT_CURRENCY: &str = "CAD";
const CURRENCY_KEY: &str = "workshop_currency";
const ALL_COLUMNS_KEY: &str = "workshop_all_columns";

/// One category of the list. `columns` is None while the category uses Auto.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct CategoryInfo {
    pub name: String,
    pub count: i64,
    pub columns: Option<Vec<String>>,
    /// Extra words that put an item into this category (see `inventory_intake`).
    #[serde(default)]
    pub keywords: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct ViewSettings {
    pub currency: String,
    /// The column choice of the All categories view. None means Auto.
    pub all_columns: Option<Vec<String>>,
}

fn category_name(name: &str) -> Result<String> {
    let name = name.trim();
    ensure!(
        (1..=MAX_CATEGORY_CHARS).contains(&name.chars().count())
            && !name.chars().any(char::is_control),
        "Enter a category name on one line with at most {MAX_CATEGORY_CHARS} characters."
    );
    Ok(name.to_owned())
}

fn checked_columns(columns: Option<Vec<String>>) -> Result<Option<Vec<String>>> {
    let Some(columns) = columns else {
        return Ok(None);
    };
    for (i, id) in columns.iter().enumerate() {
        ensure!(
            id != "name",
            "The item column is always shown. Remove it from the columns."
        );
        ensure!(
            COLUMN_IDS.contains(&id.as_str()),
            "{id} is not a column of the inventory list."
        );
        ensure!(
            !columns[..i].contains(id),
            "Each column can be chosen once. {id} appears twice."
        );
    }
    Ok(Some(columns))
}

fn columns_json(columns: &Option<Vec<String>>) -> Result<Option<String>> {
    Ok(columns.as_ref().map(serde_json::to_string).transpose()?)
}

/// A column list that cannot be read counts as Auto, so a damaged value never blocks the list.
fn read_columns(json: Option<String>) -> Option<Vec<String>> {
    json.and_then(|text| serde_json::from_str(&text).ok())
}

/// A keyword list that cannot be read counts as none, so a damaged value never blocks the list.
fn read_keywords(json: Option<String>) -> Vec<String> {
    json.and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn checked_keywords(keywords: Vec<String>) -> Result<Vec<String>> {
    ensure!(
        keywords.len() <= MAX_KEYWORDS,
        "Use at most {MAX_KEYWORDS} keywords for a category."
    );
    let mut checked: Vec<String> = Vec::with_capacity(keywords.len());
    for keyword in keywords {
        let keyword = keyword.trim().to_owned();
        ensure!(
            (1..=MAX_KEYWORD_CHARS).contains(&keyword.chars().count())
                && !keyword.chars().any(char::is_control),
            "Enter each keyword on one line with 1 to {MAX_KEYWORD_CHARS} characters."
        );
        ensure!(
            !checked.iter().any(|k| k.to_lowercase() == keyword.to_lowercase()),
            "Each keyword can be used once. {keyword} appears twice."
        );
        checked.push(keyword);
    }
    Ok(checked)
}

/// SQLite's NOCASE folds ASCII letters only, so the identity of a category does the same.
fn fold(name: &str) -> String {
    name.to_ascii_lowercase()
}

/// How the category is spelled now: the stored row first, then the most common spelling on items.
fn stored_spelling(conn: &Connection, name: &str) -> Result<Option<String>> {
    let row = conn
        .query_row(
            "SELECT name FROM workshop_categories WHERE name = ?1",
            [name],
            |row| row.get(0),
        )
        .optional()?;
    if row.is_some() {
        return Ok(row);
    }
    Ok(conn
        .query_row(
            "SELECT category FROM workshop_inventory WHERE category = ?1 COLLATE NOCASE
             GROUP BY category ORDER BY count(*) DESC, category LIMIT 1",
            [name],
            |row| row.get(0),
        )
        .optional()?)
}

fn item_count(conn: &Connection, name: &str) -> Result<i64> {
    Ok(conn.query_row(
        "SELECT count(*) FROM workshop_inventory WHERE category = ?1 COLLATE NOCASE",
        [name],
        |row| row.get(0),
    )?)
}

fn setting(conn: &Connection, key: &str) -> Result<Option<String>> {
    Ok(conn
        .query_row("SELECT value FROM settings WHERE key = ?1", [key], |row| {
            row.get(0)
        })
        .optional()?)
}

/// The categories that items use, with their counts, and the categories that were created
/// without items. Sorted by name, ignoring case.
pub(crate) fn categories_in(conn: &Connection) -> Result<Vec<CategoryInfo>> {
    let mut by_name: HashMap<String, CategoryInfo> = HashMap::new();
    // The most common spelling on items names a category that has no row.
    let mut spelling_count: HashMap<String, i64> = HashMap::new();
    let mut stmt = conn.prepare(
        "SELECT category, count(*) FROM workshop_inventory GROUP BY category ORDER BY category",
    )?;
    let used = stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (name, count) in used {
        let entry = by_name.entry(fold(&name)).or_insert_with(|| CategoryInfo {
            name: name.clone(),
            count: 0,
            columns: None,
            keywords: Vec::new(),
        });
        let best = spelling_count.entry(fold(&name)).or_insert(0);
        if count > *best {
            *best = count;
            entry.name = name;
        }
        entry.count += count;
    }
    let mut stmt = conn.prepare("SELECT name, columns, keywords FROM workshop_categories")?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, Option<String>>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (name, columns, keywords) in rows {
        let entry = by_name.entry(fold(&name)).or_insert_with(|| CategoryInfo {
            name: name.clone(),
            count: 0,
            columns: None,
            keywords: Vec::new(),
        });
        entry.name = name;
        entry.columns = read_columns(columns);
        entry.keywords = read_keywords(keywords);
    }
    let mut categories: Vec<CategoryInfo> = by_name.into_values().collect();
    categories.sort_by(|a, b| {
        fold(&a.name)
            .cmp(&fold(&b.name))
            .then_with(|| a.name.cmp(&b.name))
    });
    Ok(categories)
}

impl Library {
    /// The categories that items use, with their counts, and the categories that were created
    /// without items. Sorted by name, ignoring case.
    pub fn list_categories(&self) -> Result<Vec<CategoryInfo>> {
        categories_in(&self.c())
    }

    pub fn create_category(&self, name: &str) -> Result<()> {
        let name = category_name(name)?;
        let mut conn = self.c();
        let tx = conn.transaction()?;
        ensure!(
            stored_spelling(&tx, &name)?.is_none(),
            "A category named {name} already exists."
        );
        tx.execute(
            "INSERT INTO workshop_categories (name, columns) VALUES (?1, NULL)",
            [&name],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Gives every item of `from` the name `to` and returns how many items changed. When `to`
    /// already exists the categories merge, and the columns and keywords of `to` stay (it takes the keywords of `from` only when it has none). Renaming to the same
    /// name in other letters only changes the letters.
    pub fn rename_category(&self, from: &str, to: &str) -> Result<i64> {
        let from = from.trim();
        let mut to = category_name(to)?;
        let mut conn = self.c();
        let tx = conn.transaction()?;
        let merging = fold(from) != fold(&to);
        if merging {
            // A merge keeps the spelling of the category that already exists.
            to = stored_spelling(&tx, &to)?.unwrap_or(to);
        }
        let source: Option<(Option<String>, Option<String>)> = tx
            .query_row(
                "SELECT columns, keywords FROM workshop_categories WHERE name = ?1",
                [from],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        ensure!(
            source.is_some() || item_count(&tx, from)? > 0,
            "This category no longer exists."
        );
        let changed = tx.execute(
            "UPDATE workshop_inventory SET category = ?2
             WHERE category = ?1 COLLATE NOCASE AND category <> ?2",
            params![from, to],
        )?;
        if !merging {
            tx.execute(
                "UPDATE workshop_categories SET name = ?2 WHERE name = ?1",
                params![from, to],
            )?;
        } else {
            // Items that already had the target name in other letters take its spelling too.
            tx.execute(
                "UPDATE workshop_inventory SET category = ?1
                 WHERE category = ?1 COLLATE NOCASE AND category <> ?1",
                [&to],
            )?;
            if let Some((columns, keywords)) = source {
                // The row of an existing target keeps its columns. Otherwise the target takes the
                // columns of the source. The keywords of the target win too, unless it has none:
                // then it takes the keywords of the source. The source row goes either way.
                tx.execute(
                    "INSERT INTO workshop_categories (name, columns, keywords) VALUES (?1, ?2, ?3)
                     ON CONFLICT(name) DO UPDATE SET keywords = CASE
                         WHEN workshop_categories.keywords IS NULL OR workshop_categories.keywords = '[]'
                         THEN excluded.keywords ELSE workshop_categories.keywords END",
                    params![to, columns, keywords],
                )?;
                tx.execute("DELETE FROM workshop_categories WHERE name = ?1", [from])?;
            }
        }
        tx.commit()?;
        Ok(changed as i64)
    }

    pub fn delete_category(&self, name: &str) -> Result<()> {
        let name = name.trim();
        let mut conn = self.c();
        let tx = conn.transaction()?;
        let used = item_count(&tx, name)?;
        ensure!(
            used == 0,
            "{name} still has {used} {}. Move or delete them first.",
            if used == 1 { "item" } else { "items" }
        );
        let removed = tx.execute("DELETE FROM workshop_categories WHERE name = ?1", [name])?;
        ensure!(removed == 1, "This category no longer exists.");
        tx.commit()?;
        Ok(())
    }

    /// Sets the columns of one category view. None means Auto. The category row is made when absent.
    pub fn set_category_columns(&self, name: &str, columns: Option<Vec<String>>) -> Result<()> {
        let name = category_name(name)?;
        let columns = columns_json(&checked_columns(columns)?)?;
        let mut conn = self.c();
        let tx = conn.transaction()?;
        let name = stored_spelling(&tx, &name)?.unwrap_or(name);
        tx.execute(
            "INSERT INTO workshop_categories (name, columns) VALUES (?1, ?2)
             ON CONFLICT(name) DO UPDATE SET columns = excluded.columns",
            params![name, columns],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// Sets the extra words of a category. An empty list means none. The category row is made when absent.
    pub fn set_category_keywords(&self, name: &str, keywords: Vec<String>) -> Result<()> {
        let name = category_name(name)?;
        let keywords = checked_keywords(keywords)?;
        let json = if keywords.is_empty() {
            None
        } else {
            Some(serde_json::to_string(&keywords)?)
        };
        let mut conn = self.c();
        let tx = conn.transaction()?;
        let name = stored_spelling(&tx, &name)?.unwrap_or(name);
        tx.execute(
            "INSERT INTO workshop_categories (name, columns, keywords) VALUES (?1, NULL, ?2)
             ON CONFLICT(name) DO UPDATE SET keywords = excluded.keywords",
            params![name, json],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn view_settings(&self) -> Result<ViewSettings> {
        let conn = self.c();
        let currency = setting(&conn, CURRENCY_KEY)?
            .filter(|code| valid_currency(code))
            .unwrap_or_else(|| DEFAULT_CURRENCY.to_owned());
        Ok(ViewSettings {
            currency,
            all_columns: read_columns(setting(&conn, ALL_COLUMNS_KEY)?),
        })
    }

    /// Sets the columns of the All categories view. None means Auto.
    pub fn set_all_columns(&self, columns: Option<Vec<String>>) -> Result<()> {
        let json = columns_json(&checked_columns(columns)?)?;
        let conn = self.c();
        match json {
            Some(json) => conn.execute(
                "INSERT INTO settings (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![ALL_COLUMNS_KEY, json],
            )?,
            None => conn.execute("DELETE FROM settings WHERE key = ?1", [ALL_COLUMNS_KEY])?,
        };
        Ok(())
    }

    /// The currency of every price in the library, as a three-letter code in capitals.
    pub fn set_currency(&self, code: &str) -> Result<()> {
        ensure!(
            valid_currency(code),
            "Enter a currency code of three capital letters, such as CAD."
        );
        self.c().execute(
            "INSERT INTO settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![CURRENCY_KEY, code],
        )?;
        Ok(())
    }

    /// Changes only the count on hand, for the quick steppers of the list. A count of zero leaves
    /// no opened pack, so it also clears the amount left. So does a lower count than the stored one:
    /// the opened pack is assumed to be the one used up. An unknown stored count is not a decrease.
    pub fn set_inventory_quantity(&self, id: &str, quantity: i64) -> Result<InventoryItem> {
        ensure!(
            (0..=MAX_QUANTITY).contains(&quantity),
            "Enter a whole count from 0 to {MAX_QUANTITY}."
        );
        let mut conn = self.c();
        let tx = conn.transaction()?;
        let changed = tx.execute(
            "UPDATE workshop_inventory SET quantity = ?2,
                 measure_left = CASE WHEN ?2 = 0 OR ?2 < quantity THEN NULL ELSE measure_left END WHERE id = ?1",
            params![id, quantity],
        )?;
        ensure!(changed == 1, "This inventory item no longer exists.");
        let item = tx
            .query_row(
                &format!("{SELECT_ITEMS} WHERE id = ?1"),
                [id],
                inventory_row,
            )
            .optional()?
            .ok_or_else(|| anyhow!("This inventory item no longer exists."))?;
        tx.commit()?;
        Ok(item)
    }

    /// Records how much is left in the one opened pack, or clears it with None. The item needs a
    /// unit and a full pack amount first, and at least one pack on hand when the count is known.
    pub fn set_open_amount(&self, id: &str, left: Option<i64>) -> Result<InventoryItem> {
        let mut conn = self.c();
        let tx = conn.transaction()?;
        let item = tx
            .query_row(
                &format!("{SELECT_ITEMS} WHERE id = ?1"),
                [id],
                inventory_row,
            )
            .optional()?
            .ok_or_else(|| anyhow!("This inventory item no longer exists."))?;
        if let Some(left) = left {
            let draft = &item.draft;
            let full = draft
                .measure_full
                .filter(|_| !draft.measure_unit.is_empty());
            let Some(full) = full else {
                return Err(anyhow!(
                    "Choose a unit and enter the full pack amount of this item before you record the amount left."
                ));
            };
            ensure!(
                draft.quantity != Some(0),
                "This item has none on hand, so no pack is open. Raise the count first."
            );
            ensure!(
                (0..=full).contains(&left),
                "Enter an amount left from 0 to {full} {}.",
                draft.measure_unit
            );
        }
        tx.execute(
            "UPDATE workshop_inventory SET measure_left = ?2 WHERE id = ?1",
            params![id, left],
        )?;
        let item = tx.query_row(
            &format!("{SELECT_ITEMS} WHERE id = ?1"),
            [id],
            inventory_row,
        )?;
        tx.commit()?;
        Ok(item)
    }
}

fn valid_currency(code: &str) -> bool {
    code.len() == 3 && code.bytes().all(|b| b.is_ascii_uppercase())
}
