use chrono::NaiveDate;
use pulldown_cmark::{Event, HeadingLevel, Parser, Tag, TagEnd};

use crate::document::parse_document;
use crate::finding::{Finding, FindingCode, Severity};
use crate::yaml::NodeKind;
use crate::Result;

const UTF8_BOM: &[u8] = &[0xef, 0xbb, 0xbf];

struct IndexBody<'a> {
    source: Option<&'a [u8]>,
    offset: usize,
    findings: Vec<Finding>,
    declared_version: Option<String>,
}

#[cfg(test)]
pub(super) fn validate_index(path: &str, raw: &[u8]) -> Result<Vec<Finding>> {
    Ok(validate_index_with_version(path, raw)?.0)
}

pub(super) fn validate_index_with_version(
    path: &str,
    raw: &[u8],
) -> Result<(Vec<Finding>, Option<String>)> {
    let Ok(_) = std::str::from_utf8(raw) else {
        return Ok((
            vec![invalid_utf8(path, raw, "index is not valid UTF-8")],
            None,
        ));
    };
    let parsed_body = index_body(path, raw)?;
    let Some(body) = parsed_body.source else {
        return Ok((parsed_body.findings, parsed_body.declared_version));
    };
    let body_offset = parsed_body.offset;
    let mut findings = parsed_body.findings;
    let declared_version = parsed_body.declared_version;
    let text = std::str::from_utf8(body).expect("index body was already validated as UTF-8");
    let mut depth = 0usize;
    let mut list_depth = 0usize;
    let mut seen_heading = false;
    let mut section_has_list = false;
    let mut malformed_offset = None;
    let mut item: Option<(usize, Option<bool>)> = None;

    for (event, range) in Parser::new(text).into_offset_iter() {
        match &event {
            Event::Start(tag) => {
                if depth == 0 {
                    match tag {
                        Tag::Heading { .. } => {
                            if seen_heading && !section_has_list {
                                malformed_offset.get_or_insert(range.start);
                            }
                            seen_heading = true;
                            section_has_list = false;
                        }
                        Tag::List(_) => {
                            if !seen_heading {
                                malformed_offset.get_or_insert(range.start);
                            } else {
                                section_has_list = true;
                            }
                        }
                        _ => {
                            malformed_offset.get_or_insert(range.start);
                        }
                    }
                }
                if matches!(tag, Tag::List(_)) {
                    list_depth += 1;
                } else if matches!(tag, Tag::Item) && list_depth == 1 {
                    item = Some((range.start, None));
                } else if let Some((_, first)) = item.as_mut() {
                    if first.is_none() && !matches!(tag, Tag::Paragraph) {
                        *first = Some(matches!(tag, Tag::Link { .. }));
                    }
                }
                depth += 1;
            }
            Event::End(end) => {
                if matches!(end, TagEnd::Item) && list_depth == 1 {
                    if let Some((offset, first)) = item.take() {
                        if first != Some(true) {
                            let (line, column) = byte_position(raw, body_offset + offset);
                            findings.push(reserved_finding(
                                FindingCode::OKF202,
                                path,
                                line,
                                column,
                                "index list entry must begin with a Markdown link",
                            ));
                        }
                    }
                }
                if matches!(end, TagEnd::List(_)) {
                    list_depth -= 1;
                }
                depth -= 1;
            }
            Event::Text(value) => {
                if let Some((_, first)) = item.as_mut() {
                    if first.is_none() && !value.trim().is_empty() {
                        *first = Some(false);
                    }
                }
            }
            Event::Code(_) | Event::InlineHtml(_) | Event::Html(_) => {
                if let Some((_, first)) = item.as_mut() {
                    if first.is_none() {
                        *first = Some(false);
                    }
                }
            }
            Event::Rule if depth == 0 => {
                malformed_offset.get_or_insert(range.start);
            }
            _ => {}
        }
    }
    if !seen_heading || !section_has_list {
        malformed_offset.get_or_insert(body.len());
    }
    if let Some(offset) = malformed_offset {
        let (line, column) = byte_position(raw, body_offset + offset);
        findings.push(reserved_finding(
            FindingCode::OKF201,
            path,
            line,
            column,
            "index must contain heading sections made of link-first lists",
        ));
    }
    Ok((findings, declared_version))
}

