Below are **targeted patches** to finish/repair the implementation you posted. Each patch includes:

1. **What’s wrong / what’s missing**
2. **Exactly what to change in each file**
3. A **unified diff** you can apply

I’m focusing on correctness + bringing behavior in line with the spec you gave earlier (paths/dates unquoted, date equality, date ranges, cursor modes, JSONL import, implicit fields in stats, etc.).

---

## Patch 1 — Fix query lexer so `/docs/*` and `2024-01-01` work unquoted

### Problem

Your lexer only recognizes `Ident` tokens starting with `[A-Za-z_]`. That breaks:

* `path:/docs/*` (starts with `/`)
* `due:2024-01-01` (starts with digit and includes `-`)
* `due:2024-01-01..2024-06-30` (range needs `..` to be tokenized correctly in the middle)

### Fix

Replace the lexer’s “identifier or keyword” branch with a generic “word token” scanner that:

* reads until whitespace or an operator boundary
* stops before `..` so ranges work
* classifies the word as `Number` only if it matches a pure numeric pattern
* otherwise produces `Ident(word)`

### Patch

```diff
*** Begin Patch
*** Update File: crates/ministore/src/query/lexer.rs
@@
 use crate::{MinistoreError, Result};
 
 #[derive(Debug, Clone, PartialEq)]
 pub enum Tok {
@@
 }
 
 /// Tokenize a query string.
 pub fn lex(input: &str) -> Result<Vec<Tok>> {
     let mut tokens = Vec::new();
     let chars: Vec<char> = input.chars().collect();
     let mut i = 0;
 
+    fn is_op_boundary(c: char) -> bool {
+        matches!(c, ':' | '>' | '<' | '!' | '(' | ')' | '&' | '|' )
+    }
+
+    fn looks_like_number(s: &str) -> bool {
+        // strict numeric: -?\d+(\.\d+)?
+        if s.is_empty() { return false; }
+        let mut it = s.chars().peekable();
+        if it.peek() == Some(&'-') { it.next(); }
+        let mut saw_digit = false;
+        while let Some(&c) = it.peek() {
+            if c.is_ascii_digit() {
+                saw_digit = true;
+                it.next();
+            } else {
+                break;
+            }
+        }
+        if !saw_digit { return false; }
+        if it.peek() == Some(&'.') {
+            it.next();
+            let mut saw_frac = false;
+            while let Some(&c) = it.peek() {
+                if c.is_ascii_digit() {
+                    saw_frac = true;
+                    it.next();
+                } else {
+                    break;
+                }
+            }
+            if !saw_frac { return false; }
+        }
+        it.next().is_none()
+    }
+
     while i < chars.len() {
         // Skip whitespace
         if chars[i].is_whitespace() {
             i += 1;
             continue;
         }
@@
         // String literal (quoted)
         if chars[i] == '"' {
@@
             continue;
         }
 
-        // Number
-        if chars[i].is_ascii_digit() || (chars[i] == '-' && i + 1 < chars.len() && chars[i + 1].is_ascii_digit()) {
-            let start = i;
-            if chars[i] == '-' {
-                i += 1;
-            }
-            let mut has_dot = false;
-            while i < chars.len() {
-                if chars[i].is_ascii_digit() {
-                    i += 1;
-                } else if chars[i] == '.' && !has_dot {
-                    // Check if this is .. (range operator)
-                    if i + 1 < chars.len() && chars[i + 1] == '.' {
-                        break; // Stop before ..
-                    }
-                    has_dot = true;
-                    i += 1;
-                } else {
-                    break;
-                }
-            }
-            let num_str: String = chars[start..i].iter().collect();
-            let num = num_str.parse::<f64>()
-                .map_err(|_| MinistoreError::QueryParse(format!("invalid number: {}", num_str)))?;
-            tokens.push(Tok::Number(num));
-            continue;
-        }
-
-        // Identifier or keyword
-        if chars[i].is_alphabetic() || chars[i] == '_' {
-            let start = i;
-            while i < chars.len() && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '*' || chars[i] == '?') {
-                i += 1;
-            }
-            let ident: String = chars[start..i].iter().collect();
-            
-            // Check for keywords (case-insensitive)
-            match ident.to_uppercase().as_str() {
-                "AND" => tokens.push(Tok::And),
-                "OR" => tokens.push(Tok::Or),
-                "NOT" => tokens.push(Tok::Not),
-                _ => tokens.push(Tok::Ident(ident)),
-            }
-            continue;
-        }
+        // Word token (covers identifiers, paths, dates, wildcard patterns, etc.)
+        // Reads until whitespace or operator boundary, but stops before ".." so ranges tokenize properly.
+        let start = i;
+        while i < chars.len() {
+            if chars[i].is_whitespace() || is_op_boundary(chars[i]) {
+                break;
+            }
+            if chars[i] == '.' && i + 1 < chars.len() && chars[i + 1] == '.' {
+                break; // stop before range operator
+            }
+            i += 1;
+        }
+        if start == i {
+            return Err(MinistoreError::QueryParse(format!("unexpected character: {}", chars[i])));
+        }
+        let word: String = chars[start..i].iter().collect();
+
+        // Keywords (case-insensitive)
+        match word.to_uppercase().as_str() {
+            "AND" => { tokens.push(Tok::And); continue; }
+            "OR" => { tokens.push(Tok::Or); continue; }
+            "NOT" => { tokens.push(Tok::Not); continue; }
+            _ => {}
+        }
+
+        if looks_like_number(&word) {
+            let num = word.parse::<f64>()
+                .map_err(|_| MinistoreError::QueryParse(format!("invalid number: {}", word)))?;
+            tokens.push(Tok::Number(num));
+        } else {
+            tokens.push(Tok::Ident(word));
+        }
+        continue;
 
         // If we get here, unrecognized character
-        return Err(MinistoreError::QueryParse(format!("unexpected character: {}", chars[i])));
+        // (Should be unreachable due to word scanner above)
     }
 
     Ok(tokens)
 }
*** End Patch
```

