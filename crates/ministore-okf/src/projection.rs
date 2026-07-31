use std::collections::BTreeSet;
use std::path::Path;

use chrono::{DateTime, NaiveDate};
use ministore::{FieldSpec, Schema};
use rusqlite::{params, OptionalExtension};
use serde_json::{json, Map, Value};
use sha2::{Digest, Sha256};

use crate::model::{Projection, ValidateOptions, ValidationSummary};
use crate::staging::ValidationStage;
use crate::validate::prepare_bundle;
use crate::yaml::{Metadata, NodeKind, ScalarKind};
use crate::{parse_document, Result};

pub const PROJECTION_VERSION: usize = 1;

pub fn projection_schema() -> Schema {
    let mut schema = Schema::new();
    schema.add_field("type", FieldSpec::keyword(false));
    schema.add_field("title", FieldSpec::text(Some(3.0)));
    schema.add_field("description", FieldSpec::text(Some(2.0)));
    schema.add_field("body", FieldSpec::text(Some(1.0)));
    for name in [
        "tags",
        "verified_by",
        "source_ids",
        "source_resources",
        "source_authors",
        "link_targets",
        "backlinks",
    ] {
        schema.add_field(name, FieldSpec::keyword(true));
    }
    for name in [
        "status",
        "resource",
        "generated_by",
        "trust_tier",
        "runtime",
        "okf_version",
        "okf_source_path",
        "okf_projection_hash",
    ] {
        schema.add_field(name, FieldSpec::keyword(false));
    }
    for name in ["generated_at", "latest_verified_at", "stale_after"] {
        schema.add_field(name, FieldSpec::date(false));
    }
    schema.add_field("source_last_modified", FieldSpec::date(true));
    schema.add_field("source_usage_counts", FieldSpec::number(true));
    schema.add_field("okf_projection_version", FieldSpec::number(false));
    schema
}

pub fn walk_projections<F>(
    root: &Path,
    options: &ValidateOptions,
    mut emit: F,
) -> Result<ValidationSummary>
where
    F: FnMut(Projection) -> Result<()>,
{
    let (stage, summary) = prepare_bundle(root, options)?;
    let mut after = String::new();
    loop {
        let row:Option<(String,Vec<u8>)>=stage.connection.query_row("SELECT path,raw FROM concepts WHERE path>?1 COLLATE BINARY ORDER BY path COLLATE BINARY LIMIT 1",[&after],|r|Ok((r.get(0)?,r.get(1)?))).optional()?;
        let Some((source, raw)) = row else { break };
        after.clone_from(&source);
        emit(stage.project(&source, &raw, &summary.target_version)?)?;
    }
    Ok(summary)
}

impl ValidationStage {
    pub(crate) fn project(&self, source: &str, raw: &[u8], version: &str) -> Result<Projection> {
        let document = parse_document(source, raw)?.value;
        let stem = Path::new(source)
            .file_stem()
            .and_then(|v| v.to_str())
            .unwrap_or("");
        let mut p = Map::new();
        p.insert(
            "path".into(),
            json!(format!("/{}", source.trim_end_matches(".md"))),
        );
        p.insert("raw_document".into(), json!(String::from_utf8_lossy(raw)));
        p.insert(
            "body".into(),
            json!(document
                .body()
                .and_then(|v| std::str::from_utf8(v).ok())
                .unwrap_or("")),
        );
        p.insert("title".into(), json!(stem));
        p.insert("status".into(), json!("stable"));
        p.insert("trust_tier".into(), json!("unverified"));
        p.insert("okf_version".into(), json!(version));
        p.insert("okf_source_path".into(), json!(source));
        p.insert("okf_projection_version".into(), json!(PROJECTION_VERSION));
        if let Some(m) = document.metadata() {
            for key in [
                "type",
                "title",
                "description",
                "resource",
                "status",
                "runtime",
            ] {
                if let Some(value) = m.string(key).filter(|v| !v.is_empty()) {
                    p.insert(key.into(), json!(value));
                }
            }
            if let Some(value) = m.date("stale_after").filter(|v| valid_date(v)) {
                p.insert("stale_after".into(), json!(value));
            }
            let (tags, _, _) = m.strings("tags");
            insert_strings(
                &mut p,
                "tags",
                tags.into_iter().map(str::to_owned).collect(),
            );
            project_generated(m, &mut p);
            project_verified(m, &mut p);
            project_sources(m, &mut p);
        }
        insert_strings(&mut p, "link_targets", self.edges(source, true)?);
        insert_strings(&mut p, "backlinks", self.edges(source, false)?);
        let encoded = serde_json::to_vec(&p)?;
        p.insert(
            "okf_projection_hash".into(),
            json!(format!("{:x}", Sha256::digest(encoded))),
        );
        Ok(p)
    }
    fn edges(&self, source: &str, forward: bool) -> Result<Vec<String>> {
        let query = if forward {
            "SELECT target FROM edges WHERE source=?1 ORDER BY target COLLATE BINARY"
        } else {
            "SELECT source FROM edges WHERE target=?1 ORDER BY source COLLATE BINARY"
        };
        let mut statement = self.connection.prepare(query)?;
        let rows = statement.query_map(params![source], |r| r.get::<_, String>(0))?;
        Ok(rows
            .collect::<std::result::Result<Vec<_>, _>>()?
            .into_iter()
            .map(|v| format!("/{}", v.trim_end_matches(".md")))
            .collect())
    }
}

