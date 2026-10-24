// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: Markdown tables: the first pipe table after a heading whose text is exactly the
//! declared one, as header cells and rows (cells trimmed, `**bold**` and backticks stripped).
//!
//! Owner: latlas-decl.
//! Invariants: a row's line number is 1-based in the file; the separator row is skipped.

use alloc::string::{String, ToString};
use alloc::vec::Vec;

/// 2026-10-24: A parsed table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub header: Vec<String>,
    /// 2026-10-24: `(line, cells)`.
    pub rows: Vec<(usize, Vec<String>)>,
}

fn clean(c: &str) -> String {
    c.trim().replace("**", "").replace('`', "").trim().to_string()
}

fn cells(line: &str) -> Vec<String> {
    let t = line.trim();
    let t = t.strip_prefix('|').unwrap_or(t);
    let t = t.strip_suffix('|').unwrap_or(t);
    t.split('|').map(clean).collect()
}

fn is_separator(line: &str) -> bool {
    let t = line.trim();
    t.starts_with('|') && t.chars().all(|c| matches!(c, '|' | '-' | ':' | ' '))
}

/// 2026-10-24: The table under `heading` (`None`: no such heading, or no table under it before
/// the next heading).
pub fn under_heading(text: &str, heading: &str) -> Option<Table> {
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.iter().position(|l| {
        let t = l.trim_start();
        t.starts_with('#') && t.trim_start_matches('#').trim() == heading
    })?;
    let mut i = start + 1;
    while i < lines.len() && !lines[i].trim_start().starts_with('|') {
        if lines[i].trim_start().starts_with('#') {
            return None;
        }
        i += 1;
    }
    let header = cells(lines.get(i)?);
    let mut rows = Vec::new();
    for (j, l) in lines.iter().enumerate().skip(i + 1) {
        if !l.trim_start().starts_with('|') {
            break;
        }
        if is_separator(l) {
            continue;
        }
        rows.push((j + 1, cells(l)));
    }
    Some(Table { header, rows })
}