---

## Patch 2 — Support `field:date1..date2` (date ranges with `..`)

### Problem

`due:2024-01-01..2024-06-30` currently fails because the parser doesn’t handle `DotDot` after string/ident values in `field:...`.

### Fix

Extend `parse_field_predicate()` so that when it sees `field:<string-or-ident> .. <string-or-ident>` it emits:

* `Predicate::DateRangeAbs { field, lo_ms, hi_ms }` if the endpoints parse as dates

This is schema-agnostic at parse time; the planner already verifies the field is actually a date field (or we patch it to also support implicit created/updated, see Patch 4).

### Patch

```diff
*** Begin Patch
*** Update File: crates/ministore/src/query/parser.rs
@@
     fn parse_field_predicate(&mut self, field: &str) -> Result<Predicate> {
@@
         // Get value
         match self.current() {
             Some(Tok::String(s)) | Some(Tok::Ident(s)) => {
                 let value = s.clone();
                 self.advance();
+
+                // Support date ranges: field:2024-01-01..2024-06-30
+                // (This will later be validated by planner against schema type.)
+                if self.match_tok(&Tok::DotDot) {
+                    self.advance();
+                    let hi_s = self.expect_string_or_ident()?;
+                    let lo_ms = parse_date_to_epoch_ms(&value)?;
+                    let hi_ms = parse_date_to_epoch_ms(&hi_s)?;
+                    return Ok(Predicate::DateRangeAbs {
+                        field: field.to_string(),
+                        lo_ms,
+                        hi_ms,
+                    });
+                }
 
                 // NOTE: We do NOT decide keyword vs text here.
                 // Planner will look at schema type:
@@
                 let kind = classify_keyword_pattern(&value);
                 Ok(Predicate::Keyword {
                     field: field.to_string(),
                     pattern: value,
                     kind,
                 })
             }
*** End Patch
```

---

## Patch 3 — Fix query normalization: `Glob` patterns with no literal prefix should be rejected

### Problem

Your `normalize` guardrails don’t reject `tags:*a?*`-style globs with no literal prefix (high-cost scans). For `PathGlob` you do reject patterns without a literal prefix, but for keyword glob patterns you allow them.

### Fix

Add validation for `KeywordPatternKind::Glob`:

* if it contains `*` or `?`, require a literal prefix before first wildcard
* enforce `MIN_PREFIX_LEN` on that prefix

### Patch

```diff
*** Begin Patch
*** Update File: crates/ministore/src/query/normalize.rs
@@
 fn validate_predicate_pattern(pred: &Predicate) -> Result<()> {
     match pred {
         Predicate::Keyword { pattern, kind, .. } => {
             match kind {
@@
                 KeywordPatternKind::Contains => {
@@
                 }
-                _ => {} // Exact and Glob don't need validation
+                KeywordPatternKind::Glob => {
+                    // Require a literal prefix before any wildcard (performance guardrail).
+                    if pattern.contains(&['*', '?'][..]) {
+                        if let Some(pos) = pattern.find(&['*', '?'][..]) {
+                            let prefix = &pattern[..pos];
+                            if prefix.is_empty() {
+                                return Err(MinistoreError::QueryRejected(
+                                    "glob pattern must have a literal prefix before wildcards".into()
+                                ));
+                            }
+                            if prefix.len() < MIN_PREFIX_LEN {
+                                return Err(MinistoreError::QueryRejected(
+                                    format!("glob prefix '{}' too short (minimum {} characters)", prefix, MIN_PREFIX_LEN)
+                                ));
+                            }
+                        }
+                    }
+                }
+                _ => {} // Exact doesn't need validation
             }
         }
*** End Patch
```

---

## Patch 4 — Planner fixes: date equality for date fields + implicit created/updated date ranges

