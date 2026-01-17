// File: crates/ministore/tests/pagination_tests.rs

use ministore::{
    CursorMode, FieldSpec, Index, IndexOptions, OutputFieldSelector, RankMode, Schema, SearchOptions,
};
use serde_json::json;
use tempfile::TempDir;

fn extract_paths(page: &ministore::SearchResultPage) -> Vec<String> {
    page.items
        .iter()
        .filter_map(|v| v.get("path").and_then(|p| p.as_str()).map(|s| s.to_string()))
        .collect()
}

#[test]
fn test_pagination_after_default_fts_rank() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("fts_paging.db");

    // Schema with a text field => FTS table is created and Default rank uses BM25.
    let mut schema = Schema::new();
    schema.add_field("title", FieldSpec::text(Some(3.0)));
    schema.add_field("body", FieldSpec::text(Some(1.0)));

    let index = Index::create(&db_path, schema, IndexOptions::default()).unwrap();

    // Insert 5 docs that all match title:test
    for i in 0..5 {
        index
            .put_json(json!({
                "path": format!("/doc/{}", i),
                "title": format!("test document {}", i),
                "body": "lorem ipsum"
            }))
            .unwrap();
    }

    let query = "title:test";

    // Page 1
    let mut opts = SearchOptions {
        rank: RankMode::Default,
        limit: 2,
        after: None,
        show: OutputFieldSelector::None,
        explain: false,
        cursor_mode: CursorMode::Full,
    };

    let mut seen = std::collections::HashSet::<String>::new();
    let mut all_paths = Vec::<String>::new();

    let page1 = index.search(query, opts.clone()).unwrap();
    assert!(page1.items.len() <= 2);
    assert!(page1.next_cursor.is_some(), "expected next_cursor for first page");
    assert!(page1.has_more, "expected has_more for first page");

    for p in extract_paths(&page1) {
        assert!(seen.insert(p.clone()), "duplicate path in page1: {}", p);
        all_paths.push(p);
    }

    // Page 2
    opts.after = page1.next_cursor.clone();
    let page2 = index.search(query, opts.clone()).unwrap();

    for p in extract_paths(&page2) {
        assert!(seen.insert(p.clone()), "duplicate across pages: {}", p);
        all_paths.push(p);
    }

    // Keep paging until done (this is where score-in-WHERE used to explode)
    let mut cursor = page2.next_cursor.clone();
    let mut guard = 0;
    while let Some(c) = cursor {
        guard += 1;
        assert!(guard < 20, "paging loop guard tripped (cursor never ended)");

        opts.after = Some(c);
        let page = index.search(query, opts.clone()).unwrap();

        for p in extract_paths(&page) {
            assert!(seen.insert(p.clone()), "duplicate across pages: {}", p);
            all_paths.push(p);
        }

        cursor = page.next_cursor.clone();
        if !page.has_more {
            break;
        }
    }

    // All 5 should be returned exactly once.
    assert_eq!(seen.len(), 5);
    assert_eq!(all_paths.len(), 5);
}

