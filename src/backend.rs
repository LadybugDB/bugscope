//! Direct LadybugDB backend — in-process port of `src-tauri/src/lib.rs`.
//!
//! The Tauri commands become plain functions returning `GraphData`:
//!   * `scan_for_databases` — walk a dir for `*.lbdb` files (FilePickerModal)
//!   * `collect_edge_graph` — `MATCH (a)-[r]->(b) RETURN a, r, b LIMIT n`
//!     plus isolated nodes (initial graph load / `get_graph`)
//!   * `collect_schema_graph` — `CALL SHOW_TABLES` shape of the DB (schema view)
//!   * `search_nodes` — substring scan over `MATCH (n) RETURN n`
//!   * `neighborhood` — 1-hop expansion around a focused node
//!     (ports `get_node_neighborhood` + click-to-expand)

use anyhow::{Context, Result};
use lbug::{Connection, Database, SystemConfig, Value};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DatabaseInfo {
    pub id: usize,
    pub name: String,
    pub path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphNode {
    pub id: String,
    pub name: String,
    pub label: String,
    pub properties: HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GraphLink {
    pub source: String,
    pub target: String,
    pub label: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct GraphData {
    pub nodes: Vec<GraphNode>,
    pub links: Vec<GraphLink>,
}

pub const EDGE_SCAN_LIMIT: usize = 10_000;
pub const SEARCH_SCAN_LIMIT: usize = 50_000;
pub const SEARCH_RESULT_LIMIT: usize = 50;
pub const NEIGHBOR_LIMIT: usize = 120;

/// Walk `dir` for LadybugDB files — port of `scan_for_databases`.
/// Simple recursive walk; `dir` is expected to be small (the working dir).
pub fn scan_for_databases(dir: &Path) -> Vec<DatabaseInfo> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else {
            continue;
        };
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.extension().map(|e| e == "lbdb").unwrap_or(false) {
                out.push(database_info_for_path(&p));
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    for (i, db) in out.iter_mut().enumerate() {
        db.id = i;
    }
    out
}

/// Build a `DatabaseInfo` for an explicitly picked file.
pub fn database_info_for_path(path: &Path) -> DatabaseInfo {
    DatabaseInfo {
        id: 0,
        name: path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default(),
        path: path.to_string_lossy().into_owned(),
    }
}

pub fn default_db_dir() -> PathBuf {
    PathBuf::from(".")
}

fn value_to_string(v: &Value) -> String {
    match v {
        Value::Null(_) => String::new(),
        Value::Bool(b) => b.to_string(),
        Value::Int8(i) => i.to_string(),
        Value::Int16(i) => i.to_string(),
        Value::Int32(i) => i.to_string(),
        Value::Int64(i) => i.to_string(),
        Value::Int128(i) => i.to_string(),
        Value::UInt8(i) => i.to_string(),
        Value::UInt16(i) => i.to_string(),
        Value::UInt32(i) => i.to_string(),
        Value::UInt64(i) => i.to_string(),
        Value::Float(f) => f.to_string(),
        Value::Double(f) => f.to_string(),
        Value::String(s) => s.clone(),
        Value::Date(d) => format!("{d:?}"),
        Value::Timestamp(t) => format!("{t:?}"),
        Value::Interval(v) => format!("{v:?}"),
        _ => format!("{v:?}"),
    }
}

fn node_display_name(props: &[(String, Value)]) -> String {
    for key in ["name", "title", "label", "id"] {
        if let Some((_, v)) = props.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)) {
            let s = value_to_string(v);
            if !s.is_empty() {
                return s;
            }
        }
    }
    props
        .first()
        .map(|(_, v)| value_to_string(v))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "node".to_string())
}

fn graph_node_from_value(val: &Value) -> Option<GraphNode> {
    let Value::Node(node) = val else {
        return None;
    };
    let props = node.get_properties();
    let owned: Vec<(String, Value)> = props
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let id = format!(
        "{}:{}",
        node.get_node_id().table_id,
        node.get_node_id().offset
    );
    let mut properties = HashMap::new();
    for (k, v) in &owned {
        let s = value_to_string(v);
        if !s.is_empty() {
            properties.insert(k.clone(), s);
        }
    }
    Some(GraphNode {
        id,
        name: node_display_name(&owned),
        label: node.get_label_name().clone(),
        properties,
    })
}

pub fn open_connection(path: &str) -> Result<Connection<'_>> {
    let db = Database::new(path, SystemConfig::default())
        .with_context(|| format!("failed to open database {path}"))?;
    // Leak the Database so the Connection can outlive this call, mirroring the
    // Tauri backend which re-opens per command and drops at command end. Here a
    // short-lived query holds both alive via the leaked handle.
    let db: &'static Database = Box::leak(Box::new(db));
    Connection::new(db).context("failed to create connection")
}

