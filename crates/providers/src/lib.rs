pub mod claude_code;

use clashofmeat_core::Provider;

/// Providers compiled into the client.
pub fn all() -> Vec<Box<dyn Provider>> {
    vec![Box::new(claude_code::ClaudeCode)]
}
