//! Product suggestions and details from public supplier catalogues. Shopify documents
//! both endpoints at https://shopify.dev/docs/api/ajax/reference .

use crate::inventory::SupplierListing;
use anyhow::{bail, ensure, Result};
use reqwest::{redirect, Client, RequestBuilder};
use serde::{de::DeserializeOwned, Deserialize};
use std::time::Duration;

const STORES: [&str; 3] = [
    "https://canadabolts.ca",
    "https://3dprintingcanada.com",
    "https://thepihut.com",
];
const MAX_BYTES: usize = 512 * 1024;
const MAX_PRODUCTS: usize = 2;
const STORE_TIMEOUT: Duration = Duration::from_secs(14);

/// Returns candidates for review. The caller chooses which supplier details to keep.
pub async fn search_listings(
    name: &str,
    brand: &str,
    part_number: &str,
) -> Result<Vec<SupplierListing>> {
    let terms = queries(name, brand, part_number)?;
    let client = Client::builder()
        .https_only(true)
        .redirect(redirect::Policy::none())
        .connect_timeout(Duration::from_secs(3))
        .timeout(Duration::from_secs(6))
        .user_agent("Corydoras inventory")
        .build()?;
    let search = async |store| {
        tokio::time::timeout(
            STORE_TIMEOUT,
            search_store(&client, store, &terms, part_number.trim()),
        )
        .await?
    };
    let (hardware, materials, electronics) =
        tokio::join!(search(STORES[0]), search(STORES[1]), search(STORES[2]));
    combine([hardware, materials, electronics])
}

fn combine<const N: usize>(
    responses: [Result<Vec<SupplierListing>>; N],
) -> Result<Vec<SupplierListing>> {
    let incomplete = responses.iter().any(Result::is_err);
    let results: Vec<_> = responses
        .into_iter()
        .filter_map(Result::ok)
        .flatten()
        .collect();
    if results.is_empty() && incomplete {
        bail!("One or more supplier catalogues did not respond with usable results. Try again or enter a listing manually.");
    }
    Ok(results)
}

fn queries(name: &str, brand: &str, part_number: &str) -> Result<Vec<String>> {
    for field in [name, brand, part_number] {
        ensure!(
            field.chars().count() <= 256 && !field.chars().any(char::is_control),
            "Use a product name, brand or part number of at most 256 characters."
        );
    }
    let (name, brand, part) = (name.trim(), brand.trim(), part_number.trim());
    ensure!(
        !name.is_empty() || !part.is_empty(),
        "Enter a product name or part number to search suppliers."
    );
    let name_query = if brand.is_empty() || name.to_lowercase().contains(&brand.to_lowercase()) {
        name.to_owned()
    } else {
        format!("{brand} {name}")
    };
    let mut terms = Vec::new();
    if !part.is_empty() {
        terms.push(part.to_owned());
    }
    if !name.is_empty() && !name_query.eq_ignore_ascii_case(part) {
        terms.push(name_query);
    }
    Ok(terms)
}

#[derive(Deserialize)]
struct Suggestions {
    resources: Resources,
}
#[derive(Deserialize)]
struct Resources {
    results: ProductResults,
}
#[derive(Deserialize)]
struct ProductResults {
    products: Vec<Hit>,
}
#[derive(Deserialize)]
struct Hit {
    handle: String,
}

#[derive(Deserialize)]
struct Product {
    handle: String,
    title: String,
    #[serde(default)]
    vendor: Option<String>,
    #[serde(rename = "type", default)]
    kind: Option<String>,
    #[serde(default)]
    description: Option<String>,
    variants: Vec<Variant>,
}

#[derive(Deserialize)]
struct Variant {
    id: u64,
    title: String,
    sku: Option<String>,
}

async fn read_json<T: DeserializeOwned>(request: RequestBuilder) -> Result<T> {
    let mut response = request.send().await?;
    ensure!(response.status().is_success(), "Supplier request failed.");
    ensure!(
        response.content_length().unwrap_or(0) <= MAX_BYTES as u64,
        "Supplier response is too large."
    );
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            chunk.len() <= MAX_BYTES - body.len(),
            "Supplier response is too large."
        );
        body.extend_from_slice(&chunk);
    }
    Ok(serde_json::from_slice(&body)?)
}

fn valid_handle(handle: &str) -> bool {
    !handle.is_empty()
        && handle.len() <= 255
        && handle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
}

