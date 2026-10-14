// SPDX-License-Identifier: Apache-2.0
//
// Storage abstraction. The API layer talks to a document store keyed by
// (kind, id). This keeps the portable core independent of Cloudflare D1 while
// still allowing a D1-backed implementation in the worker crate.
//
// Values are `serde_json::Value` objects whose fields use the same camelCase
// names as the wire format, so persistence and transport share one schema.

use crate::error::{Error, Result};
use crate::model::{kind, Id};
use async_trait::async_trait;
use serde_json::Value;
use std::collections::{BTreeMap, HashMap};
use std::sync::Mutex;

/// A simple AND query. All present predicates must match.
#[derive(Clone, Debug, Default)]
pub struct Filter {
    /// field == value
    pub eq: Vec<(String, Value)>,
    /// min <= field <= max (numeric / millis)
    pub range: Vec<(String, i64, i64)>,
    /// min <= field <= max, lexicographic on ISO-8601 strings. Traccar stores
    /// instants as UTC ISO-8601 with a fixed offset, which sorts correctly as
    /// text, so this works identically in memory and in SQLite/D1.
    pub range_iso: Vec<(String, String, String)>,
    /// field IN (values)
    pub in_: Vec<(String, Vec<Value>)>,
    /// (field, descending)
    pub order_by: Option<(String, bool)>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

impl Filter {
    pub fn new() -> Self {
        Filter::default()
    }
    pub fn eq(mut self, field: &str, value: impl Into<Value>) -> Self {
        self.eq.push((field.to_string(), value.into()));
        self
    }
    pub fn range(mut self, field: &str, min: i64, max: i64) -> Self {
        self.range.push((field.to_string(), min, max));
        self
    }
    /// Inclusive range over ISO-8601 strings (chronological for UTC instants).
    pub fn range_iso(mut self, field: &str, min: impl Into<String>, max: impl Into<String>) -> Self {
        self.range_iso.push((field.to_string(), min.into(), max.into()));
        self
    }
    pub fn in_(mut self, field: &str, values: Vec<Value>) -> Self {
        self.in_.push((field.to_string(), values));
        self
    }
    pub fn order(mut self, field: &str, descending: bool) -> Self {
        self.order_by = Some((field.to_string(), descending));
        self
    }
    pub fn limit(mut self, n: usize) -> Self {
        self.limit = Some(n);
        self
    }
    pub fn offset(mut self, n: usize) -> Self {
        self.offset = Some(n);
        self
    }

    fn matches(&self, v: &Value) -> bool {
        for (f, want) in &self.eq {
            if v.get(f) != Some(want) {
                return false;
            }
        }
        for (f, vals) in &self.in_ {
            let got = v.get(f);
            if !vals.iter().any(|x| Some(x) == got) {
                return false;
            }
        }
        for (f, min, max) in &self.range {
            match v.get(f).and_then(|x| x.as_i64()) {
                Some(n) if n >= *min && n <= *max => {}
                _ => return false,
            }
        }
        for (f, min, max) in &self.range_iso {
            match v.get(f).and_then(|x| x.as_str()) {
                Some(s) if s >= min.as_str() && s <= max.as_str() => {}
                _ => return false,
            }
        }
        true
    }
}

/// Backend-agnostic storage. Object `id` is assigned by the store on insert
/// when the incoming object has id 0.
#[async_trait]
pub trait Store: Send + Sync {
    async fn insert(&self, kind: &str, obj: Value) -> Result<Id>;
    async fn get(&self, kind: &str, id: Id) -> Result<Option<Value>>;
    async fn update(&self, kind: &str, id: Id, obj: Value) -> Result<()>;
    async fn delete(&self, kind: &str, id: Id) -> Result<()>;
    async fn list(&self, kind: &str, filter: &Filter) -> Result<Vec<Value>>;
    async fn count(&self, kind: &str, filter: &Filter) -> Result<i64>;

    /// Delete every object of `kind` matching `filter`. Returns the number removed.
    async fn delete_many(&self, kind: &str, filter: &Filter) -> Result<usize> {
        let rows = self.list(kind, filter).await?;
        let mut n = 0;
        for row in rows {
            if let Some(id) = row.get("id").and_then(|v| v.as_i64()) {
                self.delete(kind, id).await?;
                n += 1;
            }
        }
        Ok(n)
    }
}

// ---------------------------------------------------------------------------
// In-memory implementation (tests, ephemeral single-isolate use)
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Inner {
    data: HashMap<String, BTreeMap<Id, Value>>,
    counters: HashMap<String, Id>,
}

/// A `Store` backed by process memory. Suitable for tests and for short-lived
/// single-isolate deployments; durable deployments use the D1-backed store.
#[derive(Default)]
pub struct MemoryStore {
    inner: Mutex<Inner>,
}

impl MemoryStore {
    pub fn new() -> Self {
        MemoryStore { inner: Mutex::new(Inner::default()) }
    }

