use std::collections::HashSet;

use chrono::{DateTime, NaiveDate, Utc};
use pulldown_cmark::{Event, HeadingLevel, Options, Parser, Tag, TagEnd};

use crate::finding::{Finding, FindingCode, Severity};
use crate::yaml::{Metadata, Node, NodeKind, ScalarKind};
use crate::Document;

use super::reserved::byte_position;

pub(super) fn validate_provenance_and_trust(document: &Document) -> Vec<Finding> {
    let Some(metadata) = document.metadata() else {
        return Vec::new();
    };
    let (mut findings, source_ids) = validate_sources(document.path(), metadata);
    findings.extend(validate_usage_window(
        document.path(),
        metadata,
        metadata.root(),
        "usage_window",
    ));
    let (generated_at, generated_findings) = validate_generated(document.path(), metadata);
    findings.extend(generated_findings);
    let (latest_verified, verified_findings) = validate_verified(document.path(), metadata);
    findings.extend(verified_findings);
    if generated_at.is_some() && latest_verified.is_some() && latest_verified < generated_at {
        let node = metadata.get("verified").unwrap();
        findings.push(advisory_finding(
            FindingCode::OKF315,
            document.path(),
            metadata,
            node,
            "latest verification predates generated.at",
            "5.2",
        ));
    }
    findings.extend(unmatched_footnote_findings(document, &source_ids));
    findings
}

fn validate_sources(path: &str, metadata: &Metadata) -> (Vec<Finding>, HashSet<String>) {
    let mut ids = HashSet::new();
    let Some(original) = metadata.get("sources") else {
        return (Vec::new(), ids);
    };
    let Some(node) = metadata.resolve(original) else {
        return (
            vec![advisory_finding(
                FindingCode::OKF300,
                path,
                metadata,
                original,
                "sources must be a sequence of mappings",
                "5.1",
            )],
            ids,
        );
    };
    let mut findings = Vec::new();
    let mut mappings = Vec::new();
    match &node.kind {
        NodeKind::Mapping(_) => {
            mappings.push(node);
            findings.push(advisory_finding(
                FindingCode::OKF300,
                path,
                metadata,
                node,
                "sources should be a sequence; a mapping is consumed as one entry",
                "5.1",
            ));
        }
        NodeKind::Sequence(entries) => {
            for entry in entries {
                let resolved = metadata.resolve(entry);
                if resolved.is_some_and(|entry| matches!(entry.kind, NodeKind::Mapping(_))) {
                    mappings.push(resolved.unwrap());
                } else {
                    findings.push(advisory_finding(
                        FindingCode::OKF300,
                        path,
                        metadata,
                        entry,
                        "sources entry must be a mapping",
                        "5.1",
                    ));
                }
            }
        }
        _ => {
            findings.push(advisory_finding(
                FindingCode::OKF300,
                path,
                metadata,
                node,
                "sources must be a sequence of mappings",
                "5.1",
            ));
            return (findings, ids);
        }
    }
    for source in mappings {
        let resource = metadata.mapping_get(source, "resource");
        if resource
            .and_then(|node| metadata.node_string(node))
            .is_none_or(|value| value.trim().is_empty())
        {
            findings.push(advisory_finding(
                FindingCode::OKF301,
                path,
                metadata,
                resource.unwrap_or(source),
                "source entry requires a non-empty resource",
                "5.1",
            ));
        }
        if let Some(id_node) = metadata.mapping_get(source, "id") {
            match metadata.node_string(id_node) {
                None => findings.push(advisory_finding(
                    FindingCode::OKF300,
                    path,
                    metadata,
                    id_node,
                    "source id must be a string",
                    "5.1",
                )),
                Some(id) if !id.trim().is_empty() => {
                    if !ids.insert(id.trim().to_owned()) {
                        findings.push(advisory_finding(
                            FindingCode::OKF302,
                            path,
                            metadata,
                            id_node,
                            "source id is duplicated",
                            "5.1",
                        ));
                    }
                }
                _ => {}
            }
        }
        if let Some(title) = metadata.mapping_get(source, "title") {
            if metadata.node_string(title).is_none() {
                findings.push(advisory_finding(
                    FindingCode::OKF300,
                    path,
                    metadata,
                    title,
                    "source title must be a string",
                    "5.1",
                ));
            }
        }
        if let Some(author) = metadata.mapping_get(source, "author") {
            match metadata.node_string(author) {
                None => findings.push(advisory_finding(
                    FindingCode::OKF304,
                    path,
                    metadata,
                    author,
                    "source author must be a string",
                    "5.1",
                )),
                Some(value) if !valid_actor(value) => findings.push(advisory_finding(
                    FindingCode::OKF314,
                    path,
                    metadata,
                    author,
                    "source author does not follow the actor convention",
                    "7",
                )),
                _ => {}
            }
        }
        if let Some(count) = metadata.mapping_get(source, "usage_count") {
            if !valid_usage_count(count) {
                findings.push(advisory_finding(
                    FindingCode::OKF304,
                    path,
                    metadata,
                    count,
                    "source usage_count must be a non-negative number",
                    "5.1",
                ));
            }
        }
        if let Some(modified) = metadata.mapping_get(source, "last_modified") {
            if !metadata.node_date(modified).is_some_and(valid_iso_date) {
                findings.push(advisory_finding(
                    FindingCode::OKF304,
                    path,
                    metadata,
                    modified,
                    "source last_modified must be an ISO date",
                    "5.1",
                ));
            }
        }
        findings.extend(validate_usage_window(
            path,
            metadata,
            source,
            "usage_window",
        ));
    }
    (findings, ids)
}

