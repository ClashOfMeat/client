mod common;

use clashofmeat_providers::codex::Codex;

#[test]
fn codex_matches_old_collector() {
    let actual = common::run_fixture(
        &Codex,
        "codex",
        ".codex/sessions/2026/10/01/rollout-main.jsonl",
    );
    let expected = common::expected("codex");
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
