// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: What `la_record` accepts, per record kind: its columns, which are required, how
//! each optional one is filled, the grammar of the columns that have one, and an example call. A
//! pack declares one [`KindSchema`] per kind ([`crate::Pack::RECORD_SCHEMAS`]); the core turns
//! the caller's fields into cells with [`KindSchema::fill`], serves every schema as the
//! `record_schema` status section, and names the schema in every field refusal.
//!
//! Owner: latlas-core.
//! Invariants:
//! - A column absent from the fields is filled only by its declared [`Fill`] rule, and every fill
//!   is reported back (PCND: declared and visible, never silent).
//! - A field refusal names the kind's columns, which are required, and the example, so a caller
//!   never has to guess.
//! - A cell with a tab, a newline or a carriage return is refused (it would shift columns); a
//!   cell matching the secrets denylist is refused (records are committed files).
//! - Time comes from the caller's Clock-port reading (`now`), never from the core.

use crate::prelude::*;
use crate::secrets::secret_pattern;
use latlas_error::{ErrorCode, Result, error};
use latlas_ports::UnixMs;
use serde_json::{Map, Value};

/// 2026-10-24: How a column absent from the caller's fields is filled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    /// The caller passes it (an explicit `-` is a value).
    Required,
    /// This literal (`-` for a column the record does not use).
    Literal(&'static str),
    /// The Clock port's now, UTC, `YYYY-MM-DDTHH:MMZ`.
    NowUtcMinute,
    /// The Clock port's today, UTC, `YYYY-MM-DD`.
    TodayUtc,
}

impl Fill {
    /// 2026-10-24: The rule as the schema and the `filled` report state it.
    pub fn rule(self) -> String {
        match self {
            Fill::Required => "required".into(),
            Fill::Literal(v) => format!("optional, default `{v}`"),
            Fill::NowUtcMinute => {
                "optional, default now (Clock port, UTC YYYY-MM-DDTHH:MMZ)".into()
            }
            Fill::TodayUtc => "optional, default today (Clock port, UTC YYYY-MM-DD)".into(),
        }
    }

    fn value(self, now: UnixMs) -> Option<String> {
        match self {
            Fill::Required => None,
            Fill::Literal(v) => Some(v.into()),
            Fill::NowUtcMinute => Some(utc_minute(now)),
            Fill::TodayUtc => Some(utc_date(now)),
        }
    }
}

/// 2026-10-24: One column the caller may pass.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Column {
    pub name: &'static str,
    pub fill: Fill,
    /// What the cell holds, one line.
    pub doc: &'static str,
}

/// 2026-10-24: The grammar of one column, with examples that parse.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Grammar {
    pub column: &'static str,
    pub rule: &'static str,
    pub examples: &'static [&'static str],
}

/// 2026-10-24: One record kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KindSchema {
    /// The `kind` argument.
    pub kind: &'static str,
    /// Which file the record appends to, and what it is for.
    pub file: &'static str,
    /// The columns the caller may pass, in file order.
    pub columns: &'static [Column],
    /// Columns the record computes (`(name, rule)`); passing one is refused.
    pub computed: &'static [(&'static str, &'static str)],
    pub grammar: &'static [Grammar],
    /// The example's `fields` object, as JSON text (a unit test in each pack records it).
    pub example: &'static str,
}

/// 2026-10-24: The cells of one record and the fills applied to reach them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filled {
    /// One cell per column, in [`KindSchema::columns`] order.
    pub cells: Vec<String>,
    /// `<column> = <value> (<rule>)` for every column the caller left out.
    pub filled: Vec<String>,
}

/// 2026-10-24: The status section that serves every schema.
pub const SECTION: &str = "record_schema";

