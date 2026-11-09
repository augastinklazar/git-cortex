use regex::Regex;
use std::sync::LazyLock;

static REDACTION_RULES: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    vec![
        // AWS Access Key ID & STS Temporary Credentials
        (Regex::new(r"\b(AKIA[0-9A-Z]{16})\b").unwrap(), "[REDACTED:AWS_KEY]"),
        (Regex::new(r"\b(ASIA[0-9A-Z]{16})\b").unwrap(), "[REDACTED:AWS_KEY]"),
        // GitHub Personal Access Token / OAuth
        (Regex::new(r"\b(gh[pousr]_[A-Za-z0-9_]{36,255})\b").unwrap(), "[REDACTED:GITHUB_TOKEN]"),
        (Regex::new(r"\b(github_pat_[A-Za-z0-9_]{82})\b").unwrap(), "[REDACTED:GITHUB_PAT]"),
        // Bearer Token
        (Regex::new(r"(?i)\bBearer\s+([A-Za-z0-9\-._~+/]{24,})\b").unwrap(), "Bearer [REDACTED:BEARER_TOKEN]"),
        // API key assignments in text / urls / env
        (Regex::new(r#"(?i)(api[_-]?key|secret|token|password|auth[_-]?token)\s*[:=]\s*['"]?([A-Za-z0-9\-._~+/]{16,})['"]?"#).unwrap(), "$1=[REDACTED:SECRET]"),
        // Private Key headers
        (Regex::new(r"-----BEGIN\s+[A-Z\s]+PRIVATE\s+KEY-----[\s\S]*?-----END\s+[A-Z\s]+PRIVATE\s+KEY-----").unwrap(), "[REDACTED:PRIVATE_KEY]"),
        // Database & service connection URLs with credentials
        (
            Regex::new(r"((?:https?|postgres|postgresql|mysql|redis|rediss|mongodb|mongodb\+srv|amqp|amqps)://)([^:\s]+):([^@\s]+)@").unwrap(),
            "${1}[REDACTED_CREDS]@",
        ),
        // Slack Token
        (Regex::new(r"\b(xox[baprs]-[0-9a-zA-Z]{10,48})\b").unwrap(), "[REDACTED:SLACK_TOKEN]"),
        // OpenAI Project Keys
        (Regex::new(r"\b(sk-proj-[A-Za-z0-9_-]{48,})\b").unwrap(), "[REDACTED:API_KEY]"),
        // Anthropic API Keys
        (Regex::new(r"\b(sk-ant-api03-[A-Za-z0-9_-]{80,})\b").unwrap(), "[REDACTED:API_KEY]"),
        // Standard OpenAI / Antigravity / Gemini Keys
        (Regex::new(r"\b(sk-[A-Za-z0-9]{20,64})\b").unwrap(), "[REDACTED:API_KEY]"),
        (Regex::new(r"\b(AIza[0-9A-Za-z-_]{35})\b").unwrap(), "[REDACTED:GOOGLE_API_KEY]"),
    ]
});

static HIGH_ENTROPY_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b([A-Za-z0-9+/=_-]{32,})\b").unwrap());

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

fn looks_like_camel_identifier(word: &str) -> bool {
    // Only alphanumeric and underscores; no base64 padding or separators (+, /, =, -)
    if !word.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        return false;
    }
    let has_lower = word.chars().any(|c| c.is_ascii_lowercase());
    let has_upper = word.chars().any(|c| c.is_ascii_uppercase());
    if !has_lower || !has_upper {
        return false;
    }

    // Count title-case word component occurrences ([A-Z][a-z]{2,})
    let bytes = word.as_bytes();
    let mut word_parts = 0;
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_uppercase() {
            let mut lower_count = 0;
            let mut j = i + 1;
            while j < bytes.len() && bytes[j].is_ascii_lowercase() {
                lower_count += 1;
                j += 1;
            }
            if lower_count >= 2 {
                word_parts += 1;
            }
            i = j;
        } else {
            i += 1;
        }
    }
    word_parts >= 2
}