fn validate_usage_window(
    path: &str,
    metadata: &Metadata,
    mapping: &Node,
    key: &str,
) -> Vec<Finding> {
    let node = if std::ptr::eq(mapping, metadata.root()) {
        metadata.get(key).and_then(|node| metadata.resolve(node))
    } else {
        metadata.mapping_get(mapping, key)
    };
    let Some(node) = node else {
        return Vec::new();
    };
    if !matches!(node.kind, NodeKind::Mapping(_)) {
        return vec![advisory_finding(
            FindingCode::OKF303,
            path,
            metadata,
            node,
            "usage_window must contain from and to dates",
            "5.1",
        )];
    }
    let from = metadata
        .mapping_get(node, "from")
        .and_then(|node| metadata.node_date(node));
    let to = metadata
        .mapping_get(node, "to")
        .and_then(|node| metadata.node_date(node));
    if !from.is_some_and(valid_iso_date)
        || !to.is_some_and(valid_iso_date)
        || from.unwrap() > to.unwrap()
    {
        return vec![advisory_finding(
            FindingCode::OKF303,
            path,
            metadata,
            node,
            "usage_window must contain an ordered ISO date range",
            "5.1",
        )];
    }
    Vec::new()
}

fn validate_generated(path: &str, metadata: &Metadata) -> (Option<DateTime<Utc>>, Vec<Finding>) {
    let Some(original) = metadata.get("generated") else {
        return (None, Vec::new());
    };
    let Some(node) = metadata.resolve(original) else {
        return (
            None,
            vec![advisory_finding(
                FindingCode::OKF310,
                path,
                metadata,
                original,
                "generated must be a mapping with a non-empty by actor",
                "5.2",
            )],
        );
    };
    if !matches!(node.kind, NodeKind::Mapping(_)) {
        return (
            None,
            vec![advisory_finding(
                FindingCode::OKF310,
                path,
                metadata,
                node,
                "generated must be a mapping with a non-empty by actor",
                "5.2",
            )],
        );
    }
    let mut findings = Vec::new();
    let by_node = metadata.mapping_get(node, "by");
    let by = by_node.and_then(|node| metadata.node_string(node));
    if by.is_none_or(|value| value.trim().is_empty()) {
        findings.push(advisory_finding(
            FindingCode::OKF310,
            path,
            metadata,
            by_node.unwrap_or(node),
            "generated requires a non-empty by actor",
            "5.2",
        ));
    } else if !valid_actor(by.unwrap()) {
        findings.push(advisory_finding(
            FindingCode::OKF314,
            path,
            metadata,
            by_node.unwrap(),
            "generated.by does not follow the actor convention",
            "7",
        ));
    }
    let mut parsed_at = None;
    if let Some(at_node) = metadata.mapping_get(node, "at") {
        parsed_at = metadata
            .node_date(at_node)
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        if parsed_at.is_none() {
            findings.push(advisory_finding(
                FindingCode::OKF311,
                path,
                metadata,
                at_node,
                "generated.at must be an ISO 8601 datetime",
                "5.2",
            ));
        }
    }
    (parsed_at, findings)
}

