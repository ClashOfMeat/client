mod common;

use clashofmeat_providers::claude_cowork::ClaudeCowork;

#[test]
fn claude_cowork_matches_old_collector() {
    let actual = common::run_fixture(
        &ClaudeCowork,
        "claude-cowork",
        "Library/Application Support/Claude/local-agent-mode-sessions/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa/workspace/local_session/audit.jsonl",
    );
    let expected = common::expected("claude-cowork");
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
