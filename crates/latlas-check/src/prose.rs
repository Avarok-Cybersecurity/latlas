// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: Prose references (check part §1 R, arXiv 2608.03734: dangling prose references
//! are the common failure, not dangling edges). Three kinds are linted in every text of the
//! bundle:
//! - a backticked typed id (`` `ledger:l2-pf` ``) whose prefix is a known label: resolves when the
//!   node is defined;
//! - an arXiv id (`2109.06049`, new scheme): resolves when its format is valid (month 01–12);
//! - a design file name (`DESIGN-x.md`): resolves when a source of the bundle has that name.
//!
//! Owner: latlas-check.
//! Invariants: pure over the bundle; a backticked span that is not a typed id is not a reference
//! (symbols are linted once the host supplies a symbol index).

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use latlas_core::Bundle;
use latlas_core::id::{NodeId, Vocab, core_labels, label_by_prefix};

/// 2026-10-24: One prose reference.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProseRef {
    pub referrer: NodeId,
    pub text: String,
    pub resolves: bool,
}

fn typed_id<V: Vocab>(b: &Bundle, span: &str) -> Option<bool> {
    let (p, k) = span.split_once(':')?;
    let label = label_by_prefix::<V>(p)?;
    if label == core_labels::Unresolved || k.is_empty() || k.contains(char::is_whitespace) {
        return None;
    }
    let id = NodeId::new(label, k).ok()?;
    Some(b.node(&id).is_some())
}

/// 2026-10-24: `(start, token)` of every arXiv-shaped id (`DDDD.DDDD` or `DDDD.DDDDD`, not part of a
/// longer number).
fn arxiv_ids(t: &str) -> Vec<&str> {
    let b = t.as_bytes();
    let mut out = Vec::new();
    let digit = |i: usize| b.get(i).is_some_and(u8::is_ascii_digit);
    let mut i = 0;
    while i + 9 <= b.len() {
        let starts = (i == 0 || !(digit(i - 1) || b[i - 1] == b'.')) && (0..4).all(|k| digit(i + k)) && b[i + 4] == b'.';
        if starts {
            let n = (5..).take_while(|&k| digit(i + k)).count();
            let after = i + 5 + n;
            if (n == 4 || n == 5) && !(b.get(after) == Some(&b'.') && digit(after + 1)) {
                out.push(&t[i..after]);
                i = after;
                continue;
            }
        }
        i += 1;
    }
    out
}

fn arxiv_valid(id: &str) -> bool {
    id.get(2..4)
        .and_then(|m| m.parse::<u32>().ok())
        .is_some_and(|m| (1..=12).contains(&m))
}

fn design_names(t: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = t;
    while let Some(i) = rest.find("DESIGN-") {
        let tail = &rest[i..];
        let end = tail
            .find(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
            .unwrap_or(tail.len());
        let tok = tail[..end].trim_end_matches('.');
        if tok.ends_with(".md") {
            out.push(tok);
        }
        rest = &tail[end.max(1)..];
    }
    out
}

/// 2026-10-24: Every prose reference in the bundle's texts.
pub fn prose_refs<V: Vocab>(b: &Bundle) -> Vec<ProseRef> {
    let names: Vec<&str> = b
        .sources
        .iter()
        .map(|s| s.path.rsplit('/').next().unwrap_or(&s.path))
        .collect();
    let mut out = Vec::new();
    for t in &b.texts {
        let mut push = |text: &str, resolves: bool| {
            out.push(ProseRef {
                referrer: t.node.clone(),
                text: text.to_string(),
                resolves,
            });
        };
        for (i, span) in t.text.split('`').enumerate() {
            if i % 2 == 1
                && let Some(ok) = typed_id::<V>(b, span)
            {
                push(span, ok);
            }
        }
        for a in arxiv_ids(&t.text) {
            push(a, arxiv_valid(a));
        }
        for d in design_names(&t.text) {
            push(d, names.contains(&d));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn arxiv_ids_are_found_whole_and_their_month_checked() {
        assert_eq!(arxiv_ids("see 2109.06049 and 2608.03734."), ["2109.06049", "2608.03734"]);
        assert!(arxiv_ids("7438.35 s and 1.2889 ms and 12345.67890").is_empty());
        assert!(arxiv_valid("2109.06049"));
        assert!(!arxiv_valid("2113.06049"));
    }

    #[test]
    fn design_names_are_tokens_ending_in_md() {
        assert_eq!(design_names("per DESIGN-latlas-check.md §3, not DESIGN-x"), ["DESIGN-latlas-check.md"]);
    }
}
