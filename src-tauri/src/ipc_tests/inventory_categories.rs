//! The category, column and price commands of the workshop inventory, called as the frontend calls them.
use super::{app_with, args_in, scratch, send, InvokeBody, Library, MockRuntime, WebviewWindow};
use serde_json::{json, Value};
use std::sync::Arc;

const INVENTORY_API: &str = include_str!("../../../src/inventoryApi.ts");

/// Calls a command with the argument names that `src/inventoryApi.ts` sends.
fn call(w: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, String> {
    let mut sent: Vec<String> = args.as_object().map(|o| o.keys().cloned().collect()).unwrap_or_default();
    sent.sort();
    assert_eq!(sent, args_in(INVENTORY_API, cmd), "this test and inventoryApi.ts send different arguments with {cmd}");
    send(w, cmd, InvokeBody::Json(args), Default::default())
}

fn ok(w: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Value {
    call(w, cmd, args).unwrap_or_else(|e| panic!("{cmd} failed: {e}"))
}

fn draft(name: &str, category: &str, price_cents: Value) -> Value {
    json!({"name":name,"category":category,"brand":"","part_number":"","dimensions":"","quantity":3,
        "pack_quantity":10,"price_cents":price_cents,"notes":"","supplier":null})
}

fn save(w: &WebviewWindow<MockRuntime>, draft: Value) -> Value {
    send(w, "save_inventory", InvokeBody::Json(json!({"id":null,"draft":draft})), Default::default()).unwrap()
}

fn category(w: &WebviewWindow<MockRuntime>, name: &str) -> Option<Value> {
    ok(w, "list_inventory_categories", json!({})).as_array().unwrap().iter().find(|c| c["name"] == name).cloned()
}

#[test]
fn categories_columns_price_and_quantity_round_trip() {
    let dir = scratch();
    let lib = Arc::new(Library::open(&dir.join("library")).unwrap());
    let app = app_with(lib.clone());
    let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();

    // Every command refuses while the module is off.
    let refusals = [
        ("list_inventory_categories", json!({})),
        ("create_inventory_category", json!({"name":"Bolts"})),
        ("rename_inventory_category", json!({"from":"Bolts","to":"Screws"})),
        ("delete_inventory_category", json!({"name":"Bolts"})),
        ("set_inventory_category_columns", json!({"name":"Bolts","columns":null})),
        ("get_inventory_view_settings", json!({})),
        ("set_inventory_all_columns", json!({"columns":null})),
        ("set_inventory_currency", json!({"code":"USD"})),
        ("set_inventory_quantity", json!({"id":"x","quantity":1})),
        ("set_inventory_open_amount", json!({"id":"x","left":100})),
        ("suggest_inventory_fields", json!({"input":{"name":"PETG","brand":"","description":"","category":"","colour":""}})),
        ("review_inventory_suggestions", json!({})),
        ("apply_inventory_suggestions", json!({"changes":[]})),
        ("set_inventory_category_keywords", json!({"name":"Bolts","keywords":["x"]})),
    ];
    for (cmd, args) in &refusals {
        let error = call(&w, cmd, args.clone()).unwrap_err();
        assert!(error.contains("off"), "{cmd}: {error}");
    }
    send(&w, "set_inventory_enabled", InvokeBody::Json(json!({"enabled":true})), Default::default()).unwrap();

    // Price survives save and list, and a missing price stays null.
    let priced = save(&w, draft("M3 bolt", "Bolt", json!(1999)));
    let unpriced = save(&w, draft("M4 bolt", "Bolt", Value::Null));
    assert_eq!(priced["price_cents"], 1999);
    assert!(unpriced["price_cents"].is_null());
    let listed = send(&w, "list_inventory", InvokeBody::Json(json!({})), Default::default()).unwrap();
    let find = |name: &str| listed.as_array().unwrap().iter().find(|i| i["name"] == name).unwrap().clone();
    assert_eq!(find("M3 bolt")["price_cents"], 1999);
    assert!(find("M4 bolt")["price_cents"].is_null());
    let negative = send(&w, "save_inventory", InvokeBody::Json(json!({"id":null,"draft":draft("Bad","Bolt",json!(-1))})), Default::default());
    assert!(negative.is_err());

    // Create, set columns (the row is made when absent), refuse a duplicate and an unknown column.
    ok(&w, "create_inventory_category", json!({"name":"Bolts"}));
    assert!(call(&w, "create_inventory_category", json!({"name":"bolts"})).is_err());
    assert_eq!(category(&w, "Bolts").unwrap(), json!({"name":"Bolts","count":0,"columns":null,"keywords":[]}));
    ok(&w, "set_inventory_category_columns", json!({"name":"Bolts","columns":["dimensions","price"]}));
    assert!(call(&w, "set_inventory_category_columns", json!({"name":"Bolts","columns":["dimensions","dimensions"]})).is_err());
    assert!(call(&w, "set_inventory_category_columns", json!({"name":"Bolts","columns":["weight"]})).is_err());
    assert_eq!(category(&w, "Bolts").unwrap()["columns"], json!(["dimensions","price"]));
    assert_eq!(category(&w, "Bolt").unwrap()["count"], 2);

    // Rename that merges: both items move, the target keeps its spelling and columns, nothing is lost.
    assert_eq!(ok(&w, "rename_inventory_category", json!({"from":"Bolt","to":"bolts"})), 2);
    assert!(category(&w, "Bolt").is_none());
    assert_eq!(category(&w, "Bolts").unwrap(), json!({"name":"Bolts","count":2,"columns":["dimensions","price"],"keywords":[]}));
    let listed = send(&w, "list_inventory", InvokeBody::Json(json!({})), Default::default()).unwrap();
    let moved: Vec<_> = listed.as_array().unwrap().iter().map(|i| (i["category"].clone(), i["price_cents"].clone())).collect();
    assert_eq!(moved.len(), 2);
    assert!(moved.iter().all(|(category, _)| category == "Bolts"));
    assert!(moved.iter().any(|(_, price)| price == &json!(1999)));
    assert!(call(&w, "rename_inventory_category", json!({"from":"Nothing","to":"Other"})).is_err());

    // Delete only when empty.
    let refused = call(&w, "delete_inventory_category", json!({"name":"Bolts"})).unwrap_err();
    assert!(refused.contains("2 items"), "{refused}");
    ok(&w, "create_inventory_category", json!({"name":"Spare"}));
    ok(&w, "delete_inventory_category", json!({"name":"Spare"}));
    assert!(category(&w, "Spare").is_none());

    // Count steppers change only the count and return the whole item.
    let id = priced["id"].as_str().unwrap();
    let stepped = ok(&w, "set_inventory_quantity", json!({"id":id,"quantity":7}));
    assert_eq!((&stepped["quantity"], &stepped["price_cents"], &stepped["name"]), (&json!(7), &json!(1999), &json!("M3 bolt")));
    assert_eq!(ok(&w, "set_inventory_quantity", json!({"id":id,"quantity":0}))["quantity"], 0);
    assert!(call(&w, "set_inventory_quantity", json!({"id":id,"quantity":-1})).is_err());
    assert!(call(&w, "set_inventory_quantity", json!({"id":"missing","quantity":1})).is_err());

    // View settings: the currency and the All categories columns.
    let settings = ok(&w, "get_inventory_view_settings", json!({}));
    assert_eq!(settings, json!({"currency":"CAD","all_columns":null}));
    ok(&w, "set_inventory_currency", json!({"code":"USD"}));
    assert!(call(&w, "set_inventory_currency", json!({"code":"usd"})).is_err());
    ok(&w, "set_inventory_all_columns", json!({"columns":["category","quantity","price"]}));
    assert_eq!(
        ok(&w, "get_inventory_view_settings", json!({})),
        json!({"currency":"USD","all_columns":["category","quantity","price"]})
    );
    ok(&w, "set_inventory_all_columns", json!({"columns":null}));
    assert!(ok(&w, "get_inventory_view_settings", json!({}))["all_columns"].is_null());

    // Switching the module off again closes the new commands too.
    send(&w, "set_inventory_enabled", InvokeBody::Json(json!({"enabled":false})), Default::default()).unwrap();
    for (cmd, args) in &refusals {
        assert!(call(&w, cmd, args.clone()).unwrap_err().contains("off"), "{cmd}");
    }
    std::fs::remove_dir_all(dir).ok();
}

fn spool(measure_unit: &str, full: Value, left: Value, quantity: Value) -> Value {
    json!({"name":"PETG spool","category":"Filament","brand":"","part_number":"","dimensions":"","quantity":quantity,
        "pack_quantity":null,"price_cents":2499,"measure_unit":measure_unit,"measure_full":full,"measure_left":left,
        "notes":"","supplier":null})
}

#[test]
fn the_open_pack_round_trips_and_the_quick_edit_checks_the_item() {
    let dir = scratch();
    let lib = Arc::new(Library::open(&dir.join("library")).unwrap());
    let app = app_with(lib.clone());
    let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    send(&w, "set_inventory_enabled", InvokeBody::Json(json!({"enabled":true})), Default::default()).unwrap();

    // The three fields survive save and list, and a draft that breaks a rule is refused.
    let saved = save(&w, spool("g", json!(1000), json!(100), json!(1)));
    assert_eq!(
        (&saved["measure_unit"], &saved["measure_full"], &saved["measure_left"]),
        (&json!("g"), &json!(1000), &json!(100))
    );
    let listed = send(&w, "list_inventory", InvokeBody::Json(json!({})), Default::default()).unwrap();
    assert_eq!(listed[0]["measure_left"], 100);
    for bad in [
        spool("kg", json!(1), Value::Null, json!(1)),
        spool("", json!(1000), Value::Null, json!(1)),
        spool("g", Value::Null, json!(5), json!(1)),
        spool("g", json!(1000), json!(1001), json!(1)),
    ] {
        let refused = send(&w, "save_inventory", InvokeBody::Json(json!({"id":null,"draft":bad})), Default::default());
        assert!(refused.is_err(), "{bad}");
    }
    // A draft from an older frontend has no measure fields at all.
    let old = send(
        &w,
        "save_inventory",
        InvokeBody::Json(json!({"id":null,"draft":{"name":"Old","category":"Other"}})),
        Default::default(),
    )
    .unwrap();
    assert_eq!((&old["measure_unit"], &old["measure_full"], &old["measure_left"]), (&json!(""), &Value::Null, &Value::Null));

    // The quick edit changes only the open amount and returns the whole item.
    let id = saved["id"].as_str().unwrap();
    let edited = ok(&w, "set_inventory_open_amount", json!({"id":id,"left":250}));
    assert_eq!(
        (&edited["measure_left"], &edited["quantity"], &edited["price_cents"], &edited["name"]),
        (&json!(250), &json!(1), &json!(2499), &json!("PETG spool"))
    );
    assert_eq!(ok(&w, "set_inventory_open_amount", json!({"id":id,"left":0}))["measure_left"], 0);
    assert!(call(&w, "set_inventory_open_amount", json!({"id":id,"left":1001})).is_err());
    assert!(call(&w, "set_inventory_open_amount", json!({"id":id,"left":-1})).is_err());
    assert!(ok(&w, "set_inventory_open_amount", json!({"id":id,"left":null}))["measure_left"].is_null());
    let old_id = old["id"].as_str().unwrap();
    let unmeasured = call(&w, "set_inventory_open_amount", json!({"id":old_id,"left":5})).unwrap_err();
    assert!(unmeasured.contains("full pack"), "{unmeasured}");
    assert!(call(&w, "set_inventory_open_amount", json!({"id":"missing","left":5})).is_err());

    // A count of zero clears the open amount, and the quick edit then refuses to set one.
    ok(&w, "set_inventory_open_amount", json!({"id":id,"left":400}));
    assert!(ok(&w, "set_inventory_quantity", json!({"id":id,"quantity":0}))["measure_left"].is_null());
    let none_open = call(&w, "set_inventory_open_amount", json!({"id":id,"left":10})).unwrap_err();
    assert!(none_open.contains("none on hand"), "{none_open}");
    assert_eq!(ok(&w, "set_inventory_quantity", json!({"id":id,"quantity":2}))["measure_full"], 1000);
    // A lower count assumes the opened pack was used up; a higher one keeps it.
    ok(&w, "set_inventory_open_amount", json!({"id":id,"left":400}));
    assert_eq!(ok(&w, "set_inventory_quantity", json!({"id":id,"quantity":3}))["measure_left"], 400);
    assert!(ok(&w, "set_inventory_quantity", json!({"id":id,"quantity":2}))["measure_left"].is_null());

    // The module switch closes the command too.
    send(&w, "set_inventory_enabled", InvokeBody::Json(json!({"enabled":false})), Default::default()).unwrap();
    assert!(call(&w, "set_inventory_open_amount", json!({"id":id,"left":1})).unwrap_err().contains("off"));
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn intake_suggestions_round_trip_through_review_and_apply() {
    let dir = scratch();
    let lib = Arc::new(Library::open(&dir.join("library")).unwrap());
    let app = app_with(lib.clone());
    let w = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
    send(&w, "set_inventory_enabled", InvokeBody::Json(json!({"enabled":true})), Default::default()).unwrap();
    for name in ["Filament", "Adhesives"] {
        ok(&w, "create_inventory_category", json!({"name":name}));
    }
    let spool = save(&w, draft("Overture PETG Filament Purple", "Materials", Value::Null));
    let glue = save(&w, draft("Gorilla Glue Sticks", "Other", Value::Null));
    let id = |item: &Value| item["id"].as_str().unwrap().to_owned();

    // The editor asks for suggestions with the names the TypeScript wrapper sends.
    let input = |name: &str, category: &str, colour: &str| {
        json!({"input":{"name":name,"brand":"","description":"","category":category,"colour":colour}})
    };
    let found = ok(&w, "suggest_inventory_fields", input("Overture PETG Filament Purple", "", ""));
    assert_eq!(
        found,
        json!({"category":{"name":"Filament","reason":"name says \"filament\""},"colour":"Purple"})
    );
    let kept = ok(&w, "suggest_inventory_fields", input("Overture PETG Filament Purple", "Filament", "Teal"));
    assert_eq!(kept, json!({"category":null,"colour":null}));

    // Designer keywords: validated, listed with the category, and used by the rules.
    ok(&w, "set_inventory_category_keywords", json!({"name":"Adhesives","keywords":["gorilla","  Stick "]}));
    assert_eq!(category(&w, "Adhesives").unwrap()["keywords"], json!(["gorilla", "Stick"]));
    assert!(call(&w, "set_inventory_category_keywords", json!({"name":"Adhesives","keywords":["a","A"]})).is_err());
    assert!(call(&w, "set_inventory_category_keywords", json!({"name":"Adhesives","keywords":[""]})).is_err());

    // Review changes nothing and names its reasons; apply changes only what was ticked.
    let review = ok(&w, "review_inventory_suggestions", json!({}));
    let review = review.as_array().unwrap();
    assert_eq!(review.len(), 2);
    let row = |item: &Value| review.iter().find(|r| r["id"] == item["id"]).unwrap().clone();
    assert_eq!(
        row(&spool),
        json!({"id":id(&spool),"name":"Overture PETG Filament Purple","category_from":"Materials","category_to":"Filament",
            "category_reason":"name says \"filament\"","colour_to":"Purple","colour_reason":"name says \"Purple\""})
    );
    assert_eq!(row(&glue)["category_to"], "Adhesives");
    assert!(row(&glue)["colour_to"].is_null());
    let listed = send(&w, "list_inventory", InvokeBody::Json(json!({})), Default::default()).unwrap();
    assert!(listed.as_array().unwrap().iter().any(|i| i["category"] == "Materials"));

    let changed = ok(&w, "apply_inventory_suggestions", json!({"changes":[
        {"id":id(&spool),"category":"Filament","colour":"Purple"},
        {"id":"missing","category":"Filament"}
    ]}));
    assert_eq!(changed, 1);
    let listed = send(&w, "list_inventory", InvokeBody::Json(json!({})), Default::default()).unwrap();
    let after = |item: &Value| listed.as_array().unwrap().iter().find(|i| i["id"] == item["id"]).unwrap().clone();
    assert_eq!((after(&spool)["category"].clone(), after(&spool)["colour"].clone()), (json!("Filament"), json!("Purple")));
    assert_eq!(after(&glue)["category"], "Other");
    // The same ticks again change nothing: the review is stale.
    assert_eq!(ok(&w, "apply_inventory_suggestions", json!({"changes":[{"id":id(&spool),"category":"Filament","colour":"Purple"}]})), 0);
    assert_eq!(ok(&w, "apply_inventory_suggestions", json!({"changes":[{"id":id(&glue),"category":"Adhesives"}]})), 1);
    assert!(ok(&w, "review_inventory_suggestions", json!({})).as_array().unwrap().is_empty());

    // The price source travels with the draft.
    let estimate = send(&w, "save_inventory", InvokeBody::Json(json!({"id":null,"draft":{"name":"Est","category":"Other",
        "price_cents":1500,"price_source":"listing:www.canadabolts.ca"}})), Default::default()).unwrap();
    assert_eq!(estimate["price_source"], "listing:www.canadabolts.ca");
    let refused = send(&w, "save_inventory", InvokeBody::Json(json!({"id":null,"draft":{"name":"Est","price_source":"made up"}})), Default::default());
    assert!(refused.is_err());
    std::fs::remove_dir_all(dir).ok();
}
