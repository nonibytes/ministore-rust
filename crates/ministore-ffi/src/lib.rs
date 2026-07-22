use ministore::{
    Batch, CursorMode, Index, IndexOptions, OutputFieldSelector, RankMode, Schema, SearchOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::{Arc, Mutex, OnceLock};

const ABI_VERSION: u32 = 1;

struct Registry {
    next_handle: u64,
    indexes: HashMap<u64, Arc<Index>>,
}

impl Default for Registry {
    fn default() -> Self {
        Self {
            next_handle: 1,
            indexes: HashMap::new(),
        }
    }
}

static REGISTRY: OnceLock<Mutex<Registry>> = OnceLock::new();

fn registry() -> &'static Mutex<Registry> {
    REGISTRY.get_or_init(|| Mutex::new(Registry::default()))
}

#[derive(Serialize)]
struct Response<T: Serialize> {
    #[serde(skip_serializing_if = "Option::is_none")]
    ok: Option<T>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
}

#[derive(Deserialize)]
#[serde(default)]
struct SearchRequest {
    limit: usize,
    after: Option<String>,
    rank: String,
    rank_field: String,
    show: String,
    fields: Vec<String>,
    explain: bool,
    cursor_mode: String,
}

impl Default for SearchRequest {
    fn default() -> Self {
        Self {
            limit: 20,
            after: None,
            rank: "default".to_string(),
            rank_field: String::new(),
            show: "none".to_string(),
            fields: Vec::new(),
            explain: false,
            cursor_mode: "full".to_string(),
        }
    }
}

#[derive(Serialize)]
struct SearchResponse {
    items: Vec<Value>,
    next_cursor: Option<String>,
    has_more: bool,
    explain_sql: Option<String>,
    explain_steps: Option<Vec<String>>,
}

#[derive(Serialize)]
struct GetResponse {
    path: String,
    document: Value,
    created_at_ms: i64,
    updated_at_ms: i64,
}

#[no_mangle]
pub extern "C" fn ministore_abi_version() -> u32 {
    ABI_VERSION
}

#[no_mangle]
pub extern "C" fn ministore_create(path: *const c_char, schema_json: *const c_char) -> *mut c_char {
    ffi_call(|| {
        let path = read_string(path, "path")?;
        let schema =
            Schema::from_json(&read_string(schema_json, "schema")?).map_err(|e| e.to_string())?;
        let index =
            Index::create(path, schema, IndexOptions::default()).map_err(|e| e.to_string())?;
        insert_index(index)
    })
}

#[no_mangle]
pub extern "C" fn ministore_open(path: *const c_char) -> *mut c_char {
    ffi_call(|| {
        let index = Index::open(read_string(path, "path")?, IndexOptions::default())
            .map_err(|e| e.to_string())?;
        insert_index(index)
    })
}

#[no_mangle]
pub extern "C" fn ministore_close(handle: u64) -> *mut c_char {
    ffi_call(|| {
        let removed = registry()
            .lock()
            .map_err(|_| "index registry lock poisoned".to_string())?
            .indexes
            .remove(&handle)
            .is_some();
        if !removed {
            return Err(format!("invalid or closed index handle {handle}"));
        }
        Ok(json!(null))
    })
}

#[no_mangle]
pub extern "C" fn ministore_put_json(handle: u64, document_json: *const c_char) -> *mut c_char {
    ffi_call(|| {
        let document = parse_json(document_json, "document")?;
        with_index(handle, |index| {
            index.put_json(document).map_err(|e| e.to_string())?;
            Ok(json!(null))
        })
    })
}

#[no_mangle]
pub extern "C" fn ministore_batch_put_json(
    handle: u64,
    documents_json: *const c_char,
) -> *mut c_char {
    ffi_call(|| {
        let documents: Vec<Value> =
            serde_json::from_str(&read_string(documents_json, "documents")?)
                .map_err(|e| format!("invalid documents JSON: {e}"))?;
        with_index(handle, |index| {
            let mut batch = Batch::new();
            for document in documents {
                batch.put_json(document).map_err(|e| e.to_string())?;
            }
            index.batch(batch).map_err(|e| e.to_string())
        })
    })
}

#[no_mangle]
pub extern "C" fn ministore_get(handle: u64, path: *const c_char) -> *mut c_char {
    ffi_call(|| {
        let path = read_string(path, "path")?;
        with_index(handle, |index| {
            let item = index.get(&path).map_err(|e| e.to_string())?;
            Ok(GetResponse {
                path: item.path,
                document: item.doc,
                created_at_ms: item.meta.created_at_ms,
                updated_at_ms: item.meta.updated_at_ms,
            })
        })
    })
}

