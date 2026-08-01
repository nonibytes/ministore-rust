use ministore_okf::{parse_document, CollectionForm, Finding, FindingCode, NodeKind, ScalarKind};
use proptest::prelude::*;

#[test]
fn preserves_source_bytes_and_exact_ranges() {
    struct Case<'a> {
        name: &'a str,
        raw: &'a [u8],
        frontmatter: &'a [u8],
        body: &'a [u8],
        codes: &'a [FindingCode],
    }
    let cases = [
        Case {
            name: "LF",
            raw: b"---\ntype: Reference\n---\nbody  \n---\n",
            frontmatter: b"type: Reference\n",
            body: b"body  \n---\n",
            codes: &[],
        },
        Case {
            name: "CRLF without final newline",
            raw: b"---\r\ntype: Reference\r\n---\r\nbody  ",
            frontmatter: b"type: Reference\r\n",
            body: b"body  ",
            codes: &[],
        },
        Case {
            name: "BOM and tolerated opening whitespace",
            raw: b"\xef\xbb\xbf \t--- \r\ntype: Reference\r\n---\r\n",
            frontmatter: b"type: Reference\r\n",
            body: b"",
            codes: &[FindingCode::OKF107, FindingCode::OKF108],
        },
    ];

    for case in cases {
        let parsed = parse_document("concept.md", case.raw).unwrap();
        assert_eq!(parsed.value.raw(), case.raw, "{}", case.name);
        assert_eq!(
            parsed.value.frontmatter_yaml(),
            Some(case.frontmatter),
            "{}",
            case.name
        );
        assert_eq!(parsed.value.body(), Some(case.body), "{}", case.name);
        assert_codes(&parsed.findings, case.codes);
    }
}

#[test]
fn emits_format_findings_without_operational_errors() {
    let cases: &[(&str, &[u8], FindingCode)] = &[
        ("invalid UTF-8", b"---\n\xff", FindingCode::OKF100),
        ("missing opening", b"type: Reference\n", FindingCode::OKF101),
        (
            "missing closing",
            b"---\ntype: Reference\n",
            FindingCode::OKF102,
        ),
        (
            "invalid YAML",
            b"---\ntype: [unterminated\n---\n",
            FindingCode::OKF103,
        ),
        ("empty YAML", b"---\n---\n", FindingCode::OKF104),
        (
            "sequence root",
            b"---\n- Reference\n---\n",
            FindingCode::OKF104,
        ),
    ];

    for (name, raw, code) in cases {
        let parsed = parse_document("concept.md", raw).unwrap();
        assert_codes(&parsed.findings, &[*code]);
        assert_eq!(parsed.value.raw(), *raw, "{name}");
    }
}

#[test]
fn uses_first_recognized_key_and_typed_accessors() {
    let raw = b"---\n\
type: First\n\
type: Second\n\
title: 2026-07-31\n\
stale_after: 2026-07-31\n\
tags: &tags [one, two]\n\
x-alias: *tags\n\
verified: {by: 'human:reviewer', at: 2026-07-31T12:00:00Z}\n\
x-extension: !vendor {nested: true}\n\
---\nbody\n";
    let parsed = parse_document("concept.md", raw).unwrap();
    assert_codes(&parsed.findings, &[FindingCode::OKF106]);
    let metadata = parsed.value.metadata().unwrap();
    assert_eq!(metadata.string("type"), Some("First"));
    assert_eq!(metadata.string("title"), None);
    assert_eq!(metadata.date("stale_after"), Some("2026-07-31"));

    let (tags, form, valid) = metadata.strings("tags");
    assert_eq!(tags, ["one", "two"]);
    assert_eq!(form, CollectionForm::Sequence);
    assert!(valid);

    let (verified, form, valid) = metadata.mappings("verified");
    assert_eq!(verified.len(), 1);
    assert_eq!(form, CollectionForm::Mapping);
    assert!(valid);
    assert!(metadata.get("x-extension").is_some());

    let title = metadata.get("title").unwrap();
    assert!(matches!(
        title.kind,
        NodeKind::Scalar(ref scalar) if scalar.kind == ScalarKind::Timestamp
    ));
}

#[test]
fn resolves_aliases_for_recognized_values() {
    let parsed = parse_document(
        "concept.md",
        b"---\ntype: &kind Reference\ntitle: *kind\n---\n",
    )
    .unwrap();
    let metadata = parsed.value.metadata().unwrap();
    assert_eq!(metadata.string("title"), Some("Reference"));
}

#[test]
fn parser_owns_input() {
    let mut raw = b"---\ntype: Reference\n---\nbody".to_vec();
    let parsed = parse_document("concept.md", &raw).unwrap();
    raw[0] = b'x';
    assert_eq!(parsed.value.raw()[0], b'-');
}

#[test]
fn recursive_alias_in_unknown_extension_is_preserved_without_expansion() {
    let parsed = parse_document(
        "concept.md",
        b"---\ntype: Reference\nx-recursive: &value [*value]\n---\n",
    )
    .unwrap();
    assert!(parsed.findings.is_empty());
    assert!(parsed
        .value
        .metadata()
        .unwrap()
        .get("x-recursive")
        .is_some());
}

#[test]
fn classifies_timestamps_strings_and_numbers_consistently() {
    let parsed = parse_document(
        "concept.md",
        b"---\ntype: Reference\ntitle: 2026-7-1T02:03:04Z\ndescription: \"2026-7-1T02:03:04Z\"\nresource: 42\n---\n",
    )
    .unwrap();
    assert!(parsed.findings.is_empty());
    let metadata = parsed.value.metadata().unwrap();
    assert_eq!(metadata.string("title"), None);
    assert_eq!(metadata.date("title"), Some("2026-7-1T02:03:04Z"));
    assert_eq!(metadata.string("description"), Some("2026-7-1T02:03:04Z"));
    assert_eq!(metadata.string("resource"), None);
}

proptest! {
    #[test]
    fn arbitrary_input_never_panics_or_changes_raw_bytes(
        raw in proptest::collection::vec(any::<u8>(), 0..65_536)
    ) {
        let parsed = parse_document("property.md", &raw).unwrap();
        prop_assert_eq!(parsed.value.raw(), raw.as_slice());
    }
}

fn assert_codes(findings: &[Finding], expected: &[FindingCode]) {
    let actual: Vec<_> = findings.iter().map(|finding| finding.code).collect();
    assert_eq!(actual, expected);
}
