use chrono::NaiveDate;
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

use crate::finding::{Finding, FindingCode, Severity};
use crate::yaml::{CollectionForm, Metadata, Node};
use crate::Document;

use super::families::{validate_attested_computation, validate_provenance_and_trust};
use super::reserved::byte_position;

pub(super) fn validate_concept_advisories(document: &Document) -> Vec<Finding> {
    let Some(metadata) = document.metadata() else {
        return Vec::new();
    };
    let mut findings = Vec::new();

    if let Some(node) = metadata.get("status") {
        let value = metadata.string("status");
        if !matches!(value, Some("draft" | "stable" | "deprecated")) {
            findings.push(advisory_finding(
                FindingCode::OKF320,
                document.path(),
                metadata,
                node,
                "status should be draft, stable, or deprecated",
                "5.4",
            ));
        }
    }
    if let Some(node) = metadata.get("stale_after") {
        if !metadata.date("stale_after").is_some_and(valid_iso_date) {
            findings.push(advisory_finding(
                FindingCode::OKF321,
                document.path(),
                metadata,
                node,
                "stale_after must be an ISO date (YYYY-MM-DD)",
                "5.5",
            ));
        }
    }
    if let Some(node) = metadata.get("tags") {
        let (_, form, valid) = metadata.strings("tags");
        if form != CollectionForm::Sequence || !valid {
            findings.push(advisory_finding(
                FindingCode::OKF330,
                document.path(),
                metadata,
                node,
                "tags should be a sequence of strings",
                "4.1",
            ));
        }
    }
    if let Some(node) = metadata.get("timestamp") {
        if metadata.get("generated").is_none() {
            findings.push(advisory_finding(
                FindingCode::OKF340,
                document.path(),
                metadata,
                node,
                "legacy timestamp is used as the generated.at fallback",
                "13.1",
            ));
        }
    }
    findings.extend(legacy_citations_findings(document));
    findings.extend(validate_provenance_and_trust(document));
    findings.extend(validate_attested_computation(document));
    findings
}

fn legacy_citations_findings(document: &Document) -> Vec<Finding> {
    let Some(body) = document.body() else {
        return Vec::new();
    };
    let text = std::str::from_utf8(body).expect("parsed document body is valid UTF-8");
    let mut heading: Option<(usize, String)> = None;
    for (event, range) in Parser::new(text).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            }) => heading = Some((range.start, String::new())),
            Event::Text(value) | Event::Code(value) if heading.is_some() => {
                heading.as_mut().unwrap().1.push_str(&value);
            }
            Event::End(TagEnd::Heading(HeadingLevel::H1)) => {
                let (offset, value) = heading.take().unwrap();
                if value.trim() == "Citations" {
                    let raw_offset = document.body_offset().unwrap_or(0) + offset;
                    let (line, column) = byte_position(document.raw(), raw_offset);
                    return vec![Finding {
                        severity: Severity::Warning,
                        code: FindingCode::OKF341,
                        path: document.path().to_owned(),
                        line: Some(line),
                        column: Some(column),
                        spec_section: Some("13.1".into()),
                        message: "legacy Citations section is used as a provenance fallback".into(),
                    }];
                }
            }
            _ => {}
        }
    }
    Vec::new()
}

fn valid_iso_date(value: &str) -> bool {
    value.len() == 10
        && NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .is_ok_and(|date| date.format("%Y-%m-%d").to_string() == value)
}

fn advisory_finding(
    code: FindingCode,
    path: &str,
    metadata: &Metadata,
    node: &Node,
    message: &str,
    section: &str,
) -> Finding {
    let position = metadata
        .resolve(node)
        .map_or(node.position, |node| node.position);
    Finding {
        severity: Severity::Warning,
        code,
        path: path.to_owned(),
        line: Some(position.line),
        column: Some(position.column),
        spec_section: Some(section.into()),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::parse_document;

    #[test]
    fn reports_lifecycle_tag_and_legacy_advisories() {
        let source = b"---\ntype: Reference\nstatus: unknown\nstale_after: 2026-02-30\ntags: one\ntimestamp: 2026-01-01T00:00:00Z\n---\n# Definition\n\nText.\n\n# Citations\n\n* Legacy.\n";
        let parsed = parse_document("concept.md", source).unwrap();
        assert!(parsed.findings.is_empty());
        let codes = validate_concept_advisories(&parsed.value)
            .into_iter()
            .map(|finding| finding.code)
            .collect::<Vec<_>>();
        assert_eq!(
            codes,
            [
                FindingCode::OKF320,
                FindingCode::OKF321,
                FindingCode::OKF330,
                FindingCode::OKF340,
                FindingCode::OKF341,
            ]
        );
    }

    #[test]
    fn generated_suppresses_legacy_timestamp_fallback_warning() {
        let parsed = parse_document(
            "concept.md",
            b"---\ntype: Reference\ntimestamp: old\ngenerated: {by: human:one}\n---\n",
        )
        .unwrap();
        assert!(validate_concept_advisories(&parsed.value).is_empty());
    }
}
