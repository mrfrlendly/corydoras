use super::*;
use std::collections::HashMap;
use std::sync::Mutex;
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
        ..Default::default()
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

#[test]
fn reads_prices_from_cents_and_from_dollar_strings_without_rounding_error() {
    assert_eq!(parse_price(&json!(1299)), Some(1299));
    assert_eq!(parse_price(&json!(0)), None);
    assert_eq!(parse_price(&json!("0.00")), None);
    assert_eq!(parse_price(&json!(1)), Some(1));
    assert_eq!(parse_price(&json!("12.99")), Some(1299));
    assert_eq!(parse_price(&json!("12.5")), Some(1250));
    assert_eq!(parse_price(&json!(" 7 ")), Some(700));
    assert_eq!(parse_price(&json!("0.07")), Some(7));
    assert_eq!(parse_price(&json!("19.90")), Some(1990));
    assert_eq!(parse_price(&json!(MAX_PRICE_CENTS)), Some(MAX_PRICE_CENTS));
}

#[test]
fn ignores_missing_negative_oversized_and_odd_prices() {
    for odd in [
        json!(null),
        json!(-1),
        json!("-1.00"),
        json!(MAX_PRICE_CENTS + 1),
        json!(u64::MAX),
        json!("99999999999999999999999"),
        json!("12.999"),
        json!("1,299.00"),
        json!("12.9x"),
        json!("$12.99"),
        json!(""),
        json!("."),
        json!(12.99),
        json!(true),
        json!([1299]),
        json!({"amount": 1299}),
    ] {
        assert_eq!(parse_price(&odd), None, "{odd}");
    }
    let variants: Vec<Variant> = serde_json::from_value(json!([
        {"id":1,"title":"A","sku":null},
        {"id":2,"title":"B","sku":null,"price":-5},
        {"id":3,"title":"C","sku":null,"price":{"x":1}},
        {"id":4,"title":"D","sku":null,"price":"3.50"},
        {"id":5,"title":"E","sku":null,"price":350}
    ]))
    .unwrap();
    let prices: Vec<_> = variants.iter().map(|v| v.price).collect();
    assert_eq!(prices, [None, None, None, Some(350), Some(350)]);
}

#[test]
fn accepts_only_three_uppercase_letters_as_a_currency() {
    assert_eq!(valid_currency("CAD"), Some("CAD"));
    assert_eq!(valid_currency("GBP"), Some("GBP"));
    for bad in ["", "cad", "Cad", "CA", "CADD", "C4D", "CÀD", " CAD", "CAD\n"] {
        assert_eq!(valid_currency(bad), None, "{bad:?}");
    }
}

fn product(variants: serde_json::Value) -> Product {
    serde_json::from_value(json!({"handle":"p","title":"Cup screw","variants":variants})).unwrap()
}

#[test]
fn a_listing_for_a_sku_matched_variant_carries_that_variants_price() {
    let p = product(json!([
        {"id":1,"title":"M3 × 8","sku":"A-8","price":150},
        {"id":2,"title":"M3 × 12","sku":"A-12","price":"2.25"},
        {"id":3,"title":"M3 × 16","sku":"A-16","price":300},
        {"id":4,"title":"M3 × 20","sku":"A-20","price":350},
        {"id":5,"title":"M3 × 25","sku":"A-25","price":400}
    ]));
    let listings = p.listings("https://shop.example/products/p", "a-12");
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].price_cents, Some(225));
    assert!(listings[0].url.ends_with("?variant=2"));
}

#[test]
fn a_listing_without_a_variant_has_a_price_only_when_all_variants_agree() {
    let many = |prices: [serde_json::Value; 5]| {
        let variants: Vec<_> = prices
            .into_iter()
            .enumerate()
            .map(|(i, price)| json!({"id":i + 1,"title":format!("V{i}"),"sku":null,"price":price}))
            .collect();
        product(json!(variants))
    };
    let same = many([json!(500), json!(500), json!("5.00"), json!(500), json!(500)]);
    let listings = same.listings("https://shop.example/products/p", "");
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].price_cents, Some(500));
    assert!(listings[0].details.contains("5 variants."));

    let mixed = many([json!(500), json!(500), json!(600), json!(500), json!(500)]);
    assert_eq!(mixed.listings("https://shop.example/products/p", "")[0].price_cents, None);

    let one_unknown = many([json!(500), json!(500), json!(null), json!(500), json!(500)]);
    assert_eq!(
        one_unknown.listings("https://shop.example/products/p", "")[0].price_cents,
        None
    );
    assert_eq!(product(json!([])).common_price(), None);
}

