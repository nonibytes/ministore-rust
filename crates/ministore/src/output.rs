use crate::{MinistoreError, Result, SearchResultPage};
use serde_json::Value;
use std::fmt::Write;
use std::time::Duration;

/// Representation produced by [`format_search_results`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchOutputFormat {
    Pretty,
    Paths,
    Json,
}

impl std::str::FromStr for SearchOutputFormat {
    type Err = MinistoreError;

    fn from_str(value: &str) -> Result<Self> {
        match value.to_ascii_lowercase().as_str() {
            "" | "pretty" => Ok(Self::Pretty),
            "paths" => Ok(Self::Paths),
            "json" => Ok(Self::Json),
            _ => Err(MinistoreError::InvalidArgument(format!(
                "unknown search output format {value:?}; expected pretty, paths, or json"
            ))),
        }
    }
}

/// Options for rendering a search page.
#[derive(Debug, Clone)]
pub struct SearchOutputOptions {
    pub format: SearchOutputFormat,
    pub elapsed: Option<Duration>,
}

impl Default for SearchOutputOptions {
    fn default() -> Self {
        Self {
            format: SearchOutputFormat::Pretty,
            elapsed: None,
        }
    }
}

/// Render a search page for terminal or machine output.
///
/// Pretty and paths are human-readable; JSON has a stable page envelope.
pub fn format_search_results(
    page: &SearchResultPage,
    options: &SearchOutputOptions,
) -> Result<String> {
    match options.format {
        SearchOutputFormat::Pretty => format_pretty(page, options.elapsed),
        SearchOutputFormat::Paths => Ok(format_paths(page)),
        SearchOutputFormat::Json => format_json(page),
    }
}

fn format_pretty(page: &SearchResultPage, elapsed: Option<Duration>) -> Result<String> {
    let mut output = String::new();
    write!(&mut output, "Found {} items", page.items.len()).unwrap();
    if let Some(elapsed) = elapsed {
        write!(&mut output, " in {}ms", elapsed.as_millis()).unwrap();
    }
    output.push('\n');

    for item in &page.items {
        let path = item
            .get("path")
            .and_then(Value::as_str)
            .unwrap_or("(no path)");
        writeln!(&mut output, "- {path}").unwrap();
        if let Some(fields) = item.as_object() {
            for (name, value) in fields {
                if name != "path" {
                    writeln!(&mut output, "  {name}: {}", human_value(value)?).unwrap();
                }
            }
        }
    }

    if let Some(cursor) = &page.next_cursor {
        writeln!(&mut output, "\nNext cursor: {cursor}").unwrap();
    } else if page.has_more {
        output.push_str("\nMore results available\n");
    }
    if let Some(steps) = &page.explain_steps {
        if !steps.is_empty() {
            output.push_str("\nExplanation:\n");
            for step in steps {
                writeln!(&mut output, "  {step}").unwrap();
            }
        }
    }
    if let Some(sql) = &page.explain_sql {
        writeln!(&mut output, "\nSQL: {sql}").unwrap();
    }

    Ok(output)
}

fn format_paths(page: &SearchResultPage) -> String {
    let mut output = String::new();
    for path in page
        .items
        .iter()
        .filter_map(|item| item.get("path").and_then(Value::as_str))
    {
        writeln!(&mut output, "{path}").unwrap();
    }
    output
}

fn format_json(page: &SearchResultPage) -> Result<String> {
    let mut envelope = serde_json::Map::new();
    envelope.insert("items".into(), Value::Array(page.items.clone()));
    if let Some(cursor) = &page.next_cursor {
        envelope.insert("next_cursor".into(), Value::String(cursor.clone()));
    }
    envelope.insert("has_more".into(), Value::Bool(page.has_more));
    if let Some(sql) = &page.explain_sql {
        envelope.insert("explain_sql".into(), Value::String(sql.clone()));
    }
    if let Some(steps) = &page.explain_steps {
        envelope.insert(
            "explain_steps".into(),
            Value::Array(steps.iter().cloned().map(Value::String).collect()),
        );
    }
    Ok(serde_json::to_string_pretty(&Value::Object(envelope))? + "\n")
}

fn human_value(value: &Value) -> Result<String> {
    match value {
        Value::String(value) => Ok(value.clone()),
        _ => Ok(serde_json::to_string(value)?),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn sample_page() -> SearchResultPage {
        SearchResultPage {
            items: vec![
                json!({
                    "path": "/guides/search",
                    "category": "guide",
                    "priority": 10,
                    "title": "Fast search"
                }),
                json!({"path": "/notes/sqlite", "title": "SQLite notes"}),
            ],
            next_cursor: Some("c:next".into()),
            explain_sql: None,
            explain_steps: None,
            has_more: true,
        }
    }

    #[test]
    fn formats_pretty_output() {
        let formatted = format_search_results(
            &sample_page(),
            &SearchOutputOptions {
                format: SearchOutputFormat::Pretty,
                elapsed: Some(Duration::from_millis(7)),
            },
        )
        .unwrap();
        assert_eq!(
            formatted,
            concat!(
                "Found 2 items in 7ms\n",
                "- /guides/search\n",
                "  category: guide\n",
                "  priority: 10\n",
                "  title: Fast search\n",
                "- /notes/sqlite\n",
                "  title: SQLite notes\n",
                "\nNext cursor: c:next\n",
            )
        );
    }

    #[test]
    fn formats_paths_output() {
        let formatted = format_search_results(
            &sample_page(),
            &SearchOutputOptions {
                format: SearchOutputFormat::Paths,
                elapsed: None,
            },
        )
        .unwrap();
        assert_eq!(formatted, "/guides/search\n/notes/sqlite\n");
    }

    #[test]
    fn formats_json_envelope() {
        let formatted = format_search_results(
            &sample_page(),
            &SearchOutputOptions {
                format: SearchOutputFormat::Json,
                elapsed: None,
            },
        )
        .unwrap();
        let value: Value = serde_json::from_str(&formatted).unwrap();
        assert_eq!(value["items"].as_array().unwrap().len(), 2);
        assert_eq!(value["next_cursor"], "c:next");
        assert_eq!(value["has_more"], true);
    }

    #[test]
    fn rejects_unknown_format() {
        let error = "yaml".parse::<SearchOutputFormat>().unwrap_err();
        assert!(error
            .to_string()
            .contains("expected pretty, paths, or json"));
    }
}