impl KindSchema {
    /// 2026-10-24: Turn the caller's fields into cells: unknown or computed fields and missing
    /// required ones are refused with the schema; each absent optional column is filled by its
    /// rule and reported.
    pub fn fill(
        &self,
        fields: &BTreeMap<String, String>,
        now: UnixMs,
        scope: &str,
    ) -> Result<Filled> {
        if let Some((k, rule)) = self.computed.iter().find(|(c, _)| fields.contains_key(*c)) {
            return Err(error!(
                ErrorCode::LatlasToolArgument,
                (*k).to_string(),
                format!(
                    "is computed ({rule}); do not pass it. {}",
                    self.usage(scope)
                )
            ));
        }
        if let Some(extra) = fields
            .keys()
            .find(|k| !self.columns.iter().any(|c| c.name == k.as_str()))
        {
            return Err(error!(
                ErrorCode::LatlasToolArgument,
                extra.clone(),
                format!("is not a column of {}. {}", self.kind, self.usage(scope))
            ));
        }
        let missing: Vec<&str> = self
            .columns
            .iter()
            .filter(|c| c.fill == Fill::Required && !fields.contains_key(c.name))
            .map(|c| c.name)
            .collect();
        if !missing.is_empty() {
            return Err(error!(
                ErrorCode::LatlasToolArgument,
                missing.join(", "),
                format!(
                    "required by {}, not passed. {}",
                    self.kind,
                    self.usage(scope)
                )
            ));
        }
        let mut cells = Vec::with_capacity(self.columns.len());
        let mut filled = Vec::new();
        for c in self.columns {
            let v = match fields.get(c.name) {
                Some(v) => v.clone(),
                None => {
                    let v = c.fill.value(now).unwrap_or_default();
                    filled.push(format!("{} = {v} ({})", c.name, c.fill.rule()));
                    v
                }
            };
            cell_ok(c.name, &v)?;
            cells.push(v);
        }
        Ok(Filled { cells, filled })
    }

    /// 2026-10-24: The column names in file order.
    pub fn names(&self) -> Vec<&'static str> {
        self.columns.iter().map(|c| c.name).collect()
    }

    /// 2026-10-24: The one-paragraph usage every field refusal carries.
    pub fn usage(&self, scope: &str) -> String {
        let req: Vec<&str> = self
            .columns
            .iter()
            .filter(|c| c.fill == Fill::Required)
            .map(|c| c.name)
            .collect();
        let opt: Vec<String> = self
            .columns
            .iter()
            .filter(|c| c.fill != Fill::Required)
            .map(|c| {
                format!(
                    "{} ({})",
                    c.name,
                    c.fill.rule().trim_start_matches("optional, ")
                )
            })
            .collect();
        let mut s = format!(
            "{} columns: required {}; optional {}.",
            self.kind,
            req.join(", "),
            if opt.is_empty() {
                "none".into()
            } else {
                opt.join(", ")
            }
        );
        for g in self.grammar {
            s.push_str(&format!(
                " {} grammar: {} e.g. {}.",
                g.column,
                g.rule,
                g.examples.join(" | ")
            ));
        }
        let mut ex = String::new();
        crate::jsonl::canon(&self.example_call(scope), &mut ex);
        s.push_str(&format!(
            " Example: {ex}. Every kind: la_status sections [\"{SECTION}\"]."
        ));
        s
    }

    /// 2026-10-24: The example as a whole `la_record` argument object for `scope`.
    pub fn example_call(&self, scope: &str) -> Value {
        let mut o = Map::new();
        o.insert("kind".into(), Value::String(self.kind.into()));
        o.insert("scope".into(), Value::String(scope.into()));
        o.insert(
            "fields".into(),
            serde_json::from_str(self.example).unwrap_or(Value::Null),
        );
        o.insert("dry_run".into(), Value::Bool(true));
        Value::Object(o)
    }

    /// 2026-10-24: The schema as the `record_schema` section lists it.
    pub fn to_json(&self, scope: &str) -> Value {
        let cols: Vec<Value> = self
            .columns
            .iter()
            .map(|c| {
                let mut o = Map::new();
                o.insert("name".into(), Value::String(c.name.into()));
                o.insert("rule".into(), Value::String(c.fill.rule()));
                o.insert("doc".into(), Value::String(c.doc.into()));
                Value::Object(o)
            })
            .collect();
        let computed: Map<String, Value> = self
            .computed
            .iter()
            .map(|(k, r)| ((*k).to_string(), Value::String((*r).into())))
            .collect();
        let grammar: Map<String, Value> = self
            .grammar
            .iter()
            .map(|g| {
                let mut o = Map::new();
                o.insert("rule".into(), Value::String(g.rule.into()));
                o.insert("examples".into(), Value::from(g.examples.to_vec()));
                (g.column.to_string(), Value::Object(o))
            })
            .collect();
        let mut o = Map::new();
        o.insert("kind".into(), Value::String(self.kind.into()));
        o.insert("file".into(), Value::String(self.file.into()));
        o.insert("columns".into(), Value::from(cols));
        if !computed.is_empty() {
            o.insert("computed".into(), Value::Object(computed));
        }
        if !grammar.is_empty() {
            o.insert("grammar".into(), Value::Object(grammar));
        }
        o.insert("example".into(), self.example_call(scope));
        Value::Object(o)
    }
}