fn index_body<'a>(path: &str, raw: &'a [u8]) -> Result<IndexBody<'a>> {
    let content_start = usize::from(raw.starts_with(UTF8_BOM)) * UTF8_BOM.len();
    let line_end = raw[content_start..]
        .iter()
        .position(|byte| *byte == b'\n')
        .map_or(raw.len(), |position| content_start + position);
    let content_end = if line_end > content_start && raw[line_end - 1] == b'\r' {
        line_end - 1
    } else {
        line_end
    };
    if std::str::from_utf8(&raw[content_start..content_end])
        .expect("index was already validated as UTF-8")
        .trim()
        != "---"
    {
        return Ok(IndexBody {
            source: Some(raw),
            offset: 0,
            findings: Vec::new(),
            declared_version: None,
        });
    }
    let parsed = parse_document(path, raw)?;
    if path != "index.md" {
        let finding = reserved_finding(
            FindingCode::OKF200,
            path,
            1,
            1,
            "only the root index may have frontmatter",
        );
        let body_offset = parsed.value.body_offset();
        return Ok(IndexBody {
            source: body_offset.map(|offset| &raw[offset..]),
            offset: body_offset.unwrap_or(0),
            findings: vec![finding],
            declared_version: None,
        });
    }
    let body_offset = parsed.value.body_offset();
    let (declared_version, mut version_findings) =
        validate_declared_version(path, parsed.value.metadata());
    let mut findings = parsed.findings;
    findings.append(&mut version_findings);
    Ok(IndexBody {
        source: body_offset.map(|offset| &raw[offset..]),
        offset: body_offset.unwrap_or(0),
        findings,
        declared_version,
    })
}

fn validate_declared_version(
    path: &str,
    metadata: Option<&crate::yaml::Metadata>,
) -> (Option<String>, Vec<Finding>) {
    if path != "index.md" {
        return (None, Vec::new());
    }
    let Some(metadata) = metadata else {
        return (None, Vec::new());
    };
    let Some(node) = metadata.get("okf_version") else {
        return (None, Vec::new());
    };
    let resolved = metadata.resolve(node);
    let value = resolved.and_then(|node| match &node.kind {
        NodeKind::Scalar(scalar) => Some(scalar.value.as_str()),
        _ => None,
    });
    let declared_version = value.map(str::to_owned);
    let position = resolved.map_or(node.position, |node| node.position);
    if !value.is_some_and(valid_version) {
        return (
            declared_version,
            vec![version_finding(
                FindingCode::OKF206,
                path,
                position.line,
                position.column,
                "okf_version must use <major>.<minor> syntax",
            )],
        );
    }
    if value != Some("0.2") {
        return (
            declared_version,
            vec![version_finding(
                FindingCode::OKF207,
                path,
                position.line,
                position.column,
                "declared OKF version is not implemented exactly",
            )],
        );
    }
    (declared_version, Vec::new())
}

fn valid_version(value: &str) -> bool {
    let Some((major, minor)) = value.split_once('.') else {
        return false;
    };
    !major.is_empty()
        && !minor.is_empty()
        && !minor.contains('.')
        && major.bytes().all(|byte| byte.is_ascii_digit())
        && minor.bytes().all(|byte| byte.is_ascii_digit())
}

fn version_finding(
    code: FindingCode,
    path: &str,
    line: usize,
    column: usize,
    message: &str,
) -> Finding {
    let mut finding = reserved_finding(code, path, line, column, message);
    finding.severity = Severity::Warning;
    finding.spec_section = Some("12".into());
    finding
}

pub(super) fn validate_log(path: &str, raw: &[u8]) -> Vec<Finding> {
    let Ok(text) = std::str::from_utf8(raw) else {
        return vec![invalid_utf8(path, raw, "log is not valid UTF-8")];
    };
    let mut findings = Vec::new();
    let mut heading: Option<(usize, String)> = None;
    for (event, range) in Parser::new(text).into_offset_iter() {
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H2,
                ..
            }) => heading = Some((range.start, String::new())),
            Event::Text(value) | Event::Code(value) if heading.is_some() => {
                heading.as_mut().unwrap().1.push_str(&value);
            }
            Event::SoftBreak | Event::HardBreak if heading.is_some() => {
                heading.as_mut().unwrap().1.push('\n');
            }
            Event::End(TagEnd::Heading(HeadingLevel::H2)) => {
                let (offset, value) = heading.take().unwrap();
                if !is_iso_date(value.trim()) {
                    let (line, column) = byte_position(raw, offset);
                    findings.push(reserved_finding(
                        FindingCode::OKF203,
                        path,
                        line,
                        column,
                        "level-2 log heading must be an ISO date (YYYY-MM-DD)",
                    ));
                }
            }
            _ => {}
        }
    }
    findings
}

