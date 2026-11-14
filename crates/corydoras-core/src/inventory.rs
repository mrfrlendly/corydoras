//! Workshop inventory in the current library. Each record describes one stock item.

use crate::Library;
use anyhow::{bail, ensure, Result};
use reqwest::Url;
use rusqlite::{params, Row};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

const SHORT_FIELD_LIMIT: usize = 512;
// The desktop form sends JSON numbers through JavaScript.
const MAX_QUANTITY: i64 = (1 << 53) - 1;

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct SupplierListing {
    pub title: String,
    pub url: String,
    pub details: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct InventoryDraft {
    pub name: String,
    pub category: String,
    pub brand: String,
    pub part_number: String,
    pub dimensions: String,
    /// Current stock. None means that the user did not enter a count.
    pub quantity: Option<i64>,
    /// The number of units per package, as printed on the label.
    pub pack_quantity: Option<i64>,
    pub notes: String,
    pub supplier: Option<SupplierListing>,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
pub struct InventoryItem {
    pub id: String,
    #[serde(flatten)]
    pub draft: InventoryDraft,
}

fn field(value: &str, name: &str, limit: usize) -> Result<String> {
    let value = value.trim();
    ensure!(
        value.chars().count() <= limit,
        "{name} exceeds {limit} characters."
    );
    ensure!(
        !value
            .chars()
            .any(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t')),
        "{name} contains an invalid character."
    );
    Ok(value.to_owned())
}

impl SupplierListing {
    /// Checks the URL syntax before storage or display. This method does not resolve DNS.
    pub fn validate_url(value: &str) -> Result<Url> {
        let error = "Enter a public HTTPS URL for the supplier listing.";
        ensure!(
            value.len() <= 4096
                && !value
                    .chars()
                    .any(|c| c.is_whitespace() || c.is_control() || c == '\\'),
            "{error}"
        );
        let url = Url::parse(value).map_err(|_| anyhow::anyhow!(error))?;
        let domain = url.domain().unwrap_or("").trim_end_matches('.');
        let suffix = domain.rsplit('.').next().unwrap_or("");
        let dns_label = |s: &str| {
            !s.is_empty()
                && s.len() <= 63
                && !s.starts_with('-')
                && !s.ends_with('-')
                && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        };
        ensure!(
            url.scheme() == "https"
                && url.as_str().len() <= 4096
                && url.username().is_empty()
                && url.password().is_none()
                && domain.contains('.')
                && domain.len() <= 253
                && domain.split('.').all(dns_label)
                && !matches!(
                    suffix,
                    "localhost"
                        | "local"
                        | "internal"
                        | "lan"
                        | "invalid"
                        | "test"
                        | "home"
                        | "example"
                        | "arpa"
                        | "onion"
                ),
            "{error}"
        );
        Ok(url)
    }

    fn normalized(&self) -> Result<Self> {
        let title = field(&self.title, "The supplier title", SHORT_FIELD_LIMIT)?;
        ensure!(!title.is_empty(), "Enter a title for the supplier listing.");
        Ok(Self {
            title,
            url: Self::validate_url(self.url.trim())?.to_string(),
            details: field(&self.details, "The supplier details", 8192)?,
        })
    }
}

impl InventoryDraft {
    pub fn normalized(&self) -> Result<Self> {
        let draft = self.normalized_for_review()?;
        ensure!(!draft.name.is_empty(), "Enter an item name.");
        Ok(draft)
    }

    /// A label can supply useful fields even when the user must enter the name.
    pub(crate) fn normalized_for_review(&self) -> Result<Self> {
        let name = field(&self.name, "The item name", SHORT_FIELD_LIMIT)?;
        let category = field(&self.category, "The category", 120)?;
        for count in [self.quantity, self.pack_quantity].into_iter().flatten() {
            ensure!(
                (0..=MAX_QUANTITY).contains(&count),
                "Enter a whole count from 0 to {MAX_QUANTITY}."
            );
        }
        Ok(Self {
            name,
            category: if category.is_empty() {
                "Other".to_owned()
            } else {
                category
            },
            brand: field(&self.brand, "The brand", SHORT_FIELD_LIMIT)?,
            part_number: field(&self.part_number, "The part number", SHORT_FIELD_LIMIT)?,
            dimensions: field(&self.dimensions, "The dimensions", 2048)?,
            quantity: self.quantity,
            pack_quantity: self.pack_quantity,
            notes: field(&self.notes, "The notes", 8192)?,
            supplier: self
                .supplier
                .as_ref()
                .map(SupplierListing::normalized)
                .transpose()?,
        })
    }
}

fn inventory_row(row: &Row<'_>) -> rusqlite::Result<InventoryItem> {
    let supplier = match row.get::<_, Option<String>>(9)? {
        Some(url) => Some(SupplierListing {
            url,
            title: row.get(10)?,
            details: row.get(11)?,
        }),
        None => None,
    };
    Ok(InventoryItem {
        id: row.get(0)?,
        draft: InventoryDraft {
            name: row.get(1)?,
            category: row.get(2)?,
            brand: row.get(3)?,
            part_number: row.get(4)?,
            dimensions: row.get(5)?,
            quantity: row.get(6)?,
            pack_quantity: row.get(7)?,
            notes: row.get(8)?,
            supplier,
        },
    })
}

impl Library {
    pub fn inventory_enabled(&self) -> Result<bool> {
        Ok(self.c().query_row(
            "SELECT EXISTS (SELECT 1 FROM settings WHERE key = 'workshop_inventory_enabled' AND value = '1')",
            [],
            |row| row.get(0),
        )?)
    }

    pub fn set_inventory_enabled(&self, enabled: bool) -> Result<()> {
        self.c().execute(
            "INSERT INTO settings (key, value) VALUES ('workshop_inventory_enabled', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [if enabled { "1" } else { "0" }],
        )?;
        Ok(())
    }

    pub fn list_inventory(&self) -> Result<Vec<InventoryItem>> {
        let conn = self.c();
        let mut stmt = conn.prepare(
            "SELECT id, name, category, brand, part_number, dimensions, quantity, pack_quantity,
                    notes, supplier_url, supplier_title, supplier_details
             FROM workshop_inventory ORDER BY name COLLATE NOCASE, id",
        )?;
        let items = stmt
            .query_map([], inventory_row)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(items)
    }

    pub fn save_inventory(
        &self,
        id: Option<&str>,
        draft: &InventoryDraft,
    ) -> Result<InventoryItem> {
        let draft = draft.normalized()?;
        let item_id = id
            .map(str::to_owned)
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        let listing = draft.supplier.as_ref();
        let sql = if id.is_some() {
            "UPDATE workshop_inventory SET name=?2, category=?3, brand=?4, part_number=?5,
                dimensions=?6, quantity=?7, pack_quantity=?8, notes=?9, supplier_url=?10,
                supplier_title=?11, supplier_details=?12 WHERE id=?1"
        } else {
            "INSERT INTO workshop_inventory (id,name,category,brand,part_number,dimensions,
                quantity,pack_quantity,notes,supplier_url,supplier_title,supplier_details)
             VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)"
        };
        let count = self.c().execute(
            sql,
            params![
                item_id,
                draft.name,
                draft.category,
                draft.brand,
                draft.part_number,
                draft.dimensions,
                draft.quantity,
                draft.pack_quantity,
                draft.notes,
                listing.map(|s| s.url.as_str()),
                listing.map_or("", |s| &s.title),
                listing.map_or("", |s| &s.details),
            ],
        )?;
        if count == 0 {
            bail!("This inventory item no longer exists.");
        }
        Ok(InventoryItem { id: item_id, draft })
    }

    pub fn delete_inventory(&self, id: &str) -> Result<()> {
        let count = self
            .c()
            .execute("DELETE FROM workshop_inventory WHERE id = ?1", [id])?;
        ensure!(count == 1, "This inventory item no longer exists.");
        Ok(())
    }
}
