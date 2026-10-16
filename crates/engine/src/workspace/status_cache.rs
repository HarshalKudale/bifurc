//! The last computed git status of each workspace's entities: entity id -> `clean` | `modified` | `new` |
//! `deleted`.

use std::collections::HashMap;

use parking_lot::Mutex;
use serde_json::{Map, Value};

static CACHE: Mutex<Option<HashMap<String, Map<String, Value>>>> = Mutex::new(None);

pub fn set_cached_status(ws_id: &str, status: Map<String, Value>) {
    CACHE.lock().get_or_insert_with(HashMap::new).insert(ws_id.to_string(), status);
}

pub fn get_cached_status(ws_id: &str) -> Map<String, Value> {
    CACHE.lock().as_ref().and_then(|cache| cache.get(ws_id).cloned()).unwrap_or_default()
}

pub fn invalidate_cache(ws_id: &str) {
    if let Some(cache) = CACHE.lock().as_mut() {
        cache.remove(ws_id);
    }
}