### Problems

1. `due:2024-01-15` is parsed as a `Keyword` predicate and then rejected for date fields (planner currently only treats dates via comparisons or DateCmpAbs/Rel).
2. `created:2024-01-01..2024-02-01` (implicit fields) fails because `DateRangeAbs` only supports schema date fields.
3. If schema has no text fields, planner still allows bare text parsing but later fails confusingly; normalize already rejects pure-negative, but bare text should still work only when schema has text fields (you already handle that).

### Fix

* In `Predicate::Keyword` compilation:

  * if the field is an **implicit** `created`/`updated` and pattern is **exact**, parse as date equality and compile against `items.created_at`/`items.updated_at`
  * if schema type is **Date** and pattern is **exact**, parse ISO date/datetime and compile `field_date` equality
  * reject wildcard patterns for date fields
* In `Predicate::DateRangeAbs` compilation:

  * support implicit `created`/`updated` by compiling against `items.created_at/items.updated_at`

### Patch

```diff
*** Begin Patch
*** Update File: crates/ministore/src/query/planner.rs
@@
 use crate::{Result, Schema, MinistoreError, FieldType};
 use crate::query::ast::*;
 use crate::index::RankMode;
+use chrono::{NaiveDate, DateTime};
+
+fn parse_date_to_epoch_ms(s: &str) -> Result<i64> {
+    if let Ok(date) = NaiveDate::parse_from_str(s, "%Y-%m-%d") {
+        return Ok(date.and_hms_opt(0, 0, 0).unwrap().and_utc().timestamp_millis());
+    }
+    if let Ok(dt) = DateTime::parse_from_rfc3339(s) {
+        return Ok(dt.timestamp_millis());
+    }
+    Err(MinistoreError::QueryParse(format!("invalid date format: {}", s)))
+}
@@
             Predicate::Keyword { field, pattern, kind } => {
-                let spec = self.schema.get(&field).ok_or_else(|| MinistoreError::UnknownField(field.clone()))?;
+                // Implicit created/updated support for equality: created:2024-01-01
+                if (field == "created" || field == "updated") {
+                    if kind != KeywordPatternKind::Exact {
+                        return Err(MinistoreError::TypeMismatch {
+                            field,
+                            message: "wildcards not supported for implicit date fields".into(),
+                        });
+                    }
+                    let epoch_ms = parse_date_to_epoch_ms(&pattern)?;
+                    let result_name = self.next_cte_name();
+                    let col = if field == "created" { "created_at" } else { "updated_at" };
+                    let p = self.push_param(epoch_ms.into());
+                    let sql = format!("SELECT id AS item_id FROM items WHERE {} = {}", col, p);
+                    self.ctes.push(Cte { name: result_name.clone(), sql });
+                    self.explain.push(format!("IMPLICIT DATE {}:{}", field, pattern));
+                    return Ok(result_name);
+                }
+
+                let spec = self.schema.get(&field).ok_or_else(|| MinistoreError::UnknownField(field.clone()))?;
 
                 // If schema says this is a TEXT field, treat field:term as FTS query.
                 if spec.field_type == FieldType::Text {
                     return self.compile_predicate(Predicate::Text { field: Some(field), fts: pattern });
                 }
                 // Bool fields: accept true/false via field:...
                 if spec.field_type == FieldType::Bool && (pattern == "true" || pattern == "false") {
                     return self.compile_predicate(Predicate::Bool { field, value: pattern == "true" });
                 }
+                // Date fields: support equality via field:YYYY-MM-DD (exact only)
+                if spec.field_type == FieldType::Date {
+                    if kind != KeywordPatternKind::Exact {
+                        return Err(MinistoreError::TypeMismatch {
+                            field,
+                            message: "wildcards not supported for date fields; use comparisons".into(),
+                        });
+                    }
+                    let epoch_ms = parse_date_to_epoch_ms(&pattern)?;
+                    return self.compile_predicate(Predicate::DateCmpAbs {
+                        field,
+                        op: CmpOp::Eq,
+                        epoch_ms,
+                    });
+                }
                 if spec.field_type != FieldType::Keyword {
                     return Err(MinistoreError::TypeMismatch {
                         field: field.clone(),
                         message: format!("predicate is keyword-style but schema type is {:?}", spec.field_type),
                     });
                 }
@@
             Predicate::DateRangeAbs { field, lo_ms, hi_ms } => {
-                let spec = self.schema.get(&field).ok_or_else(|| MinistoreError::UnknownField(field.clone()))?;
-                if spec.field_type != FieldType::Date {
-                    return Err(MinistoreError::TypeMismatch { field: field.clone(), message: format!("expected date field, got {:?}", spec.field_type) });
-                }
-                let result_name = self.next_cte_name();
-                let p_field = self.push_param(field.clone().into());
-                let p_lo = self.push_param(lo_ms.into());
-                let p_hi = self.push_param(hi_ms.into());
-                let sql = format!("SELECT item_id FROM field_date WHERE field = {} AND value >= {} AND value <= {}", p_field, p_lo, p_hi);
-
-                self.ctes.push(Cte {
-                    name: result_name.clone(),
-                    sql,
-                });
-                self.explain.push(format!("DATE {}:{}..{}", field, lo_ms, hi_ms));
-                Ok(result_name)
+                // implicit created/updated ranges compile to items table
+                if field == "created" || field == "updated" {
+                    let result_name = self.next_cte_name();
+                    let col = if field == "created" { "created_at" } else { "updated_at" };
+                    let p_lo = self.push_param(lo_ms.into());
+                    let p_hi = self.push_param(hi_ms.into());
+                    let sql = format!(
+                        "SELECT id AS item_id FROM items WHERE {} >= {} AND {} <= {}",
+                        col, p_lo, col, p_hi
+                    );
+                    self.ctes.push(Cte { name: result_name.clone(), sql });
+                    self.explain.push(format!("IMPLICIT DATE RANGE {}:{}..{}", field, lo_ms, hi_ms));
+                    return Ok(result_name);
+                }
+
+                let spec = self.schema.get(&field).ok_or_else(|| MinistoreError::UnknownField(field.clone()))?;
+                if spec.field_type != FieldType::Date {
+                    return Err(MinistoreError::TypeMismatch { field: field.clone(), message: format!("expected date field, got {:?}", spec.field_type) });
+                }
+                let result_name = self.next_cte_name();
+                let p_field = self.push_param(field.clone().into());
+                let p_lo = self.push_param(lo_ms.into());
+                let p_hi = self.push_param(hi_ms.into());
+                let sql = format!("SELECT item_id FROM field_date WHERE field = {} AND value >= {} AND value <= {}", p_field, p_lo, p_hi);
+
+                self.ctes.push(Cte {
+                    name: result_name.clone(),
+                    sql,
+                });
+                self.explain.push(format!("DATE {}:{}..{}", field, lo_ms, hi_ms));
+                Ok(result_name)
             }
*** End Patch
```