fn validate_verified(path: &str, metadata: &Metadata) -> (Option<DateTime<Utc>>, Vec<Finding>) {
    let Some(original) = metadata.get("verified") else {
        return (None, Vec::new());
    };
    let Some(node) = metadata.resolve(original) else {
        return (
            None,
            vec![advisory_finding(
                FindingCode::OKF312,
                path,
                metadata,
                original,
                "verified must be a mapping or sequence of mappings",
                "5.2",
            )],
        );
    };
    let mut findings = Vec::new();
    let mut events = Vec::new();
    match &node.kind {
        NodeKind::Mapping(_) => events.push(node),
        NodeKind::Sequence(items) => {
            for item in items {
                if let Some(event) = metadata.resolve(item) {
                    if matches!(event.kind, NodeKind::Mapping(_)) {
                        events.push(event);
                        continue;
                    }
                }
                findings.push(advisory_finding(
                    FindingCode::OKF312,
                    path,
                    metadata,
                    item,
                    "verified entries must be mappings",
                    "5.2",
                ));
            }
        }
        _ => {
            return (
                None,
                vec![advisory_finding(
                    FindingCode::OKF312,
                    path,
                    metadata,
                    node,
                    "verified must be a mapping or sequence of mappings",
                    "5.2",
                )],
            )
        }
    }
    let mut latest = None;
    for event in events {
        let by_node = metadata.mapping_get(event, "by");
        let at_node = metadata.mapping_get(event, "at");
        let by = by_node.and_then(|node| metadata.node_string(node));
        let at = at_node
            .and_then(|node| metadata.node_date(node))
            .and_then(|value| DateTime::parse_from_rfc3339(value).ok())
            .map(|value| value.with_timezone(&Utc));
        if by.is_none_or(|value| value.trim().is_empty()) || at.is_none() {
            findings.push(advisory_finding(
                FindingCode::OKF313,
                path,
                metadata,
                event,
                "verification event requires valid by and at values",
                "5.2",
            ));
        } else {
            if latest.is_none_or(|latest_value| at > Some(latest_value)) {
                latest = at;
            }
            if !valid_actor(by.unwrap()) {
                findings.push(advisory_finding(
                    FindingCode::OKF314,
                    path,
                    metadata,
                    by_node.unwrap(),
                    "verified.by does not follow the actor convention",
                    "7",
                ));
            }
        }
    }
    (latest, findings)
}

fn valid_actor(value: &str) -> bool {
    if value.is_empty() || value.trim() != value || value.chars().any(char::is_whitespace) {
        return false;
    }
    value
        .split_once(':')
        .or_else(|| value.split_once('/'))
        .is_some_and(|(left, right)| !left.is_empty() && !right.is_empty())
}

fn valid_usage_count(node: &Node) -> bool {
    let NodeKind::Scalar(scalar) = &node.kind else {
        return false;
    };
    if !matches!(scalar.kind, ScalarKind::Integer | ScalarKind::Float) {
        return false;
    }
    scalar
        .value
        .replace('_', "")
        .parse::<f64>()
        .is_ok_and(|value| value >= 0.0)
}

fn valid_iso_date(value: &str) -> bool {
    value.len() == 10
        && NaiveDate::parse_from_str(value, "%Y-%m-%d")
            .is_ok_and(|date| date.format("%Y-%m-%d").to_string() == value)
}

fn unmatched_footnote_findings(document: &Document, ids: &HashSet<String>) -> Vec<Finding> {
    let Some(body) = document.body() else {
        return Vec::new();
    };
    let text = std::str::from_utf8(body).expect("parsed body is valid UTF-8");
    let mut options = Options::empty();
    options.insert(Options::ENABLE_FOOTNOTES);
    let mut findings = Vec::new();
    for (event, range) in Parser::new_ext(text, options).into_offset_iter() {
        if let Event::Start(Tag::FootnoteDefinition(label)) = event {
            if !ids.contains(label.as_ref()) {
                let raw_offset = document.body_offset().unwrap_or(0) + range.start;
                let (line, column) = byte_position(document.raw(), raw_offset);
                findings.push(Finding {
                    severity: Severity::Warning,
                    code: FindingCode::OKF360,
                    path: document.path().to_owned(),
                    line: Some(line),
                    column: Some(column),
                    spec_section: Some("5.1".into()),
                    message: "footnote label has no matching source id".into(),
                });
            }
        }
    }
    findings
}

pub(super) fn validate_attested_computation(document: &Document) -> Vec<Finding> {
    let Some(metadata) = document.metadata() else {
        return Vec::new();
    };
    if metadata.string("type") != Some("Attested Computation") {
        return Vec::new();
    }
    let mut findings = Vec::new();
    let runtime = metadata.get("runtime");
    if runtime
        .and_then(|node| metadata.node_string(node))
        .is_none_or(|value| value.trim().is_empty())
    {
        findings.push(advisory_finding(
            FindingCode::OKF350,
            document.path(),
            metadata,
            runtime.unwrap_or(metadata.root()),
            "Attested Computation requires a non-empty runtime",
            "10.2",
        ));
    }
    if let Some(parameters) = metadata
        .get("parameters")
        .and_then(|node| metadata.resolve(node))
    {
        if let NodeKind::Sequence(items) = &parameters.kind {
            for item in items {
                if !valid_parameter(metadata, item) {
                    findings.push(advisory_finding(
                        FindingCode::OKF351,
                        document.path(),
                        metadata,
                        item,
                        "parameter requires string name/type and boolean required",
                        "10.2",
                    ));
                }
            }
        } else {
            findings.push(advisory_finding(
                FindingCode::OKF351,
                document.path(),
                metadata,
                parameters,
                "parameters must be a sequence",
                "10.2",
            ));
        }
    }
    let computation = metadata.get("computation");
    let has_file = computation
        .and_then(|node| metadata.node_string(node))
        .is_some_and(|value| !value.trim().is_empty());
    let has_inline = has_inline_computation(document);
    if has_file == has_inline {
        findings.push(advisory_finding(
            FindingCode::OKF352,
            document.path(),
            metadata,
            computation.unwrap_or(metadata.root()),
            "provide exactly one file or inline computation",
            "10.3",
        ));
    }
    findings.extend(validate_executor(document.path(), metadata));
    findings.extend(validate_attester(document.path(), metadata));
    findings
}