fn scrub_high_entropy_words(text: &str, findings: &mut Vec<RedactionFinding>) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;

    // Scan words of length >= 32 with no whitespace using pre-compiled regex
    for mat in HIGH_ENTROPY_REGEX.find_iter(text) {
        let word = mat.as_str();
        let is_pure_hex = word.chars().all(|c| c.is_ascii_hexdigit());
        let has_lower = word.chars().any(|c| c.is_ascii_lowercase());
        let has_upper = word.chars().any(|c| c.is_ascii_uppercase());
        let has_digit = word.chars().any(|c| c.is_ascii_digit());

        // Base64-like secret with mixed case and digits, ignoring typical CamelCase symbol identifiers
        if !is_pure_hex
            && has_lower
            && has_upper
            && has_digit
            && !looks_like_camel_identifier(word)
            && shannon_entropy(word) > 3.5
        {
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
        let fake_aws = format!("{}{}", "AKIA", "IOSFODNN7EXAMPLE");
        let fake_ghp = format!("{}{}", "ghp_", "ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890");
        let text = format!("Failed with {} using {} in test", fake_aws, fake_ghp);
        let (scrubbed, findings) = scrub(&text);
        assert!(!scrubbed.contains(&fake_aws));
        assert!(!scrubbed.contains("ghp_"));
        assert!(scrubbed.contains("[REDACTED:AWS_KEY]"));
        assert!(scrubbed.contains("[REDACTED:GITHUB_TOKEN]"));
        assert_eq!(findings.len(), 2);
    }

    #[test]
    fn test_scrub_aws_sts_temporary_key() {
        let fake_sts = format!("{}{}", "ASIA", "IOSFODNN7EXAMPLE");
        let text = format!("Using temporary credentials with {} in session", fake_sts);
        let (scrubbed, findings) = scrub(&text);
        assert!(!scrubbed.contains(&fake_sts));
        assert!(scrubbed.contains("[REDACTED:AWS_KEY]"));
        assert_eq!(findings.len(), 1);
    }

    #[test]
    fn test_scrub_openai_project_and_anthropic_keys() {
        let fake_proj = format!(
            "{}{}",
            "sk-proj-", "1234567890abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ12"
        );
        let fake_ant = format!(
            "{}{}",
            "sk-ant-api03-",
            "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ12345678901234567890123456789012"
        );
        let text = format!("Keys: {} and {}", fake_proj, fake_ant);
        let (scrubbed, findings) = scrub(&text);
        assert!(!scrubbed.contains("sk-proj-"));
        assert!(!scrubbed.contains("sk-ant-api03-"));
        assert_eq!(findings.len(), 2);
        assert_eq!(findings[0].rule_type, "[REDACTED:API_KEY]");
        assert_eq!(findings[1].rule_type, "[REDACTED:API_KEY]");
    }

    #[test]
    fn test_scrub_url_creds() {
        let text = "Connecting to https://user:secret123456@db.internal:5432/db";
        let (scrubbed, _) = scrub(text);
        assert!(!scrubbed.contains("secret123456"));
        assert!(scrubbed.contains("https://[REDACTED_CREDS]@db.internal:5432/db"));
    }

    #[test]
    fn test_scrub_database_connection_uris() {
        let uris = [
            "postgres://app_user:p4ssw0rd123@postgres.db.svc:5432/production",
            "postgresql://app_user:p4ssw0rd123@postgres.db.svc:5432/production",
            "mysql://root:mypassword@mysql.internal:3306/main",
            "redis://default:cache_token_99@cache.local:6379/0",
            "mongodb://dbuser:mongoPass987@mongo.internal:27017/records",
            "mongodb+srv://dbuser:mongoPass987@cluster0.example.internal/records",
            "amqp://guest:secretMq@rabbitmq.svc:5672/vhost",
        ];

        for uri in uris {
            let (scrubbed, findings) = scrub(uri);
            assert!(
                !scrubbed.contains("p4ssw0rd123")
                    && !scrubbed.contains("mypassword")
                    && !scrubbed.contains("cache_token_99")
                    && !scrubbed.contains("mongoPass987")
                    && !scrubbed.contains("secretMq"),
                "Password should have been redacted in {}",
                uri
            );
            assert!(
                scrubbed.contains("[REDACTED_CREDS]@"),
                "Redaction marker missing in {}",
                scrubbed
            );
            assert!(!findings.is_empty());
        }
    }

    #[test]
    fn test_camel_case_identifiers_not_falsely_redacted() {
        let identifier = "OAuth2TokenRefreshV2PayloadHandler";
        let text = format!("Error handling request in {}", identifier);
        let (scrubbed, findings) = scrub(&text);
        assert_eq!(scrubbed, text);
        assert!(findings.is_empty());
    }

    #[test]
    fn test_clean_input_unchanged() {
        let text = "Simple normal error: Connection timed out in WorkerQueue.dispatch after 5000ms";
        let (scrubbed, findings) = scrub(text);
        assert_eq!(scrubbed, text);
        assert!(findings.is_empty());
    }
}