---

## Patch 5 — Make FTS optional when schema has no text fields + use `content=''` mode

### Problems

1. `verify::build_fts_ddl` **errors** if schema has no text fields → prevents keyword-only indexes.
2. You currently create FTS without `content=''`. Your write-path is already “manual sync” (insert/delete by rowid), which matches contentless FTS best.

### Fix

* `build_fts_ddl(schema) -> Result<Option<String>>`
* If no text fields: return `Ok(None)`
* If text fields: create FTS with `content=''`
* `Index::create` and `Index::open` should only require/verify FTS when schema has text fields.

### Patch

```diff
*** Begin Patch
*** Update File: crates/ministore/src/db/verify.rs
@@
 use rusqlite::Connection;
 use crate::{MinistoreError, Result, Schema};
@@
 /// Build CREATE VIRTUAL TABLE search USING fts5(...) statement from schema.
-pub fn build_fts_ddl(schema: &Schema) -> Result<String> {
+pub fn build_fts_ddl(schema: &Schema) -> Result<Option<String>> {
     let text_fields = schema.text_fields_in_order();
     
     if text_fields.is_empty() {
-        return Err(MinistoreError::Schema(
-            "schema must have at least one text field for FTS".into()
-        ));
+        return Ok(None);
     }
     
     let columns: Vec<String> = text_fields.iter().map(|(name, _)| name.clone()).collect();
     let columns_list = columns.join(", ");
     
-    // Standard FTS5 - not using content='' due to corruption issues with delete command
-    Ok(format!(
-        "CREATE VIRTUAL TABLE IF NOT EXISTS search USING fts5({}, tokenize='unicode61')",
-        columns_list
-    ))
+    Ok(Some(format!(
+        "CREATE VIRTUAL TABLE IF NOT EXISTS search USING fts5({}, content='', tokenize='unicode61')",
+        columns_list
+    )))
 }
@@
 pub fn verify_fts_columns(conn: &Connection, schema: &Schema) -> Result<()> {
     let expected = schema.text_fields_in_order();
     let expected_names: Vec<String> = expected.iter().map(|(name, _)| name.clone()).collect();
+
+    // If schema has no text fields, FTS is not required.
+    if expected_names.is_empty() {
+        return Ok(());
+    }
     
     // Query FTS table columns
     let mut stmt = conn.prepare("PRAGMA table_info(search)")?;
@@
     if actual_names != expected_names {
@@
     Ok(())
 }
@@
 #[cfg(test)]
 mod tests {
@@
     fn test_build_fts_ddl() {
@@
-        let ddl = build_fts_ddl(&schema).unwrap();
+        let ddl = build_fts_ddl(&schema).unwrap().unwrap();
         assert!(ddl.contains("CREATE VIRTUAL TABLE"));
         assert!(ddl.contains("content, title")); // BTreeMap sorts alphabetically
         assert!(ddl.contains("fts5"));
     }
@@
     fn test_build_fts_ddl_no_text_fields() {
         let mut schema = Schema::new();
         schema.add_field("tags", FieldSpec::keyword(true));
         
-        assert!(build_fts_ddl(&schema).is_err());
+        assert!(build_fts_ddl(&schema).unwrap().is_none());
     }
@@
     fn test_verify_fts_columns() {
@@
-        let ddl = build_fts_ddl(&schema).unwrap();
-        conn.execute(&ddl, []).unwrap();
+        let ddl = build_fts_ddl(&schema).unwrap().unwrap();
+        conn.execute(&ddl, []).unwrap();
@@
     }
 }
*** End Patch
```