async fn search_store(
    client: &Client,
    base: &str,
    terms: &[String],
    part: &str,
) -> Result<Vec<SupplierListing>> {
    let mut incomplete = false;
    for term in terms {
        let handles = suggested_handles(client, base, term).await?;
        let mut listings = Vec::new();
        for handle in handles {
            let url = format!("{base}/products/{handle}");
            match read_json::<Product>(client.get(format!("{url}.js"))).await {
                Ok(product) if product.handle == handle && !product.title.trim().is_empty() => {
                    if !term.eq_ignore_ascii_case(part) || product.has_part(part) {
                        listings.extend(product.listings(&url, part));
                    }
                }
                _ => incomplete = true,
            }
        }
        if !listings.is_empty() {
            return Ok(listings);
        }
    }
    ensure!(
        !incomplete,
        "The supplier returned incomplete product details."
    );
    Ok(Vec::new())
}

async fn suggested_handles(client: &Client, base: &str, term: &str) -> Result<Vec<String>> {
    let request = client.get(format!("{base}/search/suggest.json")).query(&[
        ("q", term),
        ("resources[type]", "product"),
        ("resources[limit]", "2"),
        (
            "resources[options][fields]",
            "title,product_type,vendor,variants.sku,variants.title",
        ),
    ]);
    let suggestions: Suggestions = read_json(request).await?;
    let hits = suggestions.resources.results.products;
    let advertised_hits = !hits.is_empty();
    let mut handles = Vec::new();
    for hit in hits {
        if valid_handle(&hit.handle) && !handles.contains(&hit.handle) {
            handles.push(hit.handle);
        }
        if handles.len() == MAX_PRODUCTS {
            break;
        }
    }
    ensure!(
        !advertised_hits || !handles.is_empty(),
        "The supplier returned no usable product links."
    );
    Ok(handles)
}

/// Limits text by character count and removes markup control characters from fields.
fn compact(value: &str, limit: usize) -> String {
    let words = value.split_whitespace().collect::<Vec<_>>().join(" ");
    words
        .chars()
        .filter(|c| !c.is_control())
        .take(limit)
        .collect()
}

impl Variant {
    fn has_sku(&self, part: &str) -> bool {
        !part.is_empty()
            && self
                .sku
                .as_deref()
                .is_some_and(|sku| sku.trim().eq_ignore_ascii_case(part))
    }
}

impl Product {
    fn has_part(&self, part: &str) -> bool {
        let title = self.title.to_lowercase();
        let part = part.to_lowercase();
        self.variants.iter().any(|v| v.has_sku(&part))
            || title.match_indices(&part).any(|(start, text)| {
                let before = title[..start].chars().next_back();
                let after = title[start + text.len()..].chars().next();
                !before.is_some_and(char::is_alphanumeric)
                    && !after.is_some_and(char::is_alphanumeric)
            })
    }
    fn listings(&self, url: &str, part: &str) -> Vec<SupplierListing> {
        let matching: Vec<_> = self.variants.iter().filter(|v| v.has_sku(part)).collect();
        let description = self
            .description
            .as_deref()
            .map(crate::sources::html_to_text)
            .unwrap_or_default();
        let specific = if !matching.is_empty() {
            matching
        } else if self.variants.len() <= 4 {
            self.variants.iter().collect()
        } else {
            vec![]
        };
        if specific.is_empty() {
            return vec![self.listing(url, None, &description)];
        }
        specific
            .into_iter()
            .take(4)
            .map(|v| self.listing(url, Some(v), &description))
            .collect()
    }

    fn listing(&self, url: &str, variant: Option<&Variant>, description: &str) -> SupplierListing {
        let option = variant
            .map(|v| v.title.trim())
            .filter(|s| !s.is_empty() && *s != "Default Title");
        let title = match option {
            Some(o) => format!("{} · {}", compact(&self.title, 115), compact(o, 80)),
            None => compact(&self.title, 200),
        };
        let mut details = Vec::new();
        for (label, value) in [
            ("Brand", self.vendor.as_deref()),
            ("Type", self.kind.as_deref()),
            ("Supplier SKU", variant.and_then(|v| v.sku.as_deref())),
        ] {
            if let Some(v) = value.filter(|v| !v.trim().is_empty()) {
                details.push(format!("{label}: {}", compact(v, 70)));
            }
        }
        if variant.is_none() && !self.variants.is_empty() {
            details.push(format!(
                "{} variants. Confirm the variant on the supplier page.",
                self.variants.len()
            ));
        }
        if !description.trim().is_empty() {
            details.push(compact(description, 400));
        }
        let url = variant
            .map(|v| format!("{url}?variant={}", v.id))
            .unwrap_or_else(|| url.to_owned());
        SupplierListing {
            title,
            url,
            details: compact(&details.join(" · "), 650),
        }
    }
}

#[cfg(test)]
mod tests;
