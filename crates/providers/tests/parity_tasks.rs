#[allow(dead_code)]
mod common;
use clashofmeat_providers::{
    claude_code::ClaudeCode, claude_cowork::ClaudeCowork, codex::Codex, cursor::Cursor,
    gemini::Gemini,
};
use common::{expected_tasks, run_fixture_tasks};
use std::time::Duration;

#[test]
fn claude_code() {
    assert_eq!(
        run_fixture_tasks(
            &ClaudeCode,
            "claude-code",
            ".claude/projects/-tmp-demo/11111111-1111-4111-8111-111111111111.jsonl",
            Duration::from_secs(600)
        ),
        expected_tasks("claude-code")
    );
}
#[test]
fn claude_cowork() {
    assert_eq!(run_fixture_tasks(&ClaudeCowork,"claude-cowork","Library/Application Support/Claude/local-agent-mode-sessions/aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa/workspace/local_session/audit.jsonl",Duration::from_secs(600)),expected_tasks("claude-cowork"));
}
#[test]
fn codex() {
    assert_eq!(
        run_fixture_tasks(
            &Codex,
            "codex",
            ".codex/sessions/2026/10/01/rollout-main.jsonl",
            Duration::from_secs(1800)
        ),
        expected_tasks("codex")
    );
}
#[test]
fn gemini() {
    assert_eq!(
        run_fixture_tasks(
            &Gemini,
            "gemini",
            ".gemini/tmp/project-hash/chats/session-2026-10-01-main.jsonl",
            Duration::from_secs(600)
        ),
        expected_tasks("gemini")
    );
}
#[test]
fn cursor() {
    assert_eq!(
        run_fixture_tasks(
            &Cursor,
            "cursor",
            ".clashofmeat/spool/cursor.jsonl",
            Duration::from_secs(1800)
        ),
        expected_tasks("cursor")
    );
}
