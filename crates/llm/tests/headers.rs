//! Pure-function tests for `build_header_map`: the built-in header set and the
//! custom-header override semantics (custom wins over built-in by name).
use opencoder_llm::build_header_map;

#[test]
fn built_in_headers_are_present() {
    let map = build_header_map("sk-secret", &[]);
    assert_eq!(map.get("authorization").unwrap().to_str().unwrap(), "Bearer sk-secret");
    assert_eq!(map.get("content-type").unwrap().to_str().unwrap(), "application/json");
    assert_eq!(map.get("accept").unwrap().to_str().unwrap(), "text/event-stream");
}

#[test]
fn empty_custom_keeps_only_three_built_ins() {
    let map = build_header_map("k", &[]);
    assert_eq!(map.len(), 3);
    assert!(map.contains_key("authorization"));
    assert!(map.contains_key("content-type"));
    assert!(map.contains_key("accept"));
}

#[test]
fn custom_headers_are_appended() {
    let custom = vec![
        ("X-Foo".to_string(), "bar".to_string()),
        ("X-Baz".to_string(), "qux".to_string()),
    ];
    let map = build_header_map("k", &custom);
    assert_eq!(map.get("x-foo").unwrap().to_str().unwrap(), "bar");
    assert_eq!(map.get("x-baz").unwrap().to_str().unwrap(), "qux");
    // built-ins still present
    assert_eq!(map.get("authorization").unwrap().to_str().unwrap(), "Bearer k");
}

#[test]
fn custom_header_overrides_built_in_by_name() {
    // A custom "accept" must replace the built-in text/event-stream.
    let custom = vec![("accept".to_string(), "application/x-ndjson".to_string())];
    let map = build_header_map("k", &custom);
    assert_eq!(map.get("accept").unwrap().to_str().unwrap(), "application/x-ndjson");
    assert_eq!(map.len(), 3);

    // authorization can also be overridden (e.g. non-Bearer schemes).
    let custom = vec![("authorization".to_string(), "Key k-xyz".to_string())];
    let map = build_header_map("ignored", &custom);
    assert_eq!(map.get("authorization").unwrap().to_str().unwrap(), "Key k-xyz");
}

#[test]
fn custom_override_is_case_insensitive() {
    let custom = vec![("Content-Type".to_string(), "application/x-custom".to_string())];
    let map = build_header_map("k", &custom);
    assert_eq!(map.get("content-type").unwrap().to_str().unwrap(), "application/x-custom");
}

#[test]
fn malformed_custom_entries_are_skipped() {
    // Invalid header name (contains a space) is skipped; the valid one survives.
    let custom = vec![
        ("Bad Name".to_string(), "v".to_string()),
        ("X-Ok".to_string(), "good".to_string()),
    ];
    let map = build_header_map("k", &custom);
    assert!(map.get("bad name").is_none());
    assert_eq!(map.get("x-ok").unwrap().to_str().unwrap(), "good");
}
