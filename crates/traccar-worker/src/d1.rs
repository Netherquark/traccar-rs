// SPDX-License-Identifier: Apache-2.0
//
// D1-backed implementation of the portable `Store` trait.
//
// Schema (see `migrations/0001_init.sql`):
//   objects(kind TEXT, id INTEGER, data TEXT, PRIMARY KEY(kind,id))
//   counters(kind TEXT PRIMARY KEY, value INTEGER)
//
// Queries push predicates down with SQLite's `json_extract`, which keeps
// position-history scans server-side. Instants are stored as UTC ISO-8601 text,
// which SQLite compares in chronological order.

use async_trait::async_trait;
use serde::Deserialize;
use serde_json::Value;
use traccar_core::error::{Error, Result};
use traccar_core::model::Id;
use traccar_core::store::{Filter, Store};
use wasm_bindgen::JsValue;
use worker::*;

pub struct D1Store {
    db: D1Database,
}

impl D1Store {
    pub fn new(db: D1Database) -> Self {
        D1Store { db }
    }

    fn build_where(&self, kind: &str, filter: &Filter, params: &mut Vec<JsValue>) -> String {
        let mut sql = String::from(" WHERE kind = ?");
        params.push(JsValue::from_str(kind));
        for (field, value) in &filter.eq {
            sql.push_str(&format!(" AND json_extract(data, '$.{field}') = ?"));
            params.push(json_to_js(value));
        }
        for (field, min, max) in &filter.range {
            sql.push_str(&format!(
                " AND CAST(json_extract(data, '$.{field}') AS REAL) >= ? AND CAST(json_extract(data, '$.{field}') AS REAL) <= ?"
            ));
            params.push(JsValue::from_f64(*min as f64));
            params.push(JsValue::from_f64(*max as f64));
        }
        for (field, min, max) in &filter.range_iso {
            sql.push_str(&format!(
                " AND json_extract(data, '$.{field}') >= ? AND json_extract(data, '$.{field}') <= ?"
            ));
            params.push(JsValue::from_str(min));
            params.push(JsValue::from_str(max));
        }
        for (field, values) in &filter.in_ {
            if values.is_empty() {
                sql.push_str(" AND 1 = 0");
                continue;
            }
            let placeholders = vec!["?"; values.len()].join(", ");
            sql.push_str(&format!(" AND json_extract(data, '$.{field}') IN ({placeholders})"));
            for v in values {
                params.push(json_to_js(v));
            }
        }
        sql
    }
}

fn json_to_js(v: &Value) -> JsValue {
    match v {
        Value::Null => JsValue::null(),
        Value::Bool(b) => JsValue::from_bool(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                JsValue::from_f64(i as f64)
            } else {
                JsValue::from_f64(n.as_f64().unwrap_or(0.0))
            }
        }
        Value::String(s) => JsValue::from_str(s),
        other => JsValue::from_str(&other.to_string()),
    }
}

#[derive(Deserialize)]
struct DataRow {
    data: String,
}

#[derive(Deserialize)]
struct CountRow {
    n: i64,
}

#[derive(Deserialize)]
struct IdRow {
    id: i64,
}

#[async_trait]
impl Store for D1Store {
    async fn insert(&self, kind: &str, obj: Value) -> Result<Id> {
        let next: IdRow = self
            .db
            .prepare(
                "INSERT INTO counters(kind, value) VALUES (?1, 1) \
                 ON CONFLICT(kind) DO UPDATE SET value = value + 1 RETURNING value AS id",
            )
            .bind(&[JsValue::from_str(kind)])
            .map_err(to_core)?
            .first(None)
            .await
            .map_err(to_core)?
            .ok_or_else(|| Error::internal("counter did not return an id"))?;

        let mut obj = obj;
        if let Some(map) = obj.as_object_mut() {
            map.insert("id".into(), Value::from(next.id));
        }
        self.db
            .prepare("INSERT INTO objects(kind, id, data) VALUES (?1, ?2, ?3)")
            .bind(&[JsValue::from_str(kind), JsValue::from_f64(next.id as f64), JsValue::from_str(&obj.to_string())])
            .map_err(to_core)?
            .run()
            .await
            .map_err(to_core)?;
        Ok(next.id)
    }

    async fn get(&self, kind: &str, id: Id) -> Result<Option<Value>> {
        let row: Option<DataRow> = self
            .db
            .prepare("SELECT data FROM objects WHERE kind = ?1 AND id = ?2")
            .bind(&[JsValue::from_str(kind), JsValue::from_f64(id as f64)])
            .map_err(to_core)?
            .first(None)
            .await
            .map_err(to_core)?;
        match row {
            Some(r) => serde_json::from_str(&r.data).map(Some).map_err(|e| Error::internal(e.to_string())),
            None => Ok(None),
        }
    }

    async fn update(&self, kind: &str, id: Id, obj: Value) -> Result<()> {
        self.db
            .prepare("UPDATE objects SET data = ?1 WHERE kind = ?2 AND id = ?3")
            .bind(&[JsValue::from_str(&obj.to_string()), JsValue::from_str(kind), JsValue::from_f64(id as f64)])
            .map_err(to_core)?
            .run()
            .await
            .map_err(to_core)?;
        Ok(())
    }

    async fn delete(&self, kind: &str, id: Id) -> Result<()> {
        self.db
            .prepare("DELETE FROM objects WHERE kind = ?1 AND id = ?2")
            .bind(&[JsValue::from_str(kind), JsValue::from_f64(id as f64)])
            .map_err(to_core)?
            .run()
            .await
            .map_err(to_core)?;
        Ok(())
    }

    async fn list(&self, kind: &str, filter: &Filter) -> Result<Vec<Value>> {
        let mut params: Vec<JsValue> = Vec::new();
        let mut sql = String::from("SELECT data FROM objects");
        sql.push_str(&self.build_where(kind, filter, &mut params));
        if let Some((field, desc)) = &filter.order_by {
            let dir = if *desc { "DESC" } else { "ASC" };
            sql.push_str(&format!(" ORDER BY json_extract(data, '$.{field}') {dir}"));
        }
        if let Some(limit) = filter.limit {
            sql.push_str(&format!(" LIMIT {limit}"));
            if let Some(offset) = filter.offset {
                sql.push_str(&format!(" OFFSET {offset}"));
            }
        } else if let Some(offset) = filter.offset {
            sql.push_str(&format!(" LIMIT -1 OFFSET {offset}"));
        }

        let result = self
            .db
            .prepare(&sql)
            .bind(&params)
            .map_err(to_core)?
            .all()
            .await
            .map_err(to_core)?;
        let rows: Vec<DataRow> = result.results().map_err(to_core)?;
        rows.into_iter()
            .map(|r| serde_json::from_str(&r.data).map_err(|e| Error::internal(e.to_string())))
            .collect()
    }

    async fn count(&self, kind: &str, filter: &Filter) -> Result<i64> {
        let mut params: Vec<JsValue> = Vec::new();
        let mut sql = String::from("SELECT COUNT(*) AS n FROM objects");
        sql.push_str(&self.build_where(kind, filter, &mut params));
        let row: Option<CountRow> = self
            .db
            .prepare(&sql)
            .bind(&params)
            .map_err(to_core)?
            .first(None)
            .await
            .map_err(to_core)?;
        Ok(row.map(|r| r.n).unwrap_or(0))
    }
}

fn to_core(e: worker::Error) -> Error {
    Error::internal(format!("d1: {e}"))
}