Now update `Index::create` and `Index::open` accordingly:

```diff
*** Begin Patch
*** Update File: crates/ministore/src/index.rs
@@
     pub fn create<P: AsRef<Path>>(path: P, schema: Schema, opts: IndexOptions) -> Result<Self> {
         use crate::db::{ddl, meta, verify};
@@
-        // Require FTS5
-        verify::require_fts5(&conn)?;
+        // Require FTS5 only if schema uses text fields
+        if !schema.text_fields_in_order().is_empty() {
+            verify::require_fts5(&conn)?;
+        }
@@
-        // Create FTS virtual table
-        let fts_ddl = verify::build_fts_ddl(&schema)?;
-        conn.execute(&fts_ddl, [])?;
+        // Create FTS virtual table (optional)
+        if let Some(fts_ddl) = verify::build_fts_ddl(&schema)? {
+            conn.execute(&fts_ddl, [])?;
+        }
@@
     pub fn open<P: AsRef<Path>>(path: P, opts: IndexOptions) -> Result<Self> {
         use crate::db::{meta, verify};
@@
-        // Verify FTS5 available
-        verify::require_fts5(&conn)?;
-        
-        // Verify FTS columns match schema
-        verify::verify_fts_columns(&conn, &schema)?;
+        // Verify FTS only if schema has text fields
+        if !schema.text_fields_in_order().is_empty() {
+            verify::require_fts5(&conn)?;
+            verify::verify_fts_columns(&conn, &schema)?;
+        }
*** End Patch
```

---

## Patch 6 — Don’t crash delete/put if `search` table is absent (keyword-only schemas)

### Problem

`delete_by_item_id` always executes `DELETE FROM search...`. If FTS isn’t created (keyword-only schema), delete fails.

### Fix

Ignore “no such table: search” errors.

### Patch (delete)

```diff
*** Begin Patch
*** Update File: crates/ministore/src/db/delete.rs
@@
 use rusqlite::Transaction;
 use crate::Result;
 use super::sql::*;
 
+fn is_no_such_table_search(e: &rusqlite::Error) -> bool {
+    match e {
+        rusqlite::Error::SqliteFailure(err, Some(msg)) => {
+            err.code == rusqlite::ErrorCode::Unknown && msg.contains("no such table: search")
+        }
+        _ => false,
+    }
+}
+
 /// Delete an item by its internal ID, adjusting doc_freq counts.
 pub fn delete_by_item_id(tx: &Transaction, item_id: i64) -> Result<()> {
@@
     tx.execute(SQL_DELETE_BOOL_BY_ITEM, [item_id])?;
     tx.execute(SQL_DELETE_PRESENT_BY_ITEM, [item_id])?;
-    tx.execute(SQL_DELETE_SEARCH_ROW, [item_id])?;
+    if let Err(e) = tx.execute(SQL_DELETE_SEARCH_ROW, [item_id]) {
+        if !is_no_such_table_search(&e) {
+            return Err(e.into());
+        }
+    }
     tx.execute(SQL_DELETE_ITEMS_BY_ID, [item_id])?;
     
     Ok(())
 }
*** End Patch
```

### Patch (put)

Same idea in `execute_put_internal` around `SQL_DELETE_SEARCH_ROW` and the FTS insert:

```diff
*** Begin Patch
*** Update File: crates/ministore/src/db/put.rs
@@
 fn execute_put_internal(
@@
 ) -> Result<()> {
     use super::sql::*;
+    fn is_no_such_table_search(e: &rusqlite::Error) -> bool {
+        match e {
+            rusqlite::Error::SqliteFailure(err, Some(msg)) => {
+                err.code == rusqlite::ErrorCode::Unknown && msg.contains("no such table: search")
+            }
+            _ => false,
+        }
+    }
@@
-    tx.execute(SQL_DELETE_SEARCH_ROW, [item_id])?;
+    if let Err(e) = tx.execute(SQL_DELETE_SEARCH_ROW, [item_id]) {
+        if !is_no_such_table_search(&e) {
+            return Err(e.into());
+        }
+    }
@@
     // Insert FTS row (if any text fields)
     let text_fields = schema.text_fields_in_order();
     if !text_fields.is_empty() {
@@
-        tx.execute(&sql, rusqlite::params_from_iter(params))?;
+        if let Err(e) = tx.execute(&sql, rusqlite::params_from_iter(params)) {
+            if !is_no_such_table_search(&e) {
+                return Err(e.into());
+            }
+        }
     }
*** End Patch
```

