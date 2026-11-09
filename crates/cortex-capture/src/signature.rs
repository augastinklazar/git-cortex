use regex::Regex;
use std::sync::LazyLock;

static HEX_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"0x[0-9a-fA-F]+").unwrap());
static TIME_REGEX: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\d{1,2}:\d{2}(:\d{2})?").unwrap());

/// Extract and normalize a short, deterministic error signature line from terminal output
pub fn extract_error_signature(output: &str) -> String {
    let mut candidate_line = None;

    for line in output.lines().rev() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let lower = trimmed.to_lowercase();
        if lower.contains("error:")
            || lower.contains("fail")
            || lower.contains("assertion")
            || lower.contains("panic")
            || lower.contains("timeout")
            || lower.contains("exception")
            || lower.contains("traceback")
        {
            candidate_line = Some(trimmed);
            break;
        }
    }

    let line = candidate_line
        .or_else(|| output.lines().rev().find(|l| !l.trim().is_empty()))
        .unwrap_or("Non-zero exit code with empty output");

    normalize_signature_line(line)
}

fn normalize_signature_line(line: &str) -> String {
    let mut s = line.to_string();
    s = HEX_REGEX.replace_all(&s, "0x<HEX>").into_owned();
    s = TIME_REGEX.replace_all(&s, "<TIME>").into_owned();
    if s.len() > 300 {
        s.truncate(297);
        s.push_str("...");
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_signature() {
        let log = "running 1 test\ntest test_auth ... FAILED\nAssertionError: expected true but got false at 0xdeadbeef\nexit code 1";
        let sig = extract_error_signature(log);
        assert!(sig.contains("AssertionError"));
        assert!(sig.contains("0x<HEX>"));
    }
}
