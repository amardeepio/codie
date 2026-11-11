//! Credential store tests (`SPEC-PROV-004`).

use codie_ai::{AuthStore, resolve_key};
use codie_testkit::spec;
use serde_json::json;

#[spec("PROV-030")]
#[spec("SEC-006")]
#[test]
fn store_round_trips_and_is_private() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("auth.json");
    let store = AuthStore::new(&path);

    store.set("deepseek", "sk-deepseek").expect("store key");
    store.set("openrouter", "sk-openrouter").expect("store key");

    assert_eq!(
        store.get("deepseek").expect("read key").as_deref(),
        Some("sk-deepseek")
    );
    assert_eq!(
        store.list().expect("list providers"),
        vec!["deepseek".to_owned(), "openrouter".to_owned()]
    );

    let file = std::fs::read(&path).expect("auth file exists");
    let value: serde_json::Value = serde_json::from_slice(&file).expect("valid JSON");
    assert_eq!(
        value,
        json!({
            "providers": {
                "deepseek": {"api_key": "sk-deepseek"},
                "openrouter": {"api_key": "sk-openrouter"}
            }
        })
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path)
            .expect("metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600, "auth.json must be mode 0600");
    }

    let leftovers: Vec<_> = std::fs::read_dir(directory.path())
        .expect("read temp dir")
        .filter_map(Result::ok)
        .map(|entry| entry.file_name())
        .filter(|name| name != "auth.json")
        .collect();
    assert!(
        leftovers.is_empty(),
        "atomic writes must not leave temp files: {leftovers:?}"
    );
}

#[spec("PROV-032")]
#[test]
fn remove_only_touches_one_provider() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = AuthStore::new(directory.path().join("auth.json"));
    store.set("deepseek", "sk-deepseek").expect("store key");
    store.set("openrouter", "sk-openrouter").expect("store key");

    assert!(store.remove("deepseek").expect("remove key"));
    assert_eq!(store.get("deepseek").expect("read key"), None);
    assert_eq!(
        store.get("openrouter").expect("read key").as_deref(),
        Some("sk-openrouter")
    );
    assert!(!store.remove("deepseek").expect("remove missing"));
}

#[test]
fn malformed_file_is_an_error() {
    let directory = tempfile::tempdir().expect("temp dir");
    let path = directory.path().join("auth.json");
    std::fs::write(&path, "{not json").expect("write malformed file");
    let store = AuthStore::new(&path);

    let error = store.get("deepseek").expect_err("malformed JSON must fail");
    assert!(error.to_string().contains("not valid JSON"), "{error}");
    assert!(
        store
            .set("deepseek", "sk-deepseek")
            .expect_err("set must not silently replace a malformed file")
            .to_string()
            .contains("not valid JSON")
    );
}

#[spec("PROV-031")]
#[test]
fn resolution_order() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = AuthStore::new(directory.path().join("auth.json"));

    let environment = std::env::var("PATH").expect("PATH is set for test processes");

    assert_eq!(
        resolve_key(Some("explicit"), &store, "deepseek", "PATH").expect("explicit"),
        "explicit"
    );

    store.set("deepseek", "stored").expect("store key");
    assert_eq!(
        resolve_key(None, &store, "deepseek", "PATH").expect("stored"),
        "stored"
    );

    store.remove("deepseek").expect("remove stored key");
    assert_eq!(
        resolve_key(None, &store, "deepseek", "PATH").expect("environment"),
        environment.trim()
    );

    let error = resolve_key(None, &store, "deepseek", "CODIE_AI_TEST_MISSING_KEY")
        .expect_err("missing key must fail");
    let message = error.to_string();
    assert!(message.contains("CODIE_AI_TEST_MISSING_KEY"), "{message}");
    assert!(message.contains("codie auth login deepseek"), "{message}");
}

#[test]
#[spec("PROV-033")]
fn spec_prov_033_errors_never_include_the_key() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = AuthStore::new(directory.path().join("auth.json"));
    let secret = "sk-very-secret-do-not-print";

    store.set("deepseek", secret).expect("store key");

    let error = resolve_key(None, &store, "openrouter", "CODIE_AI_TEST_MISSING_KEY")
        .expect_err("a different provider must not resolve")
        .to_string();
    assert!(!error.contains(secret), "the error leaked the key");
    assert!(error.contains("openrouter"), "{error}");
    assert!(error.contains("CODIE_AI_TEST_MISSING_KEY"), "{error}");

    let debug = format!("{store:?}");
    assert!(!debug.contains(secret));
}

#[test]
fn empty_values_are_absent() {
    let directory = tempfile::tempdir().expect("temp dir");
    let store = AuthStore::new(directory.path().join("auth.json"));

    assert!(store.set("deepseek", "   ").is_err());
    let error = resolve_key(Some("   "), &store, "deepseek", "CODIE_AI_TEST_MISSING_KEY");
    assert!(error.is_err());
}