fn cell_ok(name: &str, v: &str) -> Result<()> {
    if v.contains(['\t', '\n', '\r']) {
        return Err(error!(
            ErrorCode::LatlasToolArgument,
            name.to_string(),
            "holds a tab or a line break"
        ));
    }
    if let Some(p) = secret_pattern(v) {
        return Err(error!(
            ErrorCode::LatlasToolArgument,
            name.to_string(),
            format!("matches the secrets denylist ({p})")
        ));
    }
    Ok(())
}

/// 2026-10-24: The schema of `kind`, or a refusal that lists the kinds.
pub fn by_kind<'a>(schemas: &'a [KindSchema], kind: &str) -> Result<&'a KindSchema> {
    schemas.iter().find(|s| s.kind == kind).ok_or_else(|| {
        let kinds: Vec<&str> = schemas.iter().map(|s| s.kind).collect();
        error!(
            ErrorCode::LatlasToolArgument,
            "kind",
            format!(
                "`{kind}` is not one of {}; columns: la_status sections [\"{SECTION}\"]",
                kinds.join(", ")
            )
        )
    })
}

/// 2026-10-24: The `record_schema` section: every kind's schema for `scope`.
pub fn section(schemas: &[KindSchema], scope: &str) -> Value {
    let mut o = Map::new();
    o.insert(
        "how".into(),
        Value::String(
            "la_record {kind, scope, fields}: pass the required columns; each optional one you \
             leave out is filled by its rule and listed under `filled` in the answer. \
             dry_run: true checks without writing."
                .into(),
        ),
    );
    o.insert(
        "kinds".into(),
        Value::from(schemas.iter().map(|s| s.to_json(scope)).collect::<Vec<_>>()),
    );
    Value::Object(o)
}

/// 2026-10-24: Days since 1970-01-01 as a proleptic Gregorian (year, month, day).
fn civil(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

/// 2026-10-24: `t` as UTC `YYYY-MM-DD`.
pub fn utc_date(t: UnixMs) -> String {
    let (y, m, d) = civil(t.0.div_euclid(86_400_000));
    format!("{y:04}-{m:02}-{d:02}")
}

/// 2026-10-24: `t` as UTC `YYYY-MM-DDTHH:MMZ` (the ledger's `ts` form).
pub fn utc_minute(t: UnixMs) -> String {
    let sod = t.0.div_euclid(1000).rem_euclid(86_400);
    format!("{}T{:02}:{:02}Z", utc_date(t), sod / 3600, sod % 3600 / 60)
}

#[cfg(test)]
#[path = "record_schema_tests.rs"]
mod tests;