#[no_mangle]
pub extern "C" fn ministore_delete(handle: u64, path: *const c_char) -> *mut c_char {
    ffi_call(|| {
        let path = read_string(path, "path")?;
        with_index(handle, |index| {
            index.delete(&path).map_err(|e| e.to_string())
        })
    })
}

#[no_mangle]
pub extern "C" fn ministore_search(
    handle: u64,
    query: *const c_char,
    options_json: *const c_char,
) -> *mut c_char {
    ffi_call(|| {
        let query = read_string(query, "query")?;
        let request: SearchRequest = serde_json::from_str(&read_string(options_json, "options")?)
            .map_err(|e| format!("invalid search options: {e}"))?;
        let options = search_options(request)?;
        with_index(handle, |index| {
            let result = index.search(&query, options).map_err(|e| e.to_string())?;
            Ok(SearchResponse {
                items: result.items,
                next_cursor: result.next_cursor,
                has_more: result.has_more,
                explain_sql: result.explain_sql,
                explain_steps: result.explain_steps,
            })
        })
    })
}

#[no_mangle]
pub extern "C" fn ministore_optimize(handle: u64) -> *mut c_char {
    ffi_call(|| {
        with_index(handle, |index| {
            index.optimize().map_err(|e| e.to_string())?;
            Ok(json!(null))
        })
    })
}

#[no_mangle]
pub unsafe extern "C" fn ministore_string_free(value: *mut c_char) {
    if !value.is_null() {
        drop(CString::from_raw(value));
    }
}

fn search_options(request: SearchRequest) -> Result<SearchOptions, String> {
    let rank = match request.rank.as_str() {
        "default" | "" => RankMode::Default,
        "recency" => RankMode::Recency,
        "none" => RankMode::None,
        "field" if !request.rank_field.is_empty() => RankMode::Field(request.rank_field),
        other => return Err(format!("invalid rank mode {other:?}")),
    };
    let show = match request.show.as_str() {
        "none" | "" => OutputFieldSelector::None,
        "all" => OutputFieldSelector::All,
        "fields" => OutputFieldSelector::Fields(request.fields),
        other => return Err(format!("invalid show mode {other:?}")),
    };
    let cursor_mode = match request.cursor_mode.as_str() {
        "full" | "" => CursorMode::Full,
        "short" => CursorMode::Short,
        other => return Err(format!("invalid cursor mode {other:?}")),
    };
    Ok(SearchOptions {
        rank,
        limit: request.limit,
        after: request.after,
        show,
        explain: request.explain,
        cursor_mode,
    })
}

fn insert_index(index: Index) -> Result<u64, String> {
    let mut registry = registry()
        .lock()
        .map_err(|_| "index registry lock poisoned".to_string())?;
    let handle = registry.next_handle;
    registry.next_handle = registry
        .next_handle
        .checked_add(1)
        .ok_or_else(|| "index handle space exhausted".to_string())?;
    registry.indexes.insert(handle, Arc::new(index));
    Ok(handle)
}

fn with_index<T, F>(handle: u64, operation: F) -> Result<T, String>
where
    F: FnOnce(&Index) -> Result<T, String>,
{
    let index = registry()
        .lock()
        .map_err(|_| "index registry lock poisoned".to_string())?
        .indexes
        .get(&handle)
        .cloned()
        .ok_or_else(|| format!("invalid or closed index handle {handle}"))?;
    operation(&index)
}

fn read_string(value: *const c_char, name: &str) -> Result<String, String> {
    if value.is_null() {
        return Err(format!("{name} must not be null"));
    }
    unsafe { CStr::from_ptr(value) }
        .to_str()
        .map(str::to_owned)
        .map_err(|e| format!("{name} must be UTF-8: {e}"))
}

fn parse_json(value: *const c_char, name: &str) -> Result<Value, String> {
    serde_json::from_str(&read_string(value, name)?)
        .map_err(|e| format!("invalid {name} JSON: {e}"))
}

fn ffi_call<T, F>(operation: F) -> *mut c_char
where
    T: Serialize,
    F: FnOnce() -> Result<T, String>,
{
    let serialized = match catch_unwind(AssertUnwindSafe(operation)) {
        Ok(Ok(value)) => serde_json::to_string(&Response {
            ok: Some(value),
            error: None,
        }),
        Ok(Err(error)) => serde_json::to_string(&Response::<Value> {
            ok: None,
            error: Some(error),
        }),
        Err(_) => serde_json::to_string(&Response::<Value> {
            ok: None,
            error: Some("panic inside ministore Rust library".to_string()),
        }),
    }
    .unwrap_or_else(|_| r#"{"error":"failed to serialize FFI response"}"#.to_string());
    CString::new(serialized)
        .expect("serialized JSON contains no NUL")
        .into_raw()
}