    /// Insert many objects of one kind in a single lock acquisition.
    pub fn seed(&self, kind: &str, objects: Vec<Value>) {
        let mut inner = self.inner.lock().unwrap();
        for mut obj in objects {
            let next = {
                let counter = inner.counters.entry(kind.to_string()).or_insert(0);
                *counter += 1;
                *counter
            };
            if let Some(map) = obj.as_object_mut() {
                map.insert("id".into(), Value::from(next));
            }
            inner.data.entry(kind.to_string()).or_default().insert(next, obj);
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap()
    }
}

fn numeric(v: &Value) -> Option<f64> {
    v.as_f64()
}

fn compare(a: &Value, b: &Value) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (numeric(a), numeric(b)) {
        (Some(x), Some(y)) => x.partial_cmp(&y).unwrap_or(Ordering::Equal),
        _ => {
            let sa = a.as_str().unwrap_or("");
            let sb = b.as_str().unwrap_or("");
            sa.cmp(sb)
        }
    }
}

#[async_trait]
impl Store for MemoryStore {
    async fn insert(&self, kind: &str, mut obj: Value) -> Result<Id> {
        let mut inner = self.lock();
        let id = {
            let next = inner.counters.entry(kind.to_string()).or_insert(0);
            *next += 1;
            *next
        };
        if let Some(map) = obj.as_object_mut() {
            map.insert("id".into(), Value::from(id));
        }
        inner.data.entry(kind.to_string()).or_default().insert(id, obj);
        Ok(id)
    }

    async fn get(&self, kind: &str, id: Id) -> Result<Option<Value>> {
        let inner = self.lock();
        Ok(inner.data.get(kind).and_then(|m| m.get(&id)).cloned())
    }

    async fn update(&self, kind: &str, id: Id, mut obj: Value) -> Result<()> {
        let mut inner = self.lock();
        let table = inner.data.entry(kind.to_string()).or_default();
        if !table.contains_key(&id) {
            return Err(Error::not_found());
        }
        if let Some(map) = obj.as_object_mut() {
            map.insert("id".into(), Value::from(id));
        }
        table.insert(id, obj);
        Ok(())
    }

    async fn delete(&self, kind: &str, id: Id) -> Result<()> {
        let mut inner = self.lock();
        if let Some(table) = inner.data.get_mut(kind) {
            table.remove(&id);
        }
        Ok(())
    }

    async fn list(&self, kind: &str, filter: &Filter) -> Result<Vec<Value>> {
        let inner = self.lock();
        let mut rows: Vec<Value> = match inner.data.get(kind) {
            Some(table) => table.values().cloned().collect(),
            None => Vec::new(),
        };
        rows.retain(|v| filter.matches(v));
        if let Some((field, desc)) = &filter.order_by {
            rows.sort_by(|a, b| {
                let ord = compare(a.get(field).unwrap_or(&Value::Null), b.get(field).unwrap_or(&Value::Null));
                if *desc {
                    ord.reverse()
                } else {
                    ord
                }
            });
        }
        if let Some(off) = filter.offset {
            rows = rows.into_iter().skip(off).collect();
        }
        if let Some(lim) = filter.limit {
            rows.truncate(lim);
        }
        Ok(rows)
    }

    async fn count(&self, kind: &str, filter: &Filter) -> Result<i64> {
        let inner = self.lock();
        let n = match inner.data.get(kind) {
            Some(table) => table.values().filter(|v| filter.matches(v)).count(),
            None => 0,
        };
        Ok(n as i64)
    }
}

/// Convenience: fetch the current server row, creating it if missing.
pub async fn ensure_server(store: &dyn Store) -> Result<Value> {
    if let Some(v) = store.get(kind::SERVER, 1).await? {
        return Ok(v);
    }
    let server = serde_json::to_value(crate::model::Server::default())?;
    store.update(kind::SERVER, 1, server.clone()).await?;
    Ok(server)
}