---

## Patch 7 — Fix `SQL_DECREMENT_DOC_FREQ` to never go negative (defensive)

### Problem

Repeated deletes or inconsistent state could push `doc_freq` negative.

### Patch

```diff
*** Begin Patch
*** Update File: crates/ministore/src/db/sql.rs
@@
-pub const SQL_DECREMENT_DOC_FREQ: &str = "UPDATE kw_dict SET doc_freq = doc_freq - 1 WHERE id = ?1";
+pub const SQL_DECREMENT_DOC_FREQ: &str =
+    "UPDATE kw_dict SET doc_freq = CASE WHEN doc_freq > 0 THEN doc_freq - 1 ELSE 0 END WHERE id = ?1";
*** End Patch
```

---

## Patch 8 — Stats: support implicit `created`/`updated` fields and scoped queries

### Problem

Spec supports: `ministore stats --field created -w 'tags:memory'` etc. Your stats module rejects implicit fields because they’re not in schema.

### Fix

* If field is `created` or `updated`, query `items.created_at/updated_at`.
* In scoped mode, join against the compiled `result` CTE.

### Patch

```diff
*** Begin Patch
*** Update File: crates/ministore/src/stats.rs
@@
 pub fn compute_stats(
     conn: &Connection,
     schema: &Schema,
     field: &str,
     scoped_query: Option<&str>,
 ) -> Result<Value> {
+    // Support implicit fields
+    if field == "created" || field == "updated" {
+        let col = if field == "created" { "created_at" } else { "updated_at" };
+        let (count, min, max, avg): (i64, Option<f64>, Option<f64>, Option<f64>) = if let Some(query) = scoped_query {
+            if !query.trim().is_empty() {
+                let expr = parse_query(query)?;
+                let normalized = normalize::normalize(expr)?;
+                let compiled = compile_to_ctes(schema, normalized, now_ms())?;
+                let ctes_sql: Vec<String> = compiled.ctes.iter()
+                    .map(|c| format!("{} AS ({})", c.name, c.sql))
+                    .collect();
+                let with_clause = if ctes_sql.is_empty() { String::new() } else { format!("WITH {} ", ctes_sql.join(", ")) };
+                let sql = format!(
+                    "{with_clause}
+                     SELECT COUNT(*), MIN(i.{col}), MAX(i.{col}), AVG(i.{col})
+                     FROM {result} r
+                     JOIN items i ON i.id = r.item_id",
+                    with_clause = with_clause,
+                    col = col,
+                    result = compiled.result_cte_name
+                );
+                conn.query_row(&sql, rusqlite::params_from_iter(compiled.params), |row| {
+                    Ok((row.get(0)?, row.get(1).ok(), row.get(2).ok(), row.get(3).ok()))
+                })?
+            } else {
+                let sql = format!("SELECT COUNT(*), MIN({col}), MAX({col}), AVG({col}) FROM items", col = col);
+                conn.query_row(&sql, [], |row| {
+                    Ok((row.get(0)?, row.get(1).ok(), row.get(2).ok(), row.get(3).ok()))
+                })?
+            }
+        } else {
+            let sql = format!("SELECT COUNT(*), MIN({col}), MAX({col}), AVG({col}) FROM items", col = col);
+            conn.query_row(&sql, [], |row| {
+                Ok((row.get(0)?, row.get(1).ok(), row.get(2).ok(), row.get(3).ok()))
+            })?
+        };
+
+        let mut obj = serde_json::Map::new();
+        obj.insert("field".to_string(), Value::String(field.to_string()));
+        obj.insert("count".to_string(), Value::Number(count.into()));
+        if let Some(min_val) = min {
+            obj.insert("min".to_string(), Value::Number(serde_json::Number::from_f64(min_val).unwrap()));
+        }
+        if let Some(max_val) = max {
+            obj.insert("max".to_string(), Value::Number(serde_json::Number::from_f64(max_val).unwrap()));
+        }
+        if let Some(avg_val) = avg {
+            obj.insert("avg".to_string(), Value::Number(serde_json::Number::from_f64(avg_val).unwrap()));
+        }
+        return Ok(Value::Object(obj));
+    }
+
     // Validate field exists
     if !schema.fields.contains_key(field) {
         return Err(crate::MinistoreError::UnknownField(field.to_string()));
     }
*** End Patch
```

---

## Patch 9 — Fix CLI `put --import` to support JSONL + parse `--set` based on schema (multi fields)

### Problems

