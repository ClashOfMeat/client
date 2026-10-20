mod common;

use clashofmeat_providers::cursor::Cursor;

#[test]
fn cursor_matches_old_collector() {
    let actual = common::run_fixture(&Cursor, "cursor", ".clashofmeat/spool/cursor.jsonl");
    let expected = common::expected("cursor");
    if actual != expected {
        let first = actual
            .iter()
            .zip(&expected)
            .position(|(a, e)| a != e)
            .unwrap_or(actual.len().min(expected.len()));
        panic!(
            "first differing row {first} (actual {}, expected {}):\nactual: {}\nexpected: {}",
            actual.len(),
            expected.len(),
            actual
                .get(first)
                .map_or_else(|| "<missing>".to_owned(), ToString::to_string),
            expected
                .get(first)
                .map_or_else(|| "<missing>".to_owned(), ToString::to_string),
        );
    }
}
