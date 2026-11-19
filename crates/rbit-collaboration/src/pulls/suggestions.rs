//! Bounded, exact-line review suggestions; ambiguous blocks and overlapping edits fail closed.
use std::collections::BTreeMap;

pub const MAX_SUGGESTIONS: usize = 20;
pub const MAX_SUGGESTION_BYTES: usize = 65_536;
pub const MAX_BATCH_BYTES: usize = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub before: u32,
    pub after: u32,
    pub replacement: String,
    pub delete: bool,
}

impl Suggestion {
    /// One standalone fenced `suggestion` or `suggestion:-N+N` block per comment.
    pub fn parse(body: &str) -> Result<Option<Self>, &'static str> {
        if body.len() > MAX_SUGGESTION_BYTES {
            return Err("suggestion is too large");
        }
        let mut lines = body.lines();
        let mut result = None;
        let mut other_fence = None;
        while let Some(line) = lines.next() {
            if let Some((marker, width)) = other_fence {
                if fence(line).is_some_and(|(current, count)| current == marker && count >= width && line[count..].trim().is_empty()) {
                    other_fence = None;
                }
                continue;
            }
            let Some(suffix) = line.strip_prefix("```suggestion") else {
                other_fence = fence(line);
                continue;
            };
            if result.is_some() {
                return Err("use one suggestion block per comment");
            }
            let (before, after) = if suffix.is_empty() {
                (0, 0)
            } else {
                let (before, after) = suffix.strip_prefix(":-").and_then(|s| s.split_once('+')).ok_or("invalid suggestion range")?;
                let parse = |s: &str| s.parse::<u32>().ok().filter(|n| *n <= 100).ok_or("suggestion range exceeds 100 lines");
                (parse(before)?, parse(after)?)
            };
            let mut replacement = Vec::new();
            let mut closed = false;
            for line in lines.by_ref() {
                if line == "```" {
                    closed = true;
                    break;
                }
                replacement.push(line);
            }
            if !closed {
                return Err("close the suggestion block");
            }
            result = Some(Self { before, after, delete: replacement.is_empty(), replacement: replacement.join("\n") });
        }
        Ok(result)
    }
}

fn fence(line: &str) -> Option<(u8, usize)> {
    let marker = *line.as_bytes().first()?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let count = line.bytes().take_while(|byte| *byte == marker).count();
    (count >= 3).then_some((marker, count))
}

/// Apply nonoverlapping line ranges to exact reviewed UTF-8 bytes, preserving untouched bytes.
pub fn apply(source: &str, suggestions: &[(u32, Suggestion)]) -> Result<Vec<u8>, &'static str> {
    if source.len() > MAX_BATCH_BYTES || suggestions.is_empty() || suggestions.len() > MAX_SUGGESTIONS {
        return Err("suggestion batch exceeds its limit");
    }
    if source.contains('\0') {
        return Err("suggestions require text files");
    }
    let lines: Vec<&str> = source.split_inclusive('\n').collect();
    let mut edits = BTreeMap::new();
    for (line, suggestion) in suggestions {
        let start = line.checked_sub(suggestion.before).filter(|n| *n > 0).ok_or("suggestion starts before the file")?;
        let end = line.checked_add(suggestion.after).ok_or("invalid suggestion range")?;
        if end as usize > lines.len() || edits.insert(start as usize - 1, (end as usize, suggestion)).is_some() {
            return Err("suggestion range is outside the file or overlaps another suggestion");
        }
    }
    let mut output = String::new();
    let mut position = 0;
    for (start, (end, suggestion)) in edits {
        if start < position {
            return Err("suggestion ranges overlap");
        }
        for line in &lines[position..start] {
            output.push_str(line);
        }
        let ending = if lines[start..end].iter().any(|line| line.ends_with("\r\n")) { "\r\n" } else { "\n" };
        output.push_str(&suggestion.replacement.replace('\n', ending));
        if !suggestion.delete && lines[end - 1].ends_with('\n') {
            output.push_str(ending);
        }
        position = end;
        if output.len() > MAX_BATCH_BYTES {
            return Err("suggestion output exceeds its limit");
        }
    }
    for line in &lines[position..] {
        output.push_str(line);
    }
    if output.len() > MAX_BATCH_BYTES {
        return Err("suggestion output exceeds its limit");
    }
    Ok(output.into_bytes())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn suggestions_parse_ranges_and_reject_ambiguity() {
        let suggestion = Suggestion::parse("Please use:\n```suggestion:-1+2\na\nb\n```\nThanks").unwrap().unwrap();
        assert_eq!((suggestion.before, suggestion.after, suggestion.replacement.as_str()), (1, 2, "a\nb"));
        assert!(Suggestion::parse("```suggestion\na\n```\n```suggestion\nb\n```").is_err());
        assert!(Suggestion::parse("```suggestion:-101+0\na\n```").is_err());
        assert!(Suggestion::parse("```suggestion\na").is_err());
        assert!(Suggestion::parse("```rust\n```suggestion\n```").unwrap().is_none());
        assert!(Suggestion::parse("````markdown\n```\n```suggestion\nexample\n```\n````").unwrap().is_none());
        assert!(Suggestion::parse("~~~markdown\n```suggestion\nexample\n```\n~~~").unwrap().is_none());
    }
    #[test]
    fn suggestions_preserve_bytes_and_reject_overlap() {
        let s = |text: &str| Suggestion { before: 0, after: 0, replacement: text.into(), delete: text.is_empty() };
        assert_eq!(apply("a\r\nb\r\nc", &[(2, s("B\nBB"))]).unwrap(), b"a\r\nB\r\nBB\r\nc");
        assert_eq!(apply("a\nb", &[(2, s("B"))]).unwrap(), b"a\nB");
        assert_eq!(apply("a\nb\n", &[(2, s(""))]).unwrap(), b"a\n");
        assert!(apply("a\nb\n", &[(2, s("x")), (2, s("y"))]).is_err());
        assert!(apply("a", &[(0, s("x"))]).is_err());
        assert!(apply("a", &[(2, s("x"))]).is_err());
    }
}

#[cfg(test)]
mod edge_tests {
    use super::*;
    #[test]
    fn suggestion_empty_block_deletes_but_blank_line_is_preserved() {
        let deletion = Suggestion::parse("```suggestion\n```").unwrap().unwrap();
        let blank = Suggestion::parse("```suggestion\n\n```").unwrap().unwrap();
        assert_eq!(apply("a\nb\n", &[(1, deletion)]).unwrap(), b"b\n");
        assert_eq!(apply("a\nb\n", &[(1, blank)]).unwrap(), b"\nb\n");
    }
}
