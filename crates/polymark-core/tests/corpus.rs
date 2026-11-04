use std::path::PathBuf;

use polymark_core::{Input, Output, clean, report, report_clean, transform};
use serde_json::Value;

fn corpus() -> Value {
    let path = std::env::var_os("POLYMARK_CORPUS")
        .map_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../testdata/cases.json"), PathBuf::from);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let corpus: Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{}: {e}", path.display()));

    assert_eq!(corpus["format"], 1, "{}: corpus format", path.display());
    corpus
}

fn input(case: &Value) -> String {
    let text = case["input"].as_str().expect("input");
    let repeat = case.get("repeat").map_or(1, |n| usize::try_from(n.as_u64().expect("repeat")).expect("usize"));

    text.repeat(repeat)
}

fn cases<'a>(corpus: &'a Value, kind: &str) -> &'a [Value] {
    let cases = corpus[kind].as_array().unwrap_or_else(|| panic!("{kind}: array expected"));

    assert!(!cases.is_empty(), "{kind}: no cases");
    cases
}

#[test]
fn transform_cases() {
    let corpus = corpus();

    for case in cases(&corpus, "transform") {
        let name = case["name"].as_str().expect("name");
        let from = Input::parse(case["from"].as_str().expect("from")).expect("known input");
        let to = Output::parse(case["to"].as_str().expect("to")).expect("known output");
        let got: Value = serde_json::from_str(&report(&transform(&input(case), from, to))).expect("report JSON");

        assert_eq!(got, case["want"], "{name}");
    }
}

#[test]
fn clean_cases() {
    let corpus = corpus();

    for case in cases(&corpus, "clean") {
        let name = case["name"].as_str().expect("name");
        let got: Value = serde_json::from_str(&report_clean(&clean(&input(case)))).expect("report JSON");

        assert_eq!(got, case["want"], "{name}");
    }
}
