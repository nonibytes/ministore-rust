// Central place for SQL string constants (non-DDL).

pub const SQL_GET_META: &str = "SELECT value FROM meta WHERE key = ?1";
pub const SQL_SET_META: &str = "INSERT INTO meta(key,value) VALUES(?1,?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value";

pub const SQL_FIND_ITEM_ID_BY_PATH: &str = "SELECT id FROM items WHERE path = ?1";
pub const SQL_GET_ITEM_BY_PATH: &str = "SELECT id, data_json, created_at, updated_at FROM items WHERE path = ?1";

pub const SQL_DELETE_SEARCH_ROW: &str = "DELETE FROM search WHERE rowid = ?1";
pub const SQL_DELETE_PRESENT_BY_ITEM: &str = "DELETE FROM field_present WHERE item_id = ?1";
pub const SQL_DELETE_POSTINGS_BY_ITEM: &str = "DELETE FROM kw_postings WHERE item_id = ?1";
pub const SQL_DELETE_NUMBER_BY_ITEM: &str = "DELETE FROM field_number WHERE item_id = ?1";
pub const SQL_DELETE_DATE_BY_ITEM: &str = "DELETE FROM field_date WHERE item_id = ?1";
pub const SQL_DELETE_BOOL_BY_ITEM: &str = "DELETE FROM field_bool WHERE item_id = ?1";
pub const SQL_DELETE_ITEMS_BY_ID: &str = "DELETE FROM items WHERE id = ?1";

pub const SQL_CLEANUP_EXPIRED_CURSORS: &str = "DELETE FROM cursor_store WHERE expires_at < ?1";
pub const SQL_GET_CURSOR: &str = "SELECT payload FROM cursor_store WHERE handle = ?1";
pub const SQL_PUT_CURSOR: &str = "INSERT INTO cursor_store(handle, payload, created_at, expires_at) VALUES(?1,?2,?3,?4)";

pub const SQL_GET_VALUE_IDS_BY_ITEM: &str = "SELECT value_id FROM kw_postings WHERE item_id = ?1";
pub const SQL_DECREMENT_DOC_FREQ: &str = "UPDATE kw_dict SET doc_freq = doc_freq - 1 WHERE id = ?1";
pub const SQL_INCREMENT_DOC_FREQ: &str = "UPDATE kw_dict SET doc_freq = doc_freq + 1 WHERE id = ?1";

pub const SQL_INSERT_OR_IGNORE_KW_DICT: &str = 
    "INSERT OR IGNORE INTO kw_dict(field, value, doc_freq) VALUES(?1, ?2, 0)";
pub const SQL_GET_KW_DICT_ID: &str = 
    "SELECT id FROM kw_dict WHERE field = ?1 AND value = ?2";
pub const SQL_INSERT_OR_IGNORE_KW_POSTING: &str = 
    "INSERT OR IGNORE INTO kw_postings(field, value_id, item_id) VALUES(?1, ?2, ?3)";

pub const SQL_INSERT_FIELD_PRESENT: &str = 
    "INSERT INTO field_present(item_id, field) VALUES(?1, ?2)";
pub const SQL_INSERT_FIELD_NUMBER: &str = 
    "INSERT INTO field_number(item_id, field, value) VALUES(?1, ?2, ?3)";
pub const SQL_INSERT_FIELD_DATE: &str = 
    "INSERT INTO field_date(item_id, field, value) VALUES(?1, ?2, ?3)";
pub const SQL_INSERT_FIELD_BOOL: &str = 
    "INSERT INTO field_bool(item_id, field, value) VALUES(?1, ?2, ?3)";
