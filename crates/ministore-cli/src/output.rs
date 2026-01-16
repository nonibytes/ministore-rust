use serde_json::Value;
use ministore::SearchResultPage;

#[derive(Debug, Clone)]
pub enum OutputFormat {
    Pretty,
    Paths,
    Json,
}

impl std::str::FromStr for OutputFormat {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_lowercase().as_str() {
            "pretty" => Ok(OutputFormat::Pretty),
            "paths" => Ok(OutputFormat::Paths),
            "json" => Ok(OutputFormat::Json),
            _ => Err(format!("unknown format: {}", s)),
        }
    }
}

pub fn print_search_results(
    fmt: OutputFormat,
    page: &SearchResultPage,
    duration_ms: u128,
) {
    match fmt {
        OutputFormat::Json => {
            // Print the whole page as JSON
            // We construct a wrapper object to include metadata? 
            // Or just the SearchResultPage serialization?
            // SearchResultPage is not Serialize in my impl?
            // I should have derived Serialize for SearchResultPage in library.
            // But I can construct a value here.
            let mut obj = serde_json::Map::new();
            obj.insert("items".to_string(), Value::Array(page.items.clone()));
            if let Some(c) = &page.next_cursor {
                obj.insert("next_cursor".to_string(), Value::String(c.clone()));
            }
            if let Some(sql) = &page.explain_sql {
                obj.insert("explain_sql".to_string(), Value::String(sql.clone()));
            }
            if let Some(steps) = &page.explain_steps {
                obj.insert("explain_steps".to_string(), Value::Array(steps.iter().map(|s| Value::String(s.clone())).collect()));
            }
            // Add timing? Maybe.
            // obj.insert("duration_ms".to_string(), Value::Number(duration_ms.into()));

            let json = serde_json::to_string_pretty(&Value::Object(obj)).unwrap();
            println!("{}", json);
        }
        OutputFormat::Paths => {
            for item in &page.items {
                if let Some(path) = item.get("path").and_then(|v| v.as_str()) {
                    println!("{}", path);
                }
            }
        }
        OutputFormat::Pretty => {
            println!("Found {} items in {}ms", page.items.len(), duration_ms);
            for item in &page.items {
                // Print basic info: path + basic fields
                // If it's just "path", print it.
                // If it has other fields, print them indented?
                if let Some(path) = item.get("path").and_then(|v| v.as_str()) {
                    println!("- {}", path);
                } else {
                    println!("- (no path)");
                }
                
                // Print other fields if verbose check?
                // For now just iterate keys
                if let Some(obj) = item.as_object() {
                    for (k, v) in obj {
                        if k != "path" {
                             println!("  {}: {}", k, v);
                        }
                    }
                }
            }
            
            if let Some(cursor) = &page.next_cursor {
                println!("\nNext cursor: {}", cursor);
            }
            
            if let Some(steps) = &page.explain_steps {
                println!("\nExplanation:");
                for step in steps {
                    println!("  {}", step);
                }
            }
             if let Some(sql) = &page.explain_sql {
                 println!("\nSQL: {}", sql);
             }
        }
    }
}

pub fn print_json(v: &Value) {
    println!("{}", serde_json::to_string_pretty(v).unwrap());
}

#[allow(dead_code)]
pub fn print_error(e: &ministore::MinistoreError) {
    eprintln!("Error: {}", e);
}