#[test]
fn small_products_list_each_variant_with_its_own_price() {
    let p = product(json!([
        {"id":1,"title":"Red","sku":null,"price":100},
        {"id":2,"title":"Blue","sku":null,"price":200}
    ]));
    let prices: Vec<_> = p
        .listings("https://shop.example/products/p", "")
        .iter()
        .map(|l| l.price_cents)
        .collect();
    assert_eq!(prices, [Some(100), Some(200)]);
}

#[tokio::test]
async fn a_store_search_reads_prices_and_the_currency_from_meta_json() {
    let server = serve(|r| match r.path.as_str() {
        "/search/suggest.json" => hits(&["cup-screw"]),
        "/products/cup-screw.js" => Resp::json(
            200,
            json!({"handle":"cup-screw","title":"Cup screw","variants":[
                {"id":1,"title":"M3 × 8","sku":"A-8","price":150},
                {"id":2,"title":"M3 × 12","sku":"A-12","price":225}]}),
        ),
        "/meta.json" => Resp::json(200, json!({"currency":"CAD","name":"Shop"})),
        _ => Resp::empty(404),
    })
    .await;
    let listings =
        search_store_with_currency(
            &Client::new(),
            &server.base,
            &["A-12".into()],
            "a-12",
            &no_cache(),
        )
        .await
            .unwrap();
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].price_cents, Some(225));
    assert_eq!(listings[0].currency, "CAD");
    assert_eq!(server.seen_at("/meta.json").len(), 1);
}

#[tokio::test]
async fn an_unreadable_or_invalid_currency_stays_empty_and_does_not_fail_the_search() {
    for meta in [
        Resp::empty(404),
        Resp::json(200, json!({"currency":"cad"})),
        Resp::json(200, json!({"currency":"CADX"})),
        Resp::json(200, json!({"currency":42})),
        Resp::json(200, json!({"name":"Shop"})),
        Resp::json(200, json!({"currency":"CAD","padding":"x".repeat(MAX_BYTES)})),
    ] {
        let meta = std::sync::Mutex::new(Some(meta));
        let server = serve(move |r| match r.path.as_str() {
            "/search/suggest.json" => hits(&["cup-screw"]),
            "/products/cup-screw.js" => Resp::json(
                200,
                json!({"handle":"cup-screw","title":"Cup screw","variants":[
                    {"id":1,"title":"Default Title","sku":null,"price":150}]}),
            ),
            "/meta.json" => meta.lock().unwrap().take().unwrap_or_else(|| Resp::empty(500)),
            _ => Resp::empty(404),
        })
        .await;
        let listings =
            search_store_with_currency(
            &Client::new(),
            &server.base,
            &["screw".into()],
            "",
            &no_cache(),
        )
        .await
                .unwrap();
        assert_eq!(listings[0].price_cents, Some(150));
        assert_eq!(listings[0].currency, "");
    }
}

fn no_cache() -> Mutex<Option<HashMap<String, String>>> {
    Mutex::new(None)
}

fn limited(host: &str) -> anyhow::Error {
    RateLimited {
        host: host.into(),
        retry_after: None,
    }
    .into()
}

#[test]
fn a_zero_price_is_not_a_price_in_any_listing() {
    let p = product(json!([
        {"id":1,"title":"Free","sku":"A-1","price":0},
        {"id":2,"title":"Free too","sku":"A-2","price":"0.00"}
    ]));
    assert!(p
        .listings("https://shop.example/products/p", "")
        .iter()
        .all(|l| l.price_cents.is_none()));
    let same_zero = product(json!([
        {"id":1,"title":"A","sku":null,"price":0},
        {"id":2,"title":"B","sku":null,"price":0}
    ]));
    assert_eq!(same_zero.common_price(), None);
    assert_eq!(
        same_zero.listing("https://shop.example/products/p", None, "").price_cents,
        None
    );
}

#[tokio::test]
async fn a_429_on_a_product_read_stops_the_store_at_once() {
    let server = serve(|r| match r.path.as_str() {
        "/search/suggest.json" => hits(&["first", "second"]),
        "/products/first.js" => Resp::empty(429),
        _ => Resp::empty(404),
    })
    .await;
    let host = server.base.trim_start_matches("http://").to_owned();
    let error = search_store_with_currency(
        &Client::new(),
        &server.base,
        &["A-1".into(), "screw".into()],
        "A-1",
        &no_cache(),
    )
    .await
    .unwrap_err();
    assert_eq!(
        error.to_string(),
        format!("{host} is limiting requests right now. Try again in a minute.")
    );
    let paths: Vec<_> = server.seen().into_iter().map(|r| r.path).collect();
    assert_eq!(
        paths,
        ["/meta.json", "/search/suggest.json", "/products/first.js"]
    );
}

