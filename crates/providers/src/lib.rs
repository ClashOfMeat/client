pub mod claude_code;
pub mod claude_cowork;
pub mod codex;
pub mod cursor;
pub mod gemini;

use clashofmeat_core::Provider;

/// Providers compiled into the client.
pub fn all() -> Vec<Box<dyn Provider>> {
    vec![
        Box::new(claude_code::ClaudeCode),
        Box::new(claude_cowork::ClaudeCowork),
        Box::new(codex::Codex),
        Box::new(gemini::Gemini),
        Box::new(cursor::Cursor),
    ]
}