fn valid_parameter(metadata: &Metadata, node: &Node) -> bool {
    let Some(node) = metadata.resolve(node) else {
        return false;
    };
    if !matches!(node.kind, NodeKind::Mapping(_)) {
        return false;
    }
    let name = metadata
        .mapping_get(node, "name")
        .and_then(|node| metadata.node_string(node));
    let kind = metadata
        .mapping_get(node, "type")
        .and_then(|node| metadata.node_string(node));
    let required = metadata.mapping_get(node, "required");
    name.is_some_and(|value| !value.trim().is_empty())
        && kind.is_some_and(|value| !value.trim().is_empty())
        && required.is_some_and(|node| matches!(node.kind, NodeKind::Scalar(ref scalar) if scalar.kind == ScalarKind::Boolean))
}

fn has_inline_computation(document: &Document) -> bool {
    let Some(body) = document.body() else {
        return false;
    };
    let text = std::str::from_utf8(body).expect("parsed body is valid UTF-8");
    let mut in_section = false;
    let mut heading = None::<String>;
    for event in Parser::new(text) {
        match event {
            Event::Start(Tag::Heading {
                level: HeadingLevel::H1,
                ..
            }) => heading = Some(String::new()),
            Event::Text(value) | Event::Code(value) if heading.is_some() => {
                heading.as_mut().unwrap().push_str(&value)
            }
            Event::End(TagEnd::Heading(HeadingLevel::H1)) => {
                in_section = heading.take().unwrap().trim() == "Computation"
            }
            Event::Start(Tag::CodeBlock(_)) if in_section => return true,
            _ => {}
        }
    }
    false
}

fn validate_executor(path: &str, metadata: &Metadata) -> Vec<Finding> {
    let Some(node) = metadata
        .get("executor")
        .and_then(|node| metadata.resolve(node))
    else {
        return Vec::new();
    };
    if !matches!(node.kind, NodeKind::Mapping(_)) {
        return vec![advisory_finding(
            FindingCode::OKF353,
            path,
            metadata,
            node,
            "executor must be a mapping",
            "10.2",
        )];
    }
    let resource = metadata.mapping_get(node, "resource");
    if resource
        .and_then(|node| metadata.node_string(node))
        .is_none_or(|value| value.trim().is_empty())
    {
        return vec![advisory_finding(
            FindingCode::OKF353,
            path,
            metadata,
            node,
            "executor requires a non-empty resource",
            "10.2",
        )];
    }
    if let Some(receipt) = metadata.mapping_get(node, "receipt") {
        let NodeKind::Sequence(fields) = &receipt.kind else {
            return vec![advisory_finding(
                FindingCode::OKF353,
                path,
                metadata,
                receipt,
                "executor receipt must be a sequence of strings",
                "10.2",
            )];
        };
        if fields.iter().any(|field| {
            metadata
                .node_string(field)
                .is_none_or(|value| value.trim().is_empty())
        }) {
            return vec![advisory_finding(
                FindingCode::OKF353,
                path,
                metadata,
                receipt,
                "executor receipt fields must be non-empty strings",
                "10.2",
            )];
        }
    }
    Vec::new()
}

fn validate_attester(path: &str, metadata: &Metadata) -> Vec<Finding> {
    let Some(node) = metadata
        .get("attester")
        .and_then(|node| metadata.resolve(node))
    else {
        return Vec::new();
    };
    if !matches!(node.kind, NodeKind::Mapping(_)) {
        return vec![advisory_finding(
            FindingCode::OKF354,
            path,
            metadata,
            node,
            "attester must be a mapping with a resource",
            "10.2",
        )];
    }
    let resource = metadata.mapping_get(node, "resource");
    if resource
        .and_then(|node| metadata.node_string(node))
        .is_none_or(|value| value.trim().is_empty())
    {
        return vec![advisory_finding(
            FindingCode::OKF354,
            path,
            metadata,
            node,
            "attester requires a non-empty resource",
            "10.2",
        )];
    }
    Vec::new()
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
