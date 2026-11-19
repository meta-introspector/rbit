//! High-confidence text detectors; matched credential bytes never enter a finding message.
use crate::security::{Finding, MAX_RESULTS};

/// The caller supplies a repository-keyed digest, preventing token disclosure in stored identities.
pub fn detect(path: &str, text: &str, digest: impl Fn(&str, &[u8]) -> String) -> (Vec<Finding>, bool) {
    let mut findings = Vec::new();
    let mut byte_offset = 0;
    for (index, raw) in text.split_inclusive('\n').enumerate() {
        let line = raw.trim_end_matches(['\n', '\r']);
        let line_number = u32::try_from(index + 1).unwrap_or(u32::MAX);
        for (rule, prefix, length) in [("github-classic-token", "ghp_", 40), ("github-fine-grained-token", "github_pat_", 93)] {
            for (offset, _) in line.match_indices(prefix) {
                let token: String =
                    line[offset..].chars().take(length + 1).take_while(|c| c.is_ascii_alphanumeric() || *c == '_').collect();
                if token.len() == length {
                    push(&mut findings, path, line_number, rule, &token, &digest);
                    if findings.len() >= MAX_RESULTS {
                        return (findings, false);
                    }
                }
            }
        }
        for marker in [
            "-----BEGIN PRIVATE KEY-----",
            "-----BEGIN RSA PRIVATE KEY-----",
            "-----BEGIN EC PRIVATE KEY-----",
            "-----BEGIN OPENSSH PRIVATE KEY-----",
        ] {
            if line.trim() == marker {
                let offset = line.find(marker).unwrap_or(0);
                // Bind the identity to bounded key material, not merely the common armor header.
                let start = byte_offset + offset;
                let material = text.get(start..).unwrap_or(line);
                let mut bound = material.len().min(16_384);
                while !material.is_char_boundary(bound) {
                    bound -= 1;
                }
                let material = &material[..bound];
                let end = material.find("-----END ").unwrap_or(material.len());
                push(&mut findings, path, line_number, "private-key", &material[..end], &digest);
                if findings.len() >= MAX_RESULTS {
                    return (findings, false);
                }
            }
        }
        byte_offset += raw.len();
        if findings.len() >= MAX_RESULTS {
            findings.truncate(MAX_RESULTS);
            return (findings, false);
        }
    }
    (findings, true)
}
fn push(findings: &mut Vec<Finding>, path: &str, line: u32, rule: &str, token: &str, digest: &impl Fn(&str, &[u8]) -> String) {
    let identity = digest(&format!("{rule}\0{path}\0"), token.as_bytes());
    if findings.iter().any(|f| f.identity == identity) {
        return;
    }
    findings.push(Finding { identity, rule: rule.into(), message: "Potential credential detected. Revoke or rotate it with its provider; removing the file does not revoke it. Credential contents are never stored in this finding.".into(), path: path.into(), line, severity: 4 });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redacts_tokens_and_preserves_identity_when_lines_move() {
        let token = format!("ghp_{}", "x".repeat(36));
        let digest = |scope: &str, bytes: &[u8]| format!("{scope}:{}", bytes.len());
        let (a, complete) = detect("config", &token, digest);
        let (b, _) = detect("config", &format!("\n{token}"), digest);
        assert!(complete);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0].identity, b[0].identity);
        assert_eq!(b[0].line, 2);
        assert!(!a[0].message.contains(&token));
        assert_eq!(detect("config", "ghp_short", digest).0.len(), 0);
    }
    #[test]
    fn a_single_line_stops_at_the_finding_budget() {
        use std::fmt::Write as _;
        let mut text = String::new();
        for n in 0..=MAX_RESULTS {
            write!(text, "ghp_{n:036} ").unwrap();
        }
        let digests = std::cell::Cell::new(0);
        let (findings, complete) = detect("config", &text, |_, _| {
            let count = digests.get() + 1;
            digests.set(count);
            count.to_string()
        });
        assert!(!complete);
        assert_eq!(findings.len(), MAX_RESULTS);
        assert_eq!(digests.get(), MAX_RESULTS, "do not scan the rest of a hostile long line after exhausting the budget");
    }
}
