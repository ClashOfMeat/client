mod common;

use clashofmeat_providers::claude_code::ClaudeCode;

#[test]
fn claude_code_matches_old_collector() {
    let actual = common::run_fixture(
        &ClaudeCode,
        "claude-code",
        ".claude/projects/-tmp-demo/11111111-1111-4111-8111-111111111111.jsonl",
    );
    let expected = common::expected("claude-code");
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