#[test]
fn test_pagination_after_field_rank() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("field_rank_paging.db");

    // Schema includes a rankable number field.
    let mut schema = Schema::new();
    schema.add_field("title", FieldSpec::text(Some(1.0)));
    schema.add_field("score", FieldSpec::number(false));

    let index = Index::create(&db_path, schema, IndexOptions::default()).unwrap();

    // Insert docs with distinct scores.
    // Field rank is ORDER BY score DESC, updated_at DESC, path ASC.
    // Distinct score makes ordering deterministic regardless of timestamps.
    let docs = vec![
        ("/a", 10.0),
        ("/b", 50.0),
        ("/c", 30.0),
        ("/d", 40.0),
        ("/e", 20.0),
    ];
    for (path, score) in docs {
        index
            .put_json(json!({
                "path": path,
                "title": "test",
                "score": score
            }))
            .unwrap();
        // Not required since scores are distinct, but it helps avoid tie weirdness in future edits.
        std::thread::sleep(std::time::Duration::from_millis(2));
    }

    let query = "score>0"; // positive anchor and matches all inserted docs

    let mut opts = SearchOptions {
        rank: RankMode::Field("score".to_string()),
        limit: 2,
        after: None,
        // Show score so we can verify ordering.
        show: OutputFieldSelector::Fields(vec!["score".to_string()]),
        explain: false,
        cursor_mode: CursorMode::Full,
    };

    let mut seen = std::collections::HashSet::<String>::new();
    let mut all: Vec<(String, f64)> = Vec::new();

    let mut cursor: Option<String> = None;
    let mut guard = 0;

    loop {
        guard += 1;
        assert!(guard < 20, "paging loop guard tripped (cursor never ended)");

        opts.after = cursor.clone();
        let page = index.search(query, opts.clone()).unwrap();

        for item in &page.items {
            let path = item
                .get("path")
                .and_then(|v| v.as_str())
                .unwrap()
                .to_string();
            let score = item
                .get("score")
                .and_then(|v| v.as_f64())
                .unwrap_or_else(|| panic!("missing numeric score in output for {}", path));

            assert!(seen.insert(path.clone()), "duplicate across pages: {}", path);
            all.push((path, score));
        }

        if !page.has_more {
            break;
        }
        cursor = page.next_cursor.clone();
        assert!(cursor.is_some(), "has_more=true but next_cursor is None");
    }

    // Verify we got all 5.
    assert_eq!(seen.len(), 5);
    assert_eq!(all.len(), 5);

    // Verify overall ordering is descending score (since scores are distinct).
    let scores: Vec<f64> = all.iter().map(|(_, s)| *s).collect();
    let mut sorted = scores.clone();
    sorted.sort_by(|a, b| b.partial_cmp(a).unwrap());
    assert_eq!(scores, sorted, "results were not ordered by score desc");
}

/// Test Default non-FTS fallback (Recency ordering) pagination.
/// This exercises the CursorPayload::Recency path under RankMode::Default.
#[test]
fn test_pagination_after_default_recency_fallback() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("recency_paging.db");

    // Schema with NO text fields => no FTS table, Default falls back to Recency ordering
    let mut schema = Schema::new();
    schema.add_field("category", FieldSpec::keyword(false));
    schema.add_field("priority", FieldSpec::number(false));

    let index = Index::create(&db_path, schema, IndexOptions::default()).unwrap();

    // Insert 7 docs with distinct timestamps
    for i in 0..7 {
        index
            .put_json(json!({
                "path": format!("/item/{}", i),
                "category": "test",
                "priority": i
            }))
            .unwrap();
        // Ensure distinct updated_at timestamps
        std::thread::sleep(std::time::Duration::from_millis(5));
    }

    let query = "category:test"; // keyword anchor, no FTS involved

    let mut opts = SearchOptions {
        rank: RankMode::Default, // Will fall back to Recency since no FTS
        limit: 2,
        after: None,
        show: OutputFieldSelector::None,
        explain: false,
        cursor_mode: CursorMode::Full,
    };

    let mut seen = std::collections::HashSet::<String>::new();
    let mut all_paths = Vec::<String>::new();

    let mut cursor: Option<String> = None;
    let mut guard = 0;

    loop {
        guard += 1;
        assert!(guard < 20, "paging loop guard tripped (cursor never ended)");

        opts.after = cursor.clone();
        let page = index.search(query, opts.clone()).unwrap();

        for p in extract_paths(&page) {
            assert!(seen.insert(p.clone()), "duplicate across pages: {}", p);
            all_paths.push(p);
        }

        if !page.has_more {
            break;
        }
        cursor = page.next_cursor.clone();
        assert!(cursor.is_some(), "has_more=true but next_cursor is None");
    }

    // Verify we got all 7 docs exactly once
    assert_eq!(seen.len(), 7, "expected 7 unique docs");
    assert_eq!(all_paths.len(), 7, "expected 7 total results");

    // Verify ordering is by recency (most recent first = highest item number first)
    // Since we inserted in order with delays, /item/6 should come first, /item/0 last
    assert_eq!(all_paths[0], "/item/6", "most recent item should be first");
    assert_eq!(all_paths[6], "/item/0", "oldest item should be last");
}