#[tokio::test]
async fn a_503_or_a_429_on_the_search_or_on_meta_json_also_stops_the_store() {
    for limited_path in ["/search/suggest.json", "/meta.json"] {
        for status in [429, 503] {
            let server = serve(move |r| match r.path.as_str() {
                p if p == limited_path => Resp::empty(status),
                "/search/suggest.json" => hits(&["first"]),
                _ => Resp::empty(404),
            })
            .await;
            let error = search_store_with_currency(
                &Client::new(),
                &server.base,
                &["a".into(), "b".into()],
                "",
                &no_cache(),
            )
            .await
            .unwrap_err();
            assert!(error.is::<RateLimited>(), "{limited_path} {status}");
            let suggests = server.seen_at("/search/suggest.json").len();
            assert_eq!(suggests, usize::from(limited_path != "/meta.json"));
            assert!(server.seen_at("/products/first.js").is_empty());
        }
    }
}

#[tokio::test]
async fn a_limited_store_keeps_the_results_of_the_other_stores() {
    let limited_server = serve(|_| Resp::empty(429)).await;
    let good = serve(|r| match r.path.as_str() {
        "/search/suggest.json" => hits(&["cup-screw"]),
        "/products/cup-screw.js" => Resp::json(
            200,
            json!({"handle":"cup-screw","title":"Cup screw","variants":[
                {"id":1,"title":"Default Title","sku":null,"price":150}]}),
        ),
        "/meta.json" => Resp::json(200, json!({"currency":"CAD"})),
        _ => Resp::empty(404),
    })
    .await;
    let terms = ["screw".to_string()];
    let client = Client::new();
    let cache = no_cache();
    let (a, b) = tokio::join!(
        search_store_with_currency(&client, &limited_server.base, &terms, "", &cache),
        search_store_with_currency(&client, &good.base, &terms, "", &cache),
    );
    let listings = combine([a, b]).unwrap();
    assert_eq!(listings.len(), 1);
    assert_eq!(listings[0].price_cents, Some(150));
    assert_eq!(listings[0].currency, "CAD");
}

#[test]
fn combine_names_the_limited_stores_and_keeps_the_generic_text_for_other_failures() {
    let two = combine::<3>([
        Err(limited("canadabolts.ca")),
        Err(limited("thepihut.com")),
        Ok(vec![]),
    ])
    .unwrap_err()
    .to_string();
    assert_eq!(
        two,
        "canadabolts.ca, thepihut.com are limiting requests right now. Try again in a minute."
    );
    let mixed = combine([Err(limited("canadabolts.ca")), Err(anyhow!("offline"))])
        .unwrap_err()
        .to_string();
    assert!(mixed.starts_with(
        "canadabolts.ca is limiting requests right now. Try again in a minute. One or more supplier"
    ));
    let other = combine([Err(anyhow!("offline")), Ok(vec![])])
        .unwrap_err()
        .to_string();
    assert!(other.starts_with("One or more supplier catalogues did not respond"));
    assert!(!other.contains("limiting"));
    let hit = SupplierListing {
        title: "Bolt".into(),
        ..Default::default()
    };
    assert_eq!(
        combine([Err(limited("canadabolts.ca")), Ok(vec![hit])])
            .unwrap()
            .len(),
        1
    );
}

#[test]
fn the_retry_after_header_is_only_reported() {
    assert_eq!(retry_after_seconds(" 60 "), Some(60));
    assert_eq!(retry_after_seconds("Wed, 21 Oct 2026 07:28:00 GMT"), None);
    let message = |retry_after| {
        RateLimited {
            host: "shop.example".into(),
            retry_after,
        }
        .to_string()
    };
    assert_eq!(
        message(Some(60)),
        "shop.example is limiting requests right now. Try again in a minute."
    );
    assert!(message(Some(600)).ends_with("Try again in about 10 minutes."));
    assert!(message(None).ends_with("Try again in a minute."));
}

#[tokio::test]
async fn the_currency_is_read_once_per_store_and_a_failed_read_is_retried() {
    let server = serve(|r| match r.path.as_str() {
        "/search/suggest.json" => hits(&[]),
        "/meta.json" => Resp::json(200, json!({"currency":"GBP"})),
        _ => Resp::empty(404),
    })
    .await;
    let cache = no_cache();
    for _ in 0..2 {
        search_store_with_currency(&Client::new(), &server.base, &["x".into()], "", &cache)
            .await
            .unwrap();
    }
    assert_eq!(server.seen_at("/meta.json").len(), 1);
    assert_eq!(
        store_currency(&Client::new(), &server.base, &cache)
            .await
            .unwrap(),
        "GBP"
    );

    let failing = serve(|r| match r.path.as_str() {
        "/search/suggest.json" => hits(&[]),
        _ => Resp::empty(404),
    })
    .await;
    let cache = no_cache();
    for _ in 0..2 {
        assert_eq!(
            store_currency(&Client::new(), &failing.base, &cache)
                .await
                .unwrap(),
            ""
        );
    }
    assert_eq!(failing.seen_at("/meta.json").len(), 2);
}
