use regex::Regex;
use std::sync::LazyLock;

static REDACTION_RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        // AWS Access Key ID
        (Regex::new(r"\b(AKIA[0-9A-Z]{16})\b").unwrap(), "[REDACTED:AWS_KEY]"),
        // GitHub Personal Access Token / OAuth
        (Regex::new(r"\b(gh[pousr]_[A-Za-z0-9_]{36,255})\b").unwrap(), "[REDACTED:GITHUB_TOKEN]"),
        (Regex::new(r"\b(github_pat_[A-Za-z0-9_]{82})\b").unwrap(), "[REDACTED:GITHUB_PAT]"),
        // Bearer Token
        (Regex::new(r"(?i)\bBearer\s+([A-Za-z0-9\-._~+/]{24,})\b").unwrap(), "Bearer [REDACTED:BEARER_TOKEN]"),
        // API key assignments in text / urls / env
        (Regex::new(r#"(?i)(api[_-]?key|secret|token|password|auth[_-]?token)\s*[:=]\s*['"]?([A-Za-z0-9\-._~+/]{16,})['"]?"#).unwrap(), "$1=[REDACTED:SECRET]"),
        // Private Key headers
        (Regex::new(r"-----BEGIN\s+[A-Z\s]+PRIVATE\s+KEY-----[\s\S]*?-----END\s+[A-Z\s]+PRIVATE\s+KEY-----").unwrap(), "[REDACTED:PRIVATE_KEY]"),
        // URL with user:pass
        (Regex::new(r"(https?://)([^:\s]+):([^@\s]+)@").unwrap(), "${1}[REDACTED_CREDS]@"),
        // Slack Token
        (Regex::new(r"\b(xox[baprs]-[0-9a-zA-Z]{10,48})\b").unwrap(), "[REDACTED:SLACK_TOKEN]"),
        // OpenAI / Antigravity / Gemini Keys
        (Regex::new(r"\b(sk-[A-Za-z0-9]{20,64})\b").unwrap(), "[REDACTED:API_KEY]"),
        (Regex::new(r"\b(AIza[0-9A-Za-z-_]{35})\b").unwrap(), "[REDACTED:GOOGLE_API_KEY]"),
    ]
});

/// Redaction finding
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RedactionFinding {
    pub rule_type: String,
}

/// Scrub text of known credentials and secrets
pub fn scrub(input: &str) -> (String, Vec<RedactionFinding>) {
    let mut scrubbed = input.to_string();
    let mut findings = Vec::new();

    for (regex, replacement) in REDACTION_RULES.iter() {
        if regex.is_match(&scrubbed) {
            findings.push(RedactionFinding {
                rule_type: replacement.to_string(),
            });
            scrubbed = regex.replace_all(&scrubbed, *replacement).into_owned();
        }
    }

    // High-entropy token scrubber for long base64/hex blobs >= 32 chars
    scrubbed = scrub_high_entropy_words(&scrubbed, &mut findings);

    (scrubbed, findings)
}

fn scrub_high_entropy_words(text: &str, findings: &mut Vec<RedactionFinding>) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;

    // Scan words of length >= 32 with no whitespace
    for mat in Regex::new(r"\b([A-Za-z0-9+/=_-]{32,})\b")
        .unwrap()
        .find_iter(text)
    {
        let word = mat.as_str();
        let is_pure_hex = word.chars().all(|c| c.is_ascii_hexdigit());
        let has_lower = word.chars().any(|c| c.is_ascii_lowercase());
        let has_upper = word.chars().any(|c| c.is_ascii_uppercase());
        let has_digit = word.chars().any(|c| c.is_ascii_digit());

        // Base64-like secret with mixed case and digits
        if !is_pure_hex && has_lower && has_upper && has_digit && shannon_entropy(word) > 3.5 {
            result.push_str(&text[last_end..mat.start()]);
            result.push_str("[REDACTED:HIGH_ENTROPY]");
            findings.push(RedactionFinding {
                rule_type: "[REDACTED:HIGH_ENTROPY]".to_string(),
            });
            last_end = mat.end();
        }
    }

    result.push_str(&text[last_end..]);
    result
}

fn shannon_entropy(s: &str) -> f64 {
    if s.is_empty() {
        return 0.0;
    }
    let mut counts = [0usize; 256];
    for &b in s.as_bytes() {
        counts[b as usize] += 1;
    }
    let len = s.len() as f64;
    let mut entropy = 0.0;
    for &count in counts.iter() {
        if count > 0 {
            let p = count as f64 / len;
            entropy -= p * p.log2();
        }
    }
    entropy
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scrub_aws_and_github() {
        let text = "Failed with AKIAIOSFODNN7EXAMPLE using ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890 in test";
        let (scrubbed, findings) = scrub(text);
        assert!(!scrubbed.contains("AKIAIOSFODNN7EXAMPLE"));
        assert!(!scrubbed.contains("ghp_"));
        assert!(scrubbed.contains("[REDACTED:AWS_KEY]"));
        assert!(scrubbed.contains("[REDACTED:GITHUB_TOKEN]"));
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn test_scrub_url_creds() {
        let text = "Connecting to https://user:secret123456@db.internal:5432/db";
        let (scrubbed, _) = scrub(text);
        assert!(!scrubbed.contains("secret123456"));
        assert!(scrubbed.contains("https://[REDACTED_CREDS]@db.internal:5432/db"));
    }

    #[test]
    fn test_clean_input_unchanged() {
        let text = "Simple normal error: Connection timed out in WorkerQueue.dispatch after 5000ms";
        let (scrubbed, findings) = scrub(text);
        assert_eq!(scrubbed, text);
        assert!(findings.is_empty());
    }
}