1. Spec says `--import` is JSONL, but CLI reads it as a single JSON value.
2. `--set tags=a,b,c` should become an array **only if the schema says it’s multi** (keyword/number/date).
3. Your current `serde_json::from_reader(...).map_err(|e| MinistoreError::Io(e.into()))` is incorrect error mapping (`serde_json::Error` is not `std::io::Error`).

### Fix

* For `--import <file>`: read line-by-line JSON objects, batch them.
* For `--set`: look up field in schema:

  * if `multi=true` and value contains commas -> JSON array of strings
  * else keep as string
* For stdin `--json`: keep existing behavior (object or array).

### Patch

```diff
*** Begin Patch
*** Update File: crates/ministore-cli/src/commands/put.rs
@@
 use clap::Args;
 use ministore::{Result, Index, IndexOptions, Batch};
 use crate::resolve::resolve_index_path;
 use serde_json::Value;
-use std::io::Read;
+use std::io::{Read, BufRead};
 use std::fs::File;
 
@@
 pub fn run(args: PutArgs) -> Result<()> {
     let index_path = resolve_index_path(&args.index)?;
     let index = Index::open(&index_path, IndexOptions::default())?;
 
     if let Some(path) = &args.path {
         // Path + sets mode
         let mut obj = serde_json::Map::new();
         for set in &args.sets {
             if let Some((k, v)) = set.split_once('=') {
-                // Try to guess type? For now treat as string or bool/number if obvious?
-                // V1 usually keeps simple. Strings are safe.
-                // Schema coercion handles types.
-                obj.insert(k.to_string(), Value::String(v.to_string()));
+                // Use schema to decide whether to split CSV for multi-valued fields.
+                if let Some(spec) = index.schema().get(k) {
+                    if spec.multi && v.contains(',') {
+                        let parts: Vec<Value> = v.split(',')
+                            .map(|s| Value::String(s.trim().to_string()))
+                            .filter(|s| !s.as_str().unwrap_or("").is_empty())
+                            .collect();
+                        obj.insert(k.to_string(), Value::Array(parts));
+                    } else {
+                        obj.insert(k.to_string(), Value::String(v.to_string()));
+                    }
+                } else {
+                    // unknown field: pass as string, library will reject if schema is strict
+                    obj.insert(k.to_string(), Value::String(v.to_string()));
+                }
             } else {
                 // key only? maybe bool true?
                 obj.insert(set.to_string(), Value::Bool(true));
             }
         }
         index.put_fields(path, Value::Object(obj))?;
         println!("Put {}", path);
     } else {
-        // Import mode (stdin or file)
-        let reader: Box<dyn Read> = if let Some(path) = &args.import {
-            Box::new(File::open(path)?)
-        } else if args.json { // reading from stdin explicitly requested or implied?
-             Box::new(std::io::stdin())
-        } else {
-             return Err(ministore::MinistoreError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, "Must provide --path or --json/--import")));
-        };
-        
-        // Parse JSON
-        let val: Value = serde_json::from_reader(reader).map_err(|e| ministore::MinistoreError::Io(e.into()))?;
-        
-        if let Some(arr) = val.as_array() {
-            // Batch insert
-            let mut batch = Batch::new();
-            for item in arr {
-                batch.put_json(item.clone())?;
-            }
-            let count = index.batch(batch)?;
-            println!("Imported {} items", count);
-        } else {
-            // Single doc
-            index.put_json(val)?;
-            println!("Put 1 item");
-        }
+        // Import mode: JSONL file OR stdin JSON when --json
+        if let Some(path) = &args.import {
+            // JSONL: one JSON object per line
+            let f = File::open(path)?;
+            let reader = std::io::BufReader::new(f);
+            let mut batch = Batch::new();
+            for (lineno, line) in reader.lines().enumerate() {
+                let line = line?;
+                let t = line.trim();
+                if t.is_empty() { continue; }
+                let val: Value = serde_json::from_str(t)
+                    .map_err(|e| ministore::MinistoreError::Json(e))?;
+                batch.put_json(val)?;
+                if (lineno + 1) % 10_000 == 0 {
+                    // could flush in chunks in future; v1 keeps a single batch
+                }
+            }
+            let count = index.batch(batch)?;
+            println!("Imported {} items", count);
+        } else if args.json {
+            let mut buf = String::new();
+            std::io::stdin().read_to_string(&mut buf)?;
+            let val: Value = serde_json::from_str(&buf).map_err(ministore::MinistoreError::Json)?;
+            if let Some(arr) = val.as_array() {
+                let mut batch = Batch::new();
+                for item in arr {
+                    batch.put_json(item.clone())?;
+                }
+                let count = index.batch(batch)?;
+                println!("Imported {} items", count);
+            } else {
+                index.put_json(val)?;
+                println!("Put 1 item");
+            }
+        } else {
+            return Err(ministore::MinistoreError::Io(std::io::Error::new(
+                std::io::ErrorKind::InvalidInput,
+                "Must provide --path or --json/--import"
+            )));
+        }
     }
 
     Ok(())
 }
*** End Patch
```