/// Port of `collect_edge_graph`: edge-bounded load + isolated nodes.
pub fn collect_edge_graph(conn: &Connection, limit: usize) -> Result<GraphData> {
    let mut nodes: HashMap<String, GraphNode> = HashMap::new();
    let mut links = Vec::new();
    let mut seen = HashSet::new();

    let mut result = conn
        .query(&format!("MATCH (a)-[r]->(b) RETURN a, r, b LIMIT {limit}"))
        .context("relationship query failed")?;
    for row in &mut result {
        if row.len() < 3 {
            continue;
        }
        let (Value::Node(_), Value::Rel(rel), _) = (&row[0], &row[1], &row[2]) else {
            continue;
        };
        let source = format!(
            "{}:{}",
            rel.get_src_node().table_id,
            rel.get_src_node().offset
        );
        let target = format!(
            "{}:{}",
            rel.get_dst_node().table_id,
            rel.get_dst_node().offset
        );
        for val in [&row[0], &row[2]] {
            if let Some(n) = graph_node_from_value(val) {
                nodes.entry(n.id.clone()).or_insert(n);
            }
        }
        if seen.insert((source.clone(), target.clone(), rel.get_label_name().clone())) {
            links.push(GraphLink {
                source,
                target,
                label: rel.get_label_name().clone(),
            });
        }
    }

    let mut isolated = conn
        .query(&format!("MATCH (n) WHERE NOT (n)--() RETURN n LIMIT {limit}"))
        .context("isolated node query failed")?;
    for row in &mut isolated {
        for val in row.iter() {
            if let Some(n) = graph_node_from_value(val) {
                nodes.insert(n.id.clone(), n);
            }
        }
    }

    Ok(GraphData {
        nodes: nodes.into_values().collect(),
        links,
    })
}

/// Schema view — one node per node table, linked by rel-table connectivity.
/// Port of `collect_schema_graph` (simplified: tables + rel endpoints).
pub fn collect_schema_graph(conn: &Connection) -> Result<GraphData> {
    let mut tables: Vec<String> = Vec::new();
    if let Ok(mut result) = conn.query("CALL SHOW_TABLES() RETURN *;") {
        for row in &mut result {
            for val in row.iter() {
                let s = value_to_string(val);
                if !s.is_empty() && !tables.contains(&s) {
                    tables.push(s);
                }
            }
        }
    }
    // Fallback: derive labels from a node sample.
    if tables.is_empty() {
        let mut sample = conn.query("MATCH (n) RETURN n LIMIT 2000")?;
        let mut labels = HashSet::new();
        for row in &mut sample {
            for val in row.iter() {
                if let Value::Node(n) = val {
                    labels.insert(n.get_label_name().clone());
                }
            }
        }
        tables = labels.into_iter().collect();
        tables.sort();
    }
    let nodes = tables
        .iter()
        .map(|t| GraphNode {
            id: format!("schema:{t}"),
            name: t.clone(),
            label: "table".to_string(),
            properties: HashMap::new(),
        })
        .collect();
    Ok(GraphData { nodes, links: vec![] })
}

/// Substring search over node properties — port of `search_nodes` fallback path.
pub fn search_nodes(conn: &Connection, query: &str) -> Result<Vec<GraphNode>> {
    let q = query.trim().to_lowercase();
    if q.is_empty() {
        return Ok(Vec::new());
    }
    let mut result = conn.query(&format!("MATCH (n) RETURN n LIMIT {SEARCH_SCAN_LIMIT}"))?;
    let mut matches = Vec::new();
    'rows: for row in &mut result {
        for val in row.iter() {
            let Some(n) = graph_node_from_value(val) else {
                continue;
            };
            if n.name.to_lowercase().contains(&q)
                || n.id.to_lowercase().contains(&q)
                || n.label.to_lowercase().contains(&q)
                || n.properties.values().any(|v| v.to_lowercase().contains(&q))
            {
                matches.push(n);
                if matches.len() >= SEARCH_RESULT_LIMIT {
                    break 'rows;
                }
            }
        }
    }
    matches.sort_by(|a, b| {
        let a_exact = a.name.eq_ignore_ascii_case(query) || a.id.eq_ignore_ascii_case(query);
        let b_exact = b.name.eq_ignore_ascii_case(query) || b.id.eq_ignore_ascii_case(query);
        b_exact
            .cmp(&a_exact)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    Ok(matches)
}

/// 1-hop neighborhood of `focus` — port of `get_node_neighborhood`.
pub fn neighborhood(full: &GraphData, focus: &str) -> GraphData {
    let mut degrees: HashMap<&str, usize> = HashMap::new();
    for l in &full.links {
        *degrees.entry(l.source.as_str()).or_insert(0) += 1;
        *degrees.entry(l.target.as_str()).or_insert(0) += 1;
    }
    let mut neighbor_ids: Vec<&str> = full
        .links
        .iter()
        .filter_map(|l| {
            if l.source == focus {
                Some(l.target.as_str())
            } else if l.target == focus {
                Some(l.source.as_str())
            } else {
                None
            }
        })
        .collect();
    neighbor_ids.sort();
    neighbor_ids.dedup();
    neighbor_ids.sort_by_key(|id| std::cmp::Reverse(*degrees.get(id).unwrap_or(&0)));
    let mut visible: HashSet<&str> = neighbor_ids.into_iter().take(NEIGHBOR_LIMIT).collect();
    visible.insert(focus);
    let by_id: HashMap<&str, &GraphNode> =
        full.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
    let nodes = visible
        .iter()
        .filter_map(|id| by_id.get(id).cloned().cloned())
        .collect();
    let links = full
        .links
        .iter()
        .filter(|l| visible.contains(l.source.as_str()) && visible.contains(l.target.as_str()))
        .cloned()
        .collect();
    GraphData { nodes, links }
}
