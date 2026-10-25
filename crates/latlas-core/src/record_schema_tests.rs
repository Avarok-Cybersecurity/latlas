// SPDX-License-Identifier: MIT OR Apache-2.0

//! 2026-10-24: `record_schema` unit tests: fills, refusals and the Clock-port date forms.

use super::*;

const COLS: &[Column] = &[
    Column {
        name: "ts",
        fill: Fill::NowUtcMinute,
        doc: "when",
    },
    Column {
        name: "id",
        fill: Fill::Required,
        doc: "the id",
    },
    Column {
        name: "note",
        fill: Fill::Literal("-"),
        doc: "unused",
    },
    Column {
        name: "day",
        fill: Fill::TodayUtc,
        doc: "date",
    },
    Column {
        name: "why",
        fill: Fill::Required,
        doc: "reason",
    },
];

const S: KindSchema = KindSchema {
    kind: "thing",
    file: "THINGS.tsv",
    columns: COLS,
    computed: &[("thing_id", "sha256 of the other columns")],
    grammar: &[Grammar {
        column: "id",
        rule: "a word",
        examples: &["a-1"],
    }],
    example: r#"{"id":"a-1","why":"because"}"#,
};

/// 2026-10-24: 2000-02-29T13:05:09Z: a leap day, an hour and a minute that a wrong formula
/// shifts.
const T: UnixMs = UnixMs(951_829_509_000);

fn f(kv: &[(&str, &str)]) -> BTreeMap<String, String> {
    kv.iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[test]
fn absent_optional_columns_fill_by_their_rule_and_every_fill_is_reported() {
    let out = S.fill(&f(&[("id", "x"), ("why", "y")]), T, "c/s").unwrap();
    assert_eq!(
        out.cells,
        ["2000-02-29T13:05Z", "x", "-", "2000-02-29", "y"]
    );
    assert_eq!(out.filled.len(), 3, "{:?}", out.filled);
    assert!(out.filled[0].starts_with("ts = 2000-02-29T13:05Z (optional, default now"));
    assert_eq!(out.filled[1], "note = - (optional, default `-`)");
    assert!(out.filled[2].starts_with("day = 2000-02-29 (optional, default today"));
}

#[test]
fn a_passed_value_wins_over_the_rule_and_is_not_reported_as_filled() {
    let all = [
        ("ts", "t"),
        ("id", "x"),
        ("note", "n"),
        ("day", "d"),
        ("why", "y"),
    ];
    let out = S.fill(&f(&all), T, "c/s").unwrap();
    assert_eq!(out.cells, ["t", "x", "n", "d", "y"]);
    assert!(out.filled.is_empty());
}

#[test]
fn an_unknown_field_is_refused_with_the_columns_and_the_example() {
    let e = S
        .fill(&f(&[("summary", "s"), ("id", "x"), ("why", "y")]), T, "c/s")
        .unwrap_err()
        .to_string();
    assert!(e.contains("summary: is not a column of thing"), "{e}");
    assert!(e.contains("required id, why"), "{e}");
    assert!(e.contains("optional ts (default now"), "{e}");
    assert!(e.contains("note (default `-`)"), "{e}");
    assert!(e.contains("id grammar: a word e.g. a-1"), "{e}");
    assert!(
        e.contains(r#""fields":{"id":"a-1","why":"because"}"#) && e.contains(r#""scope":"c/s""#),
        "{e}"
    );
    assert!(e.contains("la_status sections [\"record_schema\"]"), "{e}");
}

#[test]
fn every_missing_required_field_is_named_at_once() {
    let e = S.fill(&f(&[]), T, "c/s").unwrap_err().to_string();
    assert!(e.contains("id, why: required by thing"), "{e}");
    let one = S
        .fill(&f(&[("id", "x")]), T, "c/s")
        .unwrap_err()
        .to_string();
    assert!(one.contains("why: required by thing"), "{one}");
    assert!(!one.contains("id, why: required"), "{one}");
}

#[test]
fn a_computed_column_is_refused() {
    let e = S
        .fill(
            &f(&[("thing_id", "z"), ("id", "x"), ("why", "y")]),
            T,
            "c/s",
        )
        .unwrap_err()
        .to_string();
    assert!(e.contains("thing_id: is computed (sha256"), "{e}");
}

#[test]
fn a_cell_that_would_shift_columns_or_leak_a_secret_is_refused() {
    for bad in ["a\tb", "a\nb", "a\rb"] {
        let e = S
            .fill(&f(&[("id", bad), ("why", "y")]), T, "c/s")
            .unwrap_err();
        assert!(e.to_string().contains("id: holds a tab"), "{e}");
    }
    let e = S
        .fill(&f(&[("id", "x"), ("why", "sk-or-v1-zzzz")]), T, "c/s")
        .unwrap_err();
    assert!(e.to_string().contains("secrets denylist"), "{e}");
}

#[test]
fn an_unknown_kind_lists_the_kinds() {
    let e = by_kind(&[S], "frictionn").unwrap_err().to_string();
    assert!(e.contains("`frictionn` is not one of thing"), "{e}");
    assert_eq!(by_kind(&[S], "thing").unwrap().kind, "thing");
}

#[test]
fn the_section_lists_rules_grammar_and_a_parseable_example_call() {
    let v = section(&[S], "c/s");
    let k = &v["kinds"][0];
    assert_eq!(k["kind"], "thing");
    assert_eq!(k["columns"][1]["rule"], "required");
    assert_eq!(k["columns"][2]["rule"], "optional, default `-`");
    assert_eq!(k["grammar"]["id"]["examples"][0], "a-1");
    assert_eq!(k["computed"]["thing_id"], "sha256 of the other columns");
    assert_eq!(k["example"]["scope"], "c/s");
    assert_eq!(k["example"]["fields"]["why"], "because");
}

#[test]
fn utc_forms_match_the_calendar_across_leap_days_epoch_and_centuries() {
    let cases: &[(i64, &str)] = &[
        (0, "1970-01-01T00:00Z"),
        (951_782_400_000, "2000-02-29T00:00Z"),
        (951_868_799_999, "2000-02-29T23:59Z"),
        (-1, "1969-12-31T23:59Z"),
        (1_792_886_227_487, "2026-10-24T23:57Z"),
        (1_735_689_540_000, "2024-12-31T23:59Z"),
        (4_102_444_800_000, "2100-01-01T00:00Z"),
    ];
    for (ms, want) in cases {
        assert_eq!(utc_minute(UnixMs(*ms)), *want, "{ms}");
        assert_eq!(utc_date(UnixMs(*ms)), want[..10], "{ms}");
    }
}
