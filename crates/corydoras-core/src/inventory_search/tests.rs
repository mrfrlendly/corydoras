use super::*;
use anyhow::anyhow;

use crate::testserver::{serve, Resp};
use serde_json::json;

fn hits(handles: &[&str]) -> Resp {
    Resp::json(
        200,
        json!({"resources":{"results":{"products":handles.iter()
        .map(|h| json!({"handle":h,"url":"https://unrelated.example/page"})).collect::<Vec<_>>()}}}),
    )
}

#[test]
fn validates_identifiers_and_uses_part_number_first() {
    assert_eq!(
        queries("  Socket screws ", "ACME", "A-20").unwrap(),
        ["A-20", "ACME Socket screws"]
    );
    assert_eq!(queries("ACME A-20", "acme", "").unwrap(), ["ACME A-20"]);
    assert!(queries("", "Brand", "").is_err());
    assert!(queries("bolt\nfile", "Brand", "").is_err());
    assert!(queries(&"x".repeat(500), "", "").is_err());
}

#[tokio::test]
async fn follows_only_valid_handles_and_returns_supplier_evidence_for_the_matching_variant() {
    let server = serve(|r| match r.path.as_str() {
        "/search/suggest.json" => hits(&["../tracking", "//evil.example", "cup-screw"]),
        "/products/cup-screw.js" => Resp::json(
            200,
            json!({
               "handle":"cup-screw","title":"Cup screw","vendor":"Acme","type":"Hardware",
               "description":"<p>Stainless <b>steel</b>.</p>","variants":[
                   {"id":1,"title":"M3 × 8","sku":"A-8"},{"id":2,"title":"M3 × 12","sku":"A-12"}]
            }),
        ),
        _ => Resp::empty(404),
    })
    .await;
    let listings = search_store(&Client::new(), &server.base, &["A-12".into()], "a-12")
        .await
        .unwrap();
    assert_eq!(server.seen().len(), 2);
    assert_eq!(listings.len(), 1);
    assert!(listings[0].url.ends_with("/products/cup-screw?variant=2"));
    assert!(listings[0].title.contains("M3 × 12"));
    assert!(listings[0].details.contains("A-12"));
    assert!(listings[0].details.contains("Stainless steel."));
    assert!(!listings[0].details.contains("<b>"));
}

#[tokio::test]
async fn rejects_bad_responses_and_cross_product_details() {
    let server = serve(|r| match r.path.as_str() {
        "/search/suggest.json" => hits(&["bolt"]),
        _ => Resp::json(200, json!({"handle":"nut","title":"Nut","variants":[]})),
    })
    .await;
    assert!(
        search_store(&Client::new(), &server.base, &["bolt".into()], "")
            .await
            .is_err()
    );
    let server = serve(|_| Resp::json(200, json!({"message":"Not available"}))).await;
    assert!(
        search_store(&Client::new(), &server.base, &["bolt".into()], "")
            .await
            .is_err()
    );
}

#[tokio::test]
async fn an_empty_sku_search_retries_with_the_product_name() {
    let server = serve(|_| hits(&[])).await;
    let listings = search_store(
        &Client::new(),
        &server.base,
        &["A-20".into(), "screw".into()],
        "A-20",
    )
    .await
    .unwrap();
    assert!(listings.is_empty());
    assert_eq!(server.seen().len(), 2);
}

#[tokio::test]
async fn rejects_large_responses() {
    let server = serve(|_| {
        Resp::json(
            200,
            json!({"resources":{"results":{"products":[]}}, "padding":"x".repeat(MAX_BYTES)}),
        )
    })
    .await;
    assert!(
        search_store(&Client::new(), &server.base, &["bolt".into()], "")
            .await
            .is_err()
    );
}

#[test]
fn reports_incomplete_search_when_a_store_fails_and_no_matches_exist() {
    assert!(combine([Ok(vec![]), Err(anyhow!("offline"))]).is_err());
    assert!(combine([Ok(vec![]), Ok(vec![])]).unwrap().is_empty());
    let hit = SupplierListing {
        title: "Bolt".into(),
        url: "https://example.org/products/bolt".into(),
        details: "Acme".into(),
    };
    assert_eq!(
        combine([Err(anyhow!("offline")), Ok(vec![hit])])
            .unwrap()
            .len(),
        1
    );
}

#[tokio::test]
#[ignore = "Reads public supplier catalogues. Run manually with network access."]
async fn live_catalogues_return_current_product_details() {
    let client = Client::builder()
        .https_only(true)
        .redirect(redirect::Policy::none())
        .timeout(Duration::from_secs(8))
        .user_agent("Corydoras inventory")
        .build()
        .unwrap();
    for (base, name) in STORES.iter().zip(["screw", "filament", "pico"]) {
        let hits = search_store(&client, base, &[name.into()], "")
            .await
            .unwrap();
        assert!(
            hits.iter()
                .any(|h| h.url.starts_with(&format!("{base}/products/"))
                    && !h.title.is_empty()
                    && !h.details.is_empty()),
            "Missing product details from {base}"
        );
    }
}

#[tokio::test]
async fn unrelated_hits_for_the_part_number_do_not_prevent_a_name_search() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    let requests = AtomicUsize::new(0);
    let server = serve(move |r| match r.path.as_str() {
        "/search/suggest.json" if requests.fetch_add(1, Ordering::SeqCst) == 0 => hits(&["wrong-product"]),
        "/search/suggest.json" => hits(&["actual-product", "actual-product"]),
        "/products/wrong-product.js" => Resp::json(200,json!({"handle":"wrong-product","title":"A-1200 Bracket","variants":[{"sku":"A-1200","title":"Default Title","id":12}]})),
        "/products/actual-product.js" => Resp::json(200,json!({"handle":"actual-product","title":"M3 screw","variants":[]})),
        _ => Resp::empty(404),
    }).await;
    let listings = search_store(
        &Client::new(),
        &server.base,
        &["A-12".into(), "M3 screw".into()],
        "A-12",
    )
    .await
    .unwrap();
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].title, "M3 screw");
    assert_eq!(server.seen_at("/products/actual-product.js").len(), 1);
}
