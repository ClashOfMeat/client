mod common;

use clashofmeat_providers::gemini::Gemini;

#[test]
fn gemini_matches_old_collector() {
    let actual = common::run_fixture(
        &Gemini,
        "gemini",
        ".gemini/tmp/project-hash/chats/session-2026-10-01-main.jsonl",
    );
    let expected = common::expected("gemini");
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