fn is_iso_date(value: &str) -> bool {
    value.len() == 10
        && NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .is_ok_and(|date| date.format("%Y-%m-%d").to_string() == value)
}

fn invalid_utf8(path: &str, raw: &[u8], message: &str) -> Finding {
    let offset = std::str::from_utf8(raw).unwrap_err().valid_up_to();
    let (line, column) = byte_position(raw, offset);
    reserved_finding(FindingCode::OKF100, path, line, column, message)
}

pub(super) fn byte_position(raw: &[u8], offset: usize) -> (usize, usize) {
    let mut line = 1;
    let mut column = 1;
    for character in String::from_utf8_lossy(&raw[..offset.min(raw.len())]).chars() {
        if character == '\n' {
            line += 1;
            column = 1;
        } else {
            column += 1;
        }
    }
    (line, column)
}

fn reserved_finding(
    code: FindingCode,
    path: &str,
    line: usize,
    column: usize,
    message: &str,
) -> Finding {
    Finding {
        severity: Severity::Error,
        code,
        path: path.to_owned(),
        line: Some(line),
        column: Some(column),
        spec_section: Some("4.1".into()),
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_link_first_index_and_iso_log_dates() {
        assert!(validate_index(
            "index.md",
            b"---\nokf_version: 0.2\n---\n# Concepts\n\n* [One](one.md) detail\n"
        )
        .unwrap()
        .is_empty());
        assert!(validate_log("log.md", b"# Log\n\n## 2026-06-25\n\n* Added.\n").is_empty());
    }

    #[test]
    fn commonmark_structure_ignores_fenced_markup() {
        let index = validate_index("index.md", b"```md\n# Fake\n* [Fake](x)\n```\n").unwrap();
        assert_eq!(
            index.iter().map(|finding| finding.code).collect::<Vec<_>>(),
            [FindingCode::OKF201]
        );
        assert!(validate_log("log.md", b"```md\n## Not a date\n```\n").is_empty());
    }

    #[test]
    fn rejects_nested_frontmatter_link_last_entries_and_invalid_calendar_dates() {
        let nested = validate_index(
            "nested/index.md",
            b"---\nokf_version: 0.2\n---\n# Concepts\n\n* [One](one.md)\n",
        )
        .unwrap();
        assert_eq!(
            nested
                .iter()
                .map(|finding| finding.code)
                .collect::<Vec<_>>(),
            [FindingCode::OKF200]
        );

        let link_last =
            validate_index("index.md", b"# Concepts\n\n* Description [One](one.md)\n").unwrap();
        assert_eq!(
            link_last
                .iter()
                .map(|finding| finding.code)
                .collect::<Vec<_>>(),
            [FindingCode::OKF202]
        );

        let log = validate_log("log.md", b"## 2026-02-30\n");
        assert_eq!(
            log.iter().map(|finding| finding.code).collect::<Vec<_>>(),
            [FindingCode::OKF203]
        );
    }

    #[test]
    fn validates_declared_version_syntax_and_support() {
        let (valid, declared) = validate_index_with_version(
            "index.md",
            b"---\nokf_version: 0.2\n---\n# Concepts\n\n* [One](one.md)\n",
        )
        .unwrap();
        assert!(valid.is_empty());
        assert_eq!(declared.as_deref(), Some("0.2"));

        let (unsupported, declared) = validate_index_with_version(
            "index.md",
            b"---\nokf_version: 0.3\n---\n# Concepts\n\n* [One](one.md)\n",
        )
        .unwrap();
        assert_eq!(unsupported[0].code, FindingCode::OKF207);
        assert_eq!(declared.as_deref(), Some("0.3"));

        let (invalid, _) = validate_index_with_version(
            "index.md",
            b"---\nokf_version: v2\n---\n# Concepts\n\n* [One](one.md)\n",
        )
        .unwrap();
        assert_eq!(invalid[0].code, FindingCode::OKF206);
    }
}
