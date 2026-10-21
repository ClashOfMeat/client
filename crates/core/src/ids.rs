use sha2::{Digest, Sha256};

/// Largest observation value accepted by the server.
pub const MAX_OBSERVATION_VALUE: u64 = 1_000_000_000;

fn seeded_id(seed: &str, value: &str) -> String {
    let hash = hex::encode(Sha256::digest(format!("{seed}{value}").as_bytes()));
    format!(
        "{}-{}-5{}-a{}-{}",
        &hash[..8],
        &hash[8..12],
        &hash[13..16],
        &hash[17..20],
        &hash[20..32]
    )
}

/// Returns the frozen event identifier used by the JavaScript client.
pub fn stable_id(value: &str) -> String {
    seeded_id("guildbyte-v1:", value)
}

/// Returns the frozen observer task identifier.
pub fn observer_task_id(provider: &str, task_key: &str) -> String {
    seeded_id("guildbyte-observer-v1:", &format!("{provider}:{task_key}"))
}

/// Preserves a UUID session identifier or derives one from a provider session key.
pub fn session_uuid(provider: &str, value: &str) -> String {
    if is_uuid(value) {
        value.to_owned()
    } else {
        stable_id(&format!("{provider}-session:{value}"))
    }
}

/// A fresh random version 4 UUID, for request and session identifiers. Seeded from the
/// standard library's per-process random keys, the clock and a counter (no extra crate).
pub fn random_uuid() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
    hasher.write_u64(NEXT.fetch_add(1, Ordering::Relaxed));
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |at| at.as_nanos());
    let seed = format!("{}:{nanos}:{}", hasher.finish(), std::process::id());
    let hash = hex::encode(Sha256::digest(seed.as_bytes()));
    format!(
        "{}-{}-4{}-a{}-{}",
        &hash[..8],
        &hash[8..12],
        &hash[13..16],
        &hash[17..20],
        &hash[20..32]
    )
}

/// Matches the JavaScript client's case-insensitive UUID shape check.
pub fn is_uuid(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| {
            if matches!(index, 8 | 13 | 18 | 23) {
                *byte == b'-'
            } else {
                byte.is_ascii_hexdigit()
            }
        })
}

/// Returns the first matching Claude model family, or "unknown".
pub fn model_family(id: Option<&str>) -> &'static str {
    let Some(id) = id else { return "unknown" };
    let id = id.to_ascii_lowercase();
    for (needle, family) in [
        ("fable", "fable"),
        ("opus", "opus"),
        ("sonnet", "sonnet"),
        ("haiku", "haiku"),
    ] {
        if id.contains(needle) {
            return family;
        }
    }
    "unknown"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn random_uuid_is_a_fresh_uuid() {
        let (a, b) = (random_uuid(), random_uuid());
        assert!(is_uuid(&a) && is_uuid(&b));
        assert_eq!(&a[14..15], "4");
        assert_ne!(a, b);
    }

    #[test]
    fn stable_id_matches_js() {
        for (value, expected) in [
            ("", "dce46d81-1555-5e95-a250-f54c8a4605a4"),
            ("hello", "39d47007-b087-5a52-a6f3-aaa81ecf80c5"),
            (
                "request:abc:input_tokens",
                "8a4fccc0-1f6b-5917-a546-0b55554d2ea3",
            ),
            (
                "session:123e4567-e89b-12d3-a456-426614174000:session_count",
                "8f602347-75c9-5464-aeb2-db08d3fd670e",
            ),
            (
                "claude-code:response-1:output_tokens",
                "27571ebb-5d1e-59a6-a2de-ba3acb760a5e",
            ),
            ("unicode:é:🧍", "687399b3-8ff4-5922-aa4e-2fa919c475ff"),
        ] {
            assert_eq!(stable_id(value), expected, "{value}");
        }
    }

    #[test]
    fn observer_task_id_matches_js() {
        for (provider, key, expected) in [
            ("claude", "task-1", "622d91a2-f5dd-5b44-aa41-b6db3f279133"),
            (
                "codex",
                "session:abc",
                "338c341f-a731-5368-af68-64431a76183f",
            ),
            ("gemini", "", "2e0aebe2-a8e5-50c5-a784-2a7ba32a9991"),
            (
                "cursor",
                "generation:42",
                "d02dd9b8-8089-50c2-acf6-f2ea6fac72e6",
            ),
            (
                "claude-cowork",
                "é/🧍",
                "0730e7b5-6055-5b8a-a52d-ebcb1b4e0f84",
            ),
        ] {
            assert_eq!(
                observer_task_id(provider, key),
                expected,
                "{provider}:{key}"
            );
        }
    }

    #[test]
    fn session_uuid_matches_js() {
        for (provider, value, expected) in [
            (
                "claude",
                "123e4567-e89b-12d3-a456-426614174000",
                "123e4567-e89b-12d3-a456-426614174000",
            ),
            (
                "codex",
                "thread-123",
                "1249534e-38f9-5670-a44e-56440047f3eb",
            ),
            ("gemini", "", "8cb70a2a-2129-5d21-a031-7381cf99f3be"),
            (
                "cursor",
                "ABCDEFAB-CDEF-ABCD-EFAB-CDEFABCDEFAB",
                "ABCDEFAB-CDEF-ABCD-EFAB-CDEFABCDEFAB",
            ),
            (
                "claude-cowork",
                "é/🧍",
                "9c5e9b54-ae42-5278-a209-9d453a9042e5",
            ),
        ] {
            assert_eq!(
                session_uuid(provider, value),
                expected,
                "{provider}:{value}"
            );
        }
    }

    #[test]
    fn is_uuid_matches_js() {
        for (value, expected) in [
            ("123e4567-e89b-12d3-a456-426614174000", true),
            ("ABCDEFAB-CDEF-ABCD-EFAB-CDEFABCDEFAB", true),
            ("123e4567e89b12d3a456426614174000", false),
            ("g23e4567-e89b-12d3-a456-426614174000", false),
            ("00000000-0000-0000-0000-000000000000", true),
        ] {
            assert_eq!(is_uuid(value), expected, "{value}");
        }
    }

    #[test]
    fn model_family_matches_js() {
        for (value, expected) in [
            (None, "unknown"),
            (Some("claude-opus-4"), "opus"),
            (Some("claude-sonnet-4"), "sonnet"),
            (Some("claude-haiku-3"), "haiku"),
            (Some("claude-fable-1"), "fable"),
            (Some("fable-opus-sonnet"), "fable"),
            (Some("gpt-5"), "unknown"),
        ] {
            assert_eq!(model_family(value), expected, "{value:?}");
        }
        assert_eq!(MAX_OBSERVATION_VALUE, 1_000_000_000);
    }
}