---

## Patch 10 — CLI search: honor `--cursor short|full`

### Problem

CLI parses `--cursor` but ignores it; always uses `CursorMode::Full`.

### Patch

```diff
*** Begin Patch
*** Update File: crates/ministore-cli/src/commands/search.rs
@@
     let opts = SearchOptions {
         limit: args.limit,
         after: args.after,
-        cursor_mode: CursorMode::Full, // Defaulting to full for CLI
+        cursor_mode: match args.cursor.as_str() {
+            "short" => CursorMode::Short,
+            "full" => CursorMode::Full,
+            _ => CursorMode::Full,
+        },
         rank,
         show,
         explain: args.explain,
     };
*** End Patch
```

---

## Patch 11 — CLI index migrate: add `--schema` and implement rebuild using library

### Problem

`index migrate` is currently a placeholder. You have library support `Index::migrate_rebuild(new_path, new_schema)`.

### Fix

* Add `--schema <file>` and optional `--out <file>`
* Default `--out` to `<index>.db.tmp` and replace atomically

### Patch

```diff
*** Begin Patch
*** Update File: crates/ministore-cli/src/commands/index.rs
@@
 pub struct MigrateArgs {
     #[arg(short, long)]
     pub index: String,
-    // Target schema or path?
-    // Design doc says migrate_rebuild(new_path, new_schema).
-    // CLI probably needs args for that.
-    // For now assuming in-place migrate or just print help?
-    // The struct has only index. Design doc might be incomplete on CLI options.
-    // Let's implement as placeholder calling unimplemented logic or error.
+
+    #[arg(long)]
+    pub schema: std::path::PathBuf,
+
+    #[arg(long)]
+    pub out: Option<std::path::PathBuf>,
 }
@@
         IndexCmd::Migrate(args) => {
              let index_path = resolve_index_path(&args.index)?;
-             let _index = Index::open(&index_path, IndexOptions::default())?;
-             // index.migrate_rebuild(...) - needs args
-             println!("Migrate not fully exposed in CLI yet");
+             let index = Index::open(&index_path, IndexOptions::default())?;
+             let content = std::fs::read_to_string(&args.schema)?;
+             let new_schema = Schema::from_json(&content)?;
+
+             let out_path = args.out.clone().unwrap_or_else(|| {
+                 let mut p = index_path.clone();
+                 p.set_extension("db.tmp");
+                 p
+             });
+
+             index.migrate_rebuild(&out_path, new_schema)?;
+
+             // Replace original
+             std::fs::rename(&out_path, &index_path)?;
+             println!("Migrated index {}", index_path.display());
         }
*** End Patch
```

---

## Patch 12 — Index::get should return NotFound on missing rows (better UX)

### Problem

`Index::get` currently returns a raw rusqlite “QueryReturnedNoRows” error, not `MinistoreError::NotFound`.

### Patch

```diff
*** Begin Patch
*** Update File: crates/ministore/src/index.rs
@@
     pub fn get(&self, path: &str) -> Result<ItemView> {
@@
-        let row = conn.query_row(
+        let row = conn.query_row(
             SQL_GET_ITEM_BY_PATH,
             [path],
             |row| {
                 Ok((
                     row.get::<_, String>(1)?,  // data_json
                     row.get::<_, i64>(2)?,      // created_at
                     row.get::<_, i64>(3)?,      // updated_at
                 ))
             },
-        )?;
+        ).optional()?;
+
+        let row = row.ok_or_else(|| MinistoreError::NotFound(path.to_string()))?;
@@
         let (data_json, created_at, updated_at) = row;
*** End Patch
```

(You already import `OptionalExtension` at top, so this compiles.)

---

# Notes / “Why these patches first”

These address the biggest functional gaps and correctness issues:

* **Unquoted paths and dates** (core usability)
* **Date equality and date ranges** for date fields
* **Keyword-only indexes** (FTS optional)
* **JSONL import** and multi-value CLI `--set`
* **Cursor mode selection**
* **Index migrate** in CLI (no longer a stub)
* **Implicit fields in stats** (`created` / `updated`)
* **Cleaner NotFound for get**

---

# If you want the “next pass”

After you apply these, the next issues I would tackle (as another patch set) are:

1. Better NOT planning (avoid `items EXCEPT ...` huge intermediate)
2. Add `MAX_PREFIX_EXPANSION` enforcement for prefix wildcards (requires a cheap count query in planner)
3. `discover_overview` should compute `unique` and `examples` like the spec (currently minimal)
4. `search --rank=field:<date>` should distinguish date vs number cursor payload types (you currently store f64 for field rank payload)

If you paste the remaining files (or confirm there are no more), I can produce the next patch set as diffs in the same style.