fn project_generated(m: &Metadata, p: &mut Projection) {
    if let Some(generated) = m.get("generated").and_then(|v| m.resolve(v)) {
        if let Some(by) = m
            .mapping_get(generated, "by")
            .and_then(|v| m.node_string(v))
            .filter(|v| !v.is_empty())
        {
            p.insert("generated_by".into(), json!(by));
        }
        if let Some(at) = m
            .mapping_get(generated, "at")
            .and_then(|v| m.node_date(v))
            .filter(|v| DateTime::parse_from_rfc3339(v).is_ok())
        {
            p.insert("generated_at".into(), json!(at));
        }
    }
    if !p.contains_key("generated_at") {
        if let Some(at) = m
            .date("timestamp")
            .filter(|v| DateTime::parse_from_rfc3339(v).is_ok())
        {
            p.insert("generated_at".into(), json!(at));
        }
    }
}
fn project_verified(m: &Metadata, p: &mut Projection) {
    let (events, _, _) = m.mappings("verified");
    let mut actors = Vec::new();
    let mut latest = "";
    let mut human = false;
    for event in events {
        let Some(by) = m.mapping_get(event, "by").and_then(|v| m.node_string(v)) else {
            continue;
        };
        let Some(at) = m.mapping_get(event, "at").and_then(|v| m.node_date(v)) else {
            continue;
        };
        if by.is_empty() || DateTime::parse_from_rfc3339(at).is_err() {
            continue;
        }
        actors.push(by.to_owned());
        if at > latest {
            latest = at
        }
        if by.starts_with("human:") {
            human = true
        }
    }
    if !actors.is_empty() {
        insert_strings(p, "verified_by", actors);
        p.insert("latest_verified_at".into(), json!(latest));
        p.insert(
            "trust_tier".into(),
            json!(if human {
                "human-reviewed"
            } else {
                "machine-confirmed"
            }),
        );
    }
}
fn project_sources(m: &Metadata, p: &mut Projection) {
    let (sources, _, _) = m.mappings("sources");
    let mut ids = Vec::new();
    let mut resources = Vec::new();
    let mut authors = Vec::new();
    let mut modified = Vec::new();
    let mut counts: Vec<Value> = Vec::new();
    for source in sources {
        for (key, dst) in [
            ("id", &mut ids),
            ("resource", &mut resources),
            ("author", &mut authors),
        ] {
            if let Some(v) = m
                .mapping_get(source, key)
                .and_then(|n| m.node_string(n))
                .filter(|v| !v.is_empty())
            {
                dst.push(v.to_owned())
            }
        }
        if let Some(v) = m
            .mapping_get(source, "last_modified")
            .and_then(|n| m.node_date(n))
            .filter(|v| valid_date(v))
        {
            modified.push(v.to_owned())
        }
        if let Some(node) = m
            .mapping_get(source, "usage_count")
            .and_then(|n| m.resolve(n))
        {
            if let NodeKind::Scalar(s) = &node.kind {
                if matches!(s.kind, ScalarKind::Integer | ScalarKind::Float) {
                    let lexical = s.value.replace('_', "");
                    if matches!(s.kind, ScalarKind::Integer) {
                        if let Ok(v) = lexical.parse::<i64>() {
                            if v >= 0 {
                                counts.push(json!(v))
                            }
                        }
                    } else if let Ok(v) = lexical.parse::<f64>() {
                        if v >= 0.0 {
                            let number = if v.fract() == 0.0 && v < (i64::MAX as f64) {
                                (v as i64).into()
                            } else if let Some(number) = serde_json::Number::from_f64(v) {
                                number
                            } else {
                                continue;
                            };
                            counts.push(Value::Number(number))
                        }
                    }
                }
            }
        }
    }
    insert_strings(p, "source_ids", ids);
    insert_strings(p, "source_resources", resources);
    insert_strings(p, "source_authors", authors);
    insert_strings(p, "source_last_modified", modified);
    if !counts.is_empty() {
        counts.sort_by(|a, b| a.as_f64().unwrap().total_cmp(&b.as_f64().unwrap()));
        counts.dedup_by(|a, b| a.as_f64() == b.as_f64());
        p.insert("source_usage_counts".into(), json!(counts));
    }
}
fn insert_strings(p: &mut Projection, key: &str, values: Vec<String>) {
    let values: Vec<_> = values
        .into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    if !values.is_empty() {
        p.insert(key.into(), json!(values));
    }
}
fn valid_date(value: &str) -> bool {
    NaiveDate::parse_from_str(value, "%Y-%m-%d").is_ok()
        || DateTime::parse_from_rfc3339(value).is_ok()
}
