use crate::schema::{InvariantContract, TombstoneRecord};

pub const MAX_APPROACH_LEN: usize = 600;
pub const MAX_REASON_LEN: usize = 800;
pub const MAX_SIGNATURE_LEN: usize = 300;

/// Sanitize text: strip ANSI escape sequences, Bidi override codepoints, and null bytes
pub fn sanitize_text(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();

    while let Some(c) = chars.next() {
        // Strip ANSI escape sequences
        if c == '\x1B' {
            if let Some(&'[') = chars.peek() {
                chars.next();
                // Consume until terminal letter
                for next_c in chars.by_ref() {
                    if next_c.is_ascii_alphabetic() || next_c == 'm' {
                        break;
                    }
                }
                continue;
            }
        }

        // Strip dangerous unicode bidi controls and nulls
        match c {
            '\0' | '\r' => continue,
            // Bidi override characters
            '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' => continue,
            _ => out.push(c),
        }
    }

    out
}

/// Truncate text cleanly with ellipsis if exceeding max length
pub fn truncate_with_ellipsis(text: &str, max_len: usize) -> String {
    if text.chars().count() <= max_len {
        text.to_string()
    } else {
        let truncated: String = text.chars().take(max_len.saturating_sub(3)).collect();
        format!("{}...", truncated)
    }
}

/// Provenance origin of epistemic data
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Provenance {
    LocalWorkingTree,
    LocalCheckpoint,
    LedgerShared,
    RemoteUntrusted,
}

impl Provenance {
    pub fn as_banner(&self) -> &'static str {
        match self {
            Self::LocalWorkingTree => "LOCAL_WORKING_TREE",
            Self::LocalCheckpoint => "LOCAL_CHECKPOINT",
            Self::LedgerShared => "LEDGER_COMMITTED",
            Self::RemoteUntrusted => "REMOTE_UNTRUSTED",
        }
    }
}

/// Formatter that fences a TombstoneRecord into safe markdown for LLM ingestion
pub fn fence_tombstone(record: &TombstoneRecord, provenance: Provenance) -> String {
    let clean_approach = truncate_with_ellipsis(
        &sanitize_text(&record.attempted_approach),
        MAX_APPROACH_LEN,
    );
    let clean_reason = truncate_with_ellipsis(
        &sanitize_text(&record.failure_reason),
        MAX_REASON_LEN,
    );
    let clean_sig = truncate_with_ellipsis(
        &sanitize_text(&record.verification_evidence.error_signature),
        MAX_SIGNATURE_LEN,
    );

    format!(
        "<!-- CORTEX_TOMBSTONE_BEGIN id=\"{}\" provenance=\"{}\" harness=\"{}\" -->\n\
         [TOMBSTONE: {}]\n\
         - Approach Attempted: {}\n\
         - Why It Failed: {}\n\
         - Error Signature: {}\n\
         <!-- CORTEX_TOMBSTONE_END id=\"{}\" -->",
        record.id,
        provenance.as_banner(),
        record.author_harness,
        record.id,
        clean_approach,
        clean_reason,
        clean_sig,
        record.id
    )
}

/// Formatter that fences an InvariantContract into safe markdown for LLM inspection
pub fn fence_invariant(contract: &InvariantContract, provenance: Provenance) -> String {
    let clean_assertion = truncate_with_ellipsis(
        &sanitize_text(&contract.assertion),
        MAX_REASON_LEN,
    );

    format!(
        "<!-- CORTEX_INVARIANT_BEGIN id=\"{}\" severity=\"{:?}\" provenance=\"{}\" -->\n\
         [INVARIANT LOCK: {}] (Severity: {:?})\n\
         - Guarded Symbol: {}\n\
         - Architectural Assertion: {}\n\
         - Note: Verification test `{}` must pass before modifying dependencies.\n\
         <!-- CORTEX_INVARIANT_END id=\"{}\" -->",
        contract.invariant_id,
        contract.severity,
        provenance.as_banner(),
        contract.invariant_id,
        contract.severity,
        contract.guarded_symbol.symbol_uri,
        clean_assertion,
        contract.verification_test_cmd,
        contract.invariant_id
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::*;

    #[test]
    fn test_sanitize_bidi_and_ansi() {
        let dirty = "Normal \x1B[31mRed text\x1B[0m with \u{202E}reversed\u{202C} chars";
        let clean = sanitize_text(dirty);
        assert!(!clean.contains("\x1B"));
        assert!(!clean.contains("\u{202E}"));
        assert!(clean.contains("Red text"));
        assert!(clean.contains("reversed"));
    }

    #[test]
    fn test_fence_tombstone_format() {
        let t = TombstoneRecord::new(
            "tmb_01".into(),
            "claude-code".into(),
            "main".into(),
            "abc1234".into(),
            CaptureMode::Manual,
            vec![],
            "Tried mutex".into(),
            "Deadlock on callback".into(),
            VerificationEvidence {
                command: "npm test".into(),
                exit_code: 1,
                error_signature: "ERR_TIMEOUT".into(),
            },
            vec![],
        );

        let fenced = fence_tombstone(&t, Provenance::LocalWorkingTree);
        assert!(fenced.contains("<!-- CORTEX_TOMBSTONE_BEGIN"));
        assert!(fenced.contains("Tried mutex"));
        assert!(fenced.contains("Deadlock on callback"));
    }
}
