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
use lbug::{Connection, Database, InternalID, SystemConfig, Value};
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

/// List LadybugDB files directly inside `dir` — port of `scan_for_databases`.
/// Single level only (maxdepth 1): never descend into subdirectories, so a
/// stray working dir (e.g. `/` when launched as a macOS .app) can't trigger
/// a whole-filesystem walk that hangs startup.
pub fn scan_for_databases(dir: &Path) -> Vec<DatabaseInfo> {
    let mut out = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let p = entry.path();
        if p.is_file() && p.extension().map(|e| e == "lbdb").unwrap_or(false) {
            out.push(database_info_for_path(&p));
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
    let owned: Vec<(String, Value)> = props.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
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
    let conn = Connection::new(db).context("failed to create connection")?;
    // Best-effort: make the icebug-backed algo extension (PAGE_RANK,
    // GDS_PAGE_RANK, PROJECT_GRAPH, …) available to Cypher. Missing bundle
    // just means those functions are unavailable; everything else works.
    ensure_algo_extension(&conn);
    Ok(conn)
}

/// Install the algo extension from the official repo
/// (`https://extension.ladybugdb.com`). A no-op when already installed.
/// Returns false when the download fails (e.g. offline). Never fails the caller.
pub fn install_algo_extension(conn: &Connection) -> bool {
    conn.query("INSTALL algo").is_ok()
}

/// Load the algo extension on this connection. Returns true when the
/// GDS_* surface is available. Never fails the caller.
///
/// Must run per connection, not once per process: ladybug tracks loaded
/// extensions per Database (`ExtensionManager` lives on the `Database`), and
/// this app opens a fresh `Database` per `open_connection` call. A cached
/// "already loaded" result would leave other connections without GDS.
///
/// Resolution order: `LOAD algo` (already installed), otherwise
/// `INSTALL algo` from the official repo followed by `LOAD algo`. As an
/// escape hatch for local extension builds, `BUGSCOPE_ALGO_EXTENSION` may
/// point at a `.lbug_extension` file to `LOAD EXTENSION` directly.
pub fn ensure_algo_extension(conn: &Connection) -> bool {
    if let Some(path) = std::env::var_os("BUGSCOPE_ALGO_EXTENSION").map(PathBuf::from) {
        // LOAD EXTENSION takes a path without the suffix on some builds;
        // try the full path first, then the stem form.
        let full = path.to_string_lossy().replace('\\', "/");
        if conn.query(&format!("LOAD EXTENSION '{full}'")).is_ok() {
            return true;
        }
        let stem = full
            .strip_suffix(".lbug_extension")
            .unwrap_or(&full)
            .to_string();
        if conn.query(&format!("LOAD EXTENSION '{stem}'")).is_ok() {
            return true;
        }
    }
    if conn.query("LOAD algo").is_ok() {
        return true;
    }
    // Not installed yet: fetch it, then load.
    if install_algo_extension(conn) {
        return conn.query("LOAD algo").is_ok();
    }
    false
}

/// In-process PageRank over an already-collected graph via the linked icebug
/// (NetworKit) library — no extension needed. Returns `(node_id, score)`
/// sorted by score descending. Mirrors the GDS_PAGE_RANK ranking so the
/// extension result can be cross-checked (see `tests/gds_page_rank.rs`).
///
/// Without the `icebug-analytics` feature this is unavailable and returns
/// an empty ranking (callers show an empty Top PageRank list).
#[cfg(feature = "icebug-analytics")]
pub fn graphr_page_rank(data: &GraphData) -> Vec<(String, f64)> {
    // icebug 13.2.0 speaks arrow 56: build with the alias, not the
    // lbug-pinned `arrow` (55).
    use arrow56::array::UInt64Array;
    let index: HashMap<&str, u64> = data
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i as u64))
        .collect();
    let n = data.nodes.len() as u64;
    if n == 0 {
        return Vec::new();
    }
    // Directed GraphR needs both out- and in-edge CSRs.
    let mut outgoing: Vec<Vec<u64>> = vec![Vec::new(); n as usize];
    let mut incoming: Vec<Vec<u64>> = vec![Vec::new(); n as usize];
    for l in &data.links {
        if let (Some(&s), Some(&t)) = (index.get(l.source.as_str()), index.get(l.target.as_str())) {
            if s != t {
                outgoing[s as usize].push(t);
                incoming[t as usize].push(s);
            }
        }
    }
    fn pack(adj: Vec<Vec<u64>>) -> (Vec<u64>, Vec<u64>) {
        let mut indptr = Vec::with_capacity(adj.len() + 1);
        let mut indices = Vec::new();
        for mut neighbors in adj {
            neighbors.sort_unstable();
            neighbors.dedup();
            indptr.push(indices.len() as u64);
            indices.extend(neighbors);
        }
        indptr.push(indices.len() as u64);
        (indices, indptr)
    }
    let (out_indices, out_indptr) = pack(outgoing);
    let (in_indices, in_indptr) = pack(incoming);
    let Ok(graph) = icebug::GraphR::from_directed_csr(
        n,
        UInt64Array::from(out_indices),
        UInt64Array::from(out_indptr),
        UInt64Array::from(in_indices),
        UInt64Array::from(in_indptr),
    ) else {
        return Vec::new();
    };
    // normalized=false: scores form a sum-to-1 distribution like GDS_PAGE_RANK.
    let Ok(mut pr) = icebug::PageRank::new(&graph, 0.85, 1e-9, false) else {
        return Vec::new();
    };
    if pr.run().is_err() {
        return Vec::new();
    }
    let Ok(scores) = pr.scores() else {
        return Vec::new();
    };
    let mut out: Vec<(String, f64)> = data
        .nodes
        .iter()
        .enumerate()
        .map(|(i, nd)| (nd.id.clone(), *scores.get(i).unwrap_or(&0.0)))
        .collect();
    out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    out
}

/// (table_id, name, kind) for every table — `CALL SHOW_TABLES` is tiny.
fn table_catalog(conn: &Connection) -> Result<Vec<(u64, String, String)>> {
    let mut out = Vec::new();
    let mut result = conn.query("CALL SHOW_TABLES() RETURN *;")?;
    for row in &mut result {
        if row.len() < 3 {
            continue;
        }
        let (Value::UInt64(id), Value::String(name), Value::String(kind)) =
            (&row[0], &row[1], &row[2])
        else {
            continue;
        };
        out.push((*id, name.clone(), kind.clone()));
    }
    Ok(out)
}

/// A sampled node identity: `(table_id, offset)`, matching `"table:offset"` ids.
type NodeId = (u64, u64);

/// Decode an `INTERNAL_ID` Arrow struct column (`{offset, table}`) into
/// `(table_id, offset)` pairs, skipping nulls.
///
/// Our `arrow` is pinned to 55 to match lbug, so these downcasts hit the
/// same types lbug builds its batches with. A 55/56 skew here silently
/// decodes zero rows (observed: 6285 edges → 0), which is why the loader
/// verifies every tier instead of trusting an empty success.
fn decode_internal_id(col: &arrow::array::StructArray) -> Vec<NodeId> {
    use arrow::array::Array;
    let offsets: Vec<i64> = col
        .column_by_name("offset")
        .and_then(|c| c.as_any().downcast_ref::<arrow::array::Int64Array>())
        .map(|a| a.iter().map(|v| v.unwrap_or(0)).collect())
        .unwrap_or_default();
    let tables: Vec<i64> = col
        .column_by_name("table")
        .and_then(|c| c.as_any().downcast_ref::<arrow::array::Int64Array>())
        .map(|a| a.iter().map(|v| v.unwrap_or(0)).collect())
        .unwrap_or_default();
    (0..col.len())
        .filter(|&i| col.is_valid(i))
        .filter_map(|i| Some((*tables.get(i)?, *offsets.get(i)?)))
        .map(|(t, o)| (t.max(0) as u64, o.max(0) as u64))
        .collect()
}

/// Columnar edge scan via Arrow memory: `RETURN id(a), id(b), label(r)`.
/// No `Node`/`Rel` objects are materialized per row, so bulk loads from
/// larger DBs stay fast. Decoding relies on the arrow-55 pin (see above).
fn collect_edges_arrow(
    conn: &Connection,
    limit: usize,
    links: &mut Vec<GraphLink>,
    node_ids: &mut HashSet<NodeId>,
) -> Result<()> {
    let mut result = conn
        .query_as_arrow(
            &format!(
                "MATCH (a)-[r]->(b) RETURN id(a) AS src, id(b) AS dst, label(r) AS rel LIMIT {limit}"
            ),
            65_536,
        )
        .context("arrow edge query failed")?;
    let mut seen = HashSet::new();
    for batch in result.iter_arrow(65_536)? {
        let srcs = batch
            .column_by_name("src")
            .and_then(|c| c.as_any().downcast_ref::<arrow::array::StructArray>())
            .map(decode_internal_id)
            .unwrap_or_default();
        let dsts = batch
            .column_by_name("dst")
            .and_then(|c| c.as_any().downcast_ref::<arrow::array::StructArray>())
            .map(decode_internal_id)
            .unwrap_or_default();
        let rels: Vec<String> = batch
            .column_by_name("rel")
            .and_then(|c| c.as_any().downcast_ref::<arrow::array::StringArray>())
            .map(|a| a.iter().map(|v| v.unwrap_or("").to_string()).collect())
            .unwrap_or_default();
        for (i, ((st, so), (dt, doff))) in srcs.into_iter().zip(dsts).enumerate() {
            let label = rels.get(i).cloned().unwrap_or_default();
            let source = format!("{st}:{so}");
            let target = format!("{dt}:{doff}");
            node_ids.insert((st, so));
            node_ids.insert((dt, doff));
            if seen.insert((source.clone(), target.clone(), label.clone())) {
                links.push(GraphLink {
                    source,
                    target,
                    label,
                });
            }
        }
    }
    Ok(())
}

/// Row-wise edge scan: `RETURN id(a), id(b), label(r)`.
/// Only internal ids and the rel label cross each row — no `Node`/`Rel`
/// objects are materialized — so this stays cheap up to the edge limit.
///
/// NOTE: this deliberately avoids decoding `query_as_arrow` batches: it is
/// the version-proof last resort behind the pinned-Arrow fast paths.
/// Row values cross the boundary as plain `Value`s.
fn collect_edges_rows(
    conn: &Connection,
    limit: usize,
    links: &mut Vec<GraphLink>,
    node_ids: &mut HashSet<NodeId>,
) -> Result<()> {
    let mut result = conn
        .query(&format!(
            "MATCH (a)-[r]->(b) RETURN id(a), id(b), label(r) LIMIT {limit}"
        ))
        .context("edge scan failed")?;
    let mut seen = HashSet::new();
    for row in &mut result {
        if row.len() < 3 {
            continue;
        }
        let (Value::InternalID(src), Value::InternalID(dst), Value::String(label)) =
            (&row[0], &row[1], &row[2])
        else {
            continue;
        };
        let (st, so) = (src.table_id, src.offset);
        let (dt, doff) = (dst.table_id, dst.offset);
        let source = format!("{st}:{so}");
        let target = format!("{dt}:{doff}");
        node_ids.insert((st, so));
        node_ids.insert((dt, doff));
        if seen.insert((source.clone(), target.clone(), label.clone())) {
            links.push(GraphLink {
                source,
                target,
                label: label.clone(),
            });
        }
    }
    Ok(())
}

/// Native CSR data path: per rel table, `RETURN a.rowid, r.rowid, b.rowid`
/// via `query_as_arrow` carries CSR metadata and `QueryResult::csr()` hands
/// back zero-copy `indptr`/`indices` arrays — the full adjacency list
/// without materializing a single row. Src/dst table ids come from one cheap
/// normal `LIMIT 1` query per rel table (metadata only). Any failure
/// (no/empty CSR metadata, e.g. older storage versions) is an `Err` so the
/// caller can fall back to the row-wise scan.
fn collect_edges_csr(
    conn: &Connection,
    rel_tables: &[String],
    limit: usize,
) -> Result<(Vec<GraphLink>, HashSet<NodeId>)> {
    let mut links = Vec::new();
    let mut node_ids = HashSet::new();
    let mut seen = HashSet::new();
    let mut budget = limit;
    for rel in rel_tables {
        if budget == 0 {
            break;
        }
        let result = conn
            .query_as_arrow(
                &format!("MATCH (a)-[r:{rel}]->(b) RETURN a.rowid, r.rowid, b.rowid"),
                65_536,
            )
            .context("arrow csr query failed")?;
        let csr = result.csr().context("no CSR metadata")?;
        let mut meta = conn
            .query(&format!(
                "MATCH (a)-[r:{rel}]->(b) RETURN id(a), id(b) LIMIT 1"
            ))
            .context("rel endpoint query failed")?;
        let (src_table, dst_table) = match meta.next() {
            Some(row) if row.len() >= 2 => match (&row[0], &row[1]) {
                (Value::InternalID(s), Value::InternalID(d)) => (s.table_id, d.table_id),
                _ => continue,
            },
            _ => continue, // empty rel table
        };
        let indptr = csr.indptr.values();
        let indices = csr.indices.values();
        let n_src = csr.indptr.len().saturating_sub(1);
        'src: for s in 0..n_src {
            let start = indptr[s] as usize;
            let end = indptr[s + 1] as usize;
            for &d in &indices[start.min(indices.len())..end.min(indices.len())] {
                if budget == 0 {
                    break 'src;
                }
                budget -= 1;
                let (su, du) = (s as u64, d.max(0) as u64);
                node_ids.insert((src_table, su));
                node_ids.insert((dst_table, du));
                let source = format!("{src_table}:{su}");
                let target = format!("{dst_table}:{du}");
                if seen.insert((source.clone(), target.clone())) {
                    links.push(GraphLink {
                        source,
                        target,
                        label: rel.clone(),
                    });
                }
            }
        }
    }
    // Some storage versions report Ok with empty CSR arrays instead of an
    // error. Zero links from a non-empty rel catalog is indistinguishable
    // from a broken fast path, so decline it and let the caller verify via
    // the row-wise scan (which also finds nothing on a truly empty db).
    if links.is_empty() && !rel_tables.is_empty() {
        anyhow::bail!("empty CSR for {} rel tables", rel_tables.len());
    }
    Ok((links, node_ids))
}

/// Port of `collect_edge_graph`: edge-bounded load + isolated nodes.
///
/// Topology comes from native CSR via Arrow memory (`query_as_arrow` with a
/// `RETURN a.rowid, r.rowid, b.rowid` projection + `QueryResult::csr()`),
/// one CSR per rel table — no rows are materialized for edges. Missing or
/// empty CSR metadata falls back to the columnar Arrow id scan, then to the
/// row-wise id scan; schema (`SHOW_TABLES`) and node properties always use
/// normal row queries. Tiers decline on error *or* empty success, so a
/// silent-empty fast path can never hide the graph.
pub fn collect_edge_graph(conn: &Connection, limit: usize) -> Result<GraphData> {
    let catalog = table_catalog(conn).unwrap_or_default();
    let rel_tables: Vec<String> = catalog
        .iter()
        .filter(|(_, _, kind)| kind == "REL")
        .map(|(_, name, _)| name.clone())
        .collect();
    let label_of_table: HashMap<u64, String> = catalog
        .iter()
        .filter(|(_, _, kind)| kind == "NODE")
        .map(|(id, name, _)| (*id, name.clone()))
        .collect();

    let mut links = Vec::new();
    let mut node_ids: HashSet<NodeId> = HashSet::new();
    // CSR first for data, then columnar Arrow, then row-wise. Each tier
    // declines on error or empty success (an empty win from a non-empty
    // catalog is indistinguishable from a broken fast path).
    match collect_edges_csr(conn, &rel_tables, limit) {
        Ok((l, ids)) => {
            links = l;
            node_ids = ids;
        }
        Err(e) => {
            eprintln!("bugscope: CSR edge scan unavailable ({e}); trying Arrow scan");
            let mut arrow_links = Vec::new();
            let mut arrow_ids = HashSet::new();
            let arrow_ok = collect_edges_arrow(conn, limit, &mut arrow_links, &mut arrow_ids)
                .is_ok()
                && !arrow_links.is_empty();
            if arrow_ok {
                links = arrow_links;
                node_ids = arrow_ids;
            } else {
                eprintln!("bugscope: Arrow scan unavailable/empty; falling back to row-wise scan");
                collect_edges_rows(conn, limit, &mut links, &mut node_ids)?
            }
        }
    }

    // Isolated nodes by internal id — schema/details stay on normal queries.
    if let Ok(mut iso) = conn.query(&format!(
        "MATCH (n) WHERE NOT (n)--() RETURN id(n) LIMIT {limit}"
    )) {
        for row in &mut iso {
            if let Some(Value::InternalID(id)) = row.first() {
                node_ids.insert((id.table_id, id.offset));
            }
        }
    }

    let mut nodes = HashMap::new();
    enrich_node_properties(conn, &label_of_table, &node_ids, &mut nodes);
    Ok(GraphData {
        nodes: nodes.into_values().collect(),
        links,
    })
}

/// Fetch display names / properties for exactly the sampled nodes, grouped by
/// table and batched with `WHERE offset(id(n)) IN [...]` so property
/// materialization stays proportional to the sample, not the database.
fn enrich_node_properties(
    conn: &Connection,
    label_of_table: &HashMap<u64, String>,
    node_ids: &HashSet<NodeId>,
    nodes: &mut HashMap<String, GraphNode>,
) {
    let mut by_table: HashMap<u64, Vec<u64>> = HashMap::new();
    for &(t, o) in node_ids {
        by_table.entry(t).or_default().push(o);
    }
    for (table, mut offsets) in by_table {
        let Some(label) = label_of_table.get(&table) else {
            continue;
        };
        // Seed id+label so nodes survive even if the property fetch fails.
        for &o in &offsets {
            let id = format!("{table}:{o}");
            nodes.entry(id.clone()).or_insert_with(|| GraphNode {
                id: id.clone(),
                name: id.clone(),
                label: label.clone(),
                properties: HashMap::new(),
            });
        }
        offsets.sort_unstable();
        offsets.dedup();
        for chunk in offsets.chunks(1000) {
            let list = chunk
                .iter()
                .map(|o| o.to_string())
                .collect::<Vec<_>>()
                .join(",");
            let Ok(mut result) = conn.query(&format!(
                "MATCH (n:{label}) WHERE offset(id(n)) IN [{list}] RETURN n"
            )) else {
                continue;
            };
            for row in &mut result {
                for val in row.iter() {
                    if let Some(n) = graph_node_from_value(val) {
                        nodes.insert(n.id.clone(), n);
                    }
                }
            }
        }
    }
}

/// Schema view — one node per node table, linked by rel-table connectivity.
/// Port of `collect_schema_graph`: node tables become nodes, and each rel
/// table contributes one link, with endpoints resolved by sampling a single
/// edge per rel table.
pub fn collect_schema_graph(conn: &Connection) -> Result<GraphData> {
    let catalog = table_catalog(conn).unwrap_or_default();
    let mut node_tables: Vec<String> = catalog
        .iter()
        .filter(|(_, _, kind)| kind == "NODE")
        .map(|(_, name, _)| name.clone())
        .collect();
    let rel_tables: Vec<String> = catalog
        .iter()
        .filter(|(_, _, kind)| kind == "REL")
        .map(|(_, name, _)| name.clone())
        .collect();
    // Fallback: derive labels from a node sample when the catalog is empty.
    if node_tables.is_empty() {
        let mut sample = conn.query("MATCH (n) RETURN n LIMIT 2000")?;
        let mut labels = HashSet::new();
        for row in &mut sample {
            for val in row.iter() {
                if let Value::Node(n) = val {
                    labels.insert(n.get_label_name().clone());
                }
            }
        }
        node_tables = labels.into_iter().collect();
        node_tables.sort();
    }
    // One edge sample per rel table resolves its endpoint node tables.
    let mut links = Vec::new();
    let mut seen = HashSet::new();
    for rel in &rel_tables {
        let Ok(mut result) = conn.query(&format!(
            "MATCH (a)-[r:{rel}]->(b) RETURN label(a) AS s, label(b) AS t LIMIT 1"
        )) else {
            continue;
        };
        for row in &mut result {
            if row.len() < 2 {
                continue;
            }
            let (Value::String(s), Value::String(t)) = (&row[0], &row[1]) else {
                continue;
            };
            if seen.insert((s.clone(), t.clone(), rel.clone())) {
                links.push(GraphLink {
                    source: format!("schema:{s}"),
                    target: format!("schema:{t}"),
                    label: rel.clone(),
                });
            }
        }
    }
    let nodes = node_tables
        .iter()
        .map(|t| GraphNode {
            id: format!("schema:{t}"),
            name: t.clone(),
            label: "table".to_string(),
            properties: HashMap::new(),
        })
        .collect();
    Ok(GraphData { nodes, links })
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

/// Run an arbitrary read Cypher query and build a graph from the result:
/// every returned `Node` becomes a node, every `Rel` a link (with endpoint
/// nodes seeded from the rel's internal ids if not returned themselves),
/// recursive rels contribute both. Scalar-only results yield an empty graph.
pub fn run_cypher(conn: &Connection, query: &str) -> Result<GraphData> {
    const CYPHER_NODE_LIMIT: usize = 20_000;
    let mut nodes: HashMap<String, GraphNode> = HashMap::new();
    let mut links = Vec::new();
    let seed = |nodes: &mut HashMap<String, GraphNode>, id: &InternalID| {
        let key = format!("{id}");
        nodes.entry(key.clone()).or_insert_with(|| GraphNode {
            id: key,
            name: format!("{id}"),
            label: String::new(),
            properties: HashMap::new(),
        });
    };
    let mut result = conn.query(query)?;
    'rows: for row in &mut result {
        for val in row.iter() {
            match val {
                Value::Node(_) => {
                    if let Some(n) = graph_node_from_value(val) {
                        if nodes.len() >= CYPHER_NODE_LIMIT {
                            break 'rows;
                        }
                        nodes.insert(n.id.clone(), n);
                    }
                }
                Value::Rel(rel) => {
                    let (src, dst) = (rel.get_src_node(), rel.get_dst_node());
                    seed(&mut nodes, src);
                    seed(&mut nodes, dst);
                    if links.len() >= EDGE_SCAN_LIMIT {
                        break 'rows;
                    }
                    links.push(GraphLink {
                        source: format!("{src}"),
                        target: format!("{dst}"),
                        label: rel.get_label_name().clone(),
                    });
                }
                Value::RecursiveRel {
                    nodes: rn,
                    rels: rr,
                } => {
                    for n in rn {
                        seed(&mut nodes, n.get_node_id());
                    }
                    for r in rr {
                        let (src, dst) = (r.get_src_node(), r.get_dst_node());
                        seed(&mut nodes, src);
                        seed(&mut nodes, dst);
                        if links.len() >= EDGE_SCAN_LIMIT {
                            break 'rows;
                        }
                        links.push(GraphLink {
                            source: format!("{src}"),
                            target: format!("{dst}"),
                            label: r.get_label_name().clone(),
                        });
                    }
                }
                _ => {}
            }
        }
    }
    Ok(GraphData {
        nodes: nodes.into_values().collect(),
        links,
    })
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
    let by_id: HashMap<&str, &GraphNode> = full.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
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

/// Fallback when `icebug-analytics` is off: no ranking available.
#[cfg(not(feature = "icebug-analytics"))]
pub fn graphr_page_rank(data: &GraphData) -> Vec<(String, f64)> {
    let _ = data;
    Vec::new()
}

/// Leiden community detection over an already-collected graph via the linked
/// icebug (NetworKit) library — the same algorithm family as the
/// `GDS_LEIDEN()` table function in the algo extension (see
/// `tests/gds_leiden.rs`, ported from the extension's `gds_leiden.test`).
/// Runs in-process, so it works on any displayed graph — full edge scan,
/// Cypher result, or 1-hop neighborhood — without a `PROJECT_GRAPH` step.
///
/// Returns `(assignment, modularity, community_count)` where `assignment[i]`
/// is the community of `data.nodes[i]`. Community ids are arbitrary; only
/// equality matters.
#[cfg(feature = "icebug-analytics")]
pub fn graphr_leiden_full(data: &GraphData) -> (Vec<u64>, f64, u64) {
    // icebug 13.2.0 speaks arrow 56: build with the alias, not the
    // lbug-pinned `arrow` (55).
    use arrow56::array::UInt64Array;
    let n = data.nodes.len();
    if n == 0 {
        return (Vec::new(), 0.0, 0);
    }
    if data.links.is_empty() {
        return (vec![0; n], 0.0, 1);
    }
    let index: HashMap<&str, u64> = data
        .nodes
        .iter()
        .enumerate()
        .map(|(i, nd)| (nd.id.as_str(), i as u64))
        .collect();
    // Leiden works on undirected topology: union both directions.
    let mut adj: Vec<Vec<u64>> = vec![Vec::new(); n];
    for l in &data.links {
        if let (Some(&s), Some(&t)) = (index.get(l.source.as_str()), index.get(l.target.as_str())) {
            if s != t {
                adj[s as usize].push(t);
                adj[t as usize].push(s);
            }
        }
    }
    let mut indptr = Vec::with_capacity(n + 1);
    let mut indices = Vec::new();
    for mut neighbors in adj {
        neighbors.sort_unstable();
        neighbors.dedup();
        indptr.push(indices.len() as u64);
        indices.extend(neighbors);
    }
    indptr.push(indices.len() as u64);
    let Ok(graph) = icebug::GraphR::from_csr(
        n as u64,
        false,
        UInt64Array::from(indices),
        UInt64Array::from(indptr),
    ) else {
        return (vec![0; n], 0.0, 1);
    };
    // Deterministic (randomize=false) so the treemap is stable across loads.
    let Ok(mut leiden) = icebug::Leiden::new(&graph, 32, false, 1.0) else {
        return (vec![0; n], 0.0, 1);
    };
    if leiden.run().is_err() {
        return (vec![0; n], 0.0, 1);
    }
    let (membership, count) = match leiden.partition() {
        Ok(p) => (p.membership, p.count),
        Err(_) => (vec![0; n], 1),
    };
    let modularity = leiden.modularity().unwrap_or(0.0);
    let mut assignment = vec![0u64; n];
    for (i, c) in membership.into_iter().enumerate().take(n) {
        assignment[i] = c;
    }
    (assignment, modularity, count)
}

/// Leiden community assignment per node, in `data.nodes` order.
/// Convenience wrapper over [`graphr_leiden_full`] when modularity is unneeded.
#[cfg(feature = "icebug-analytics")]
pub fn graphr_leiden(data: &GraphData) -> Vec<u64> {
    graphr_leiden_full(data).0
}

/// Fallback when `icebug-analytics` is off: everything in one community.
#[cfg(not(feature = "icebug-analytics"))]
pub fn graphr_leiden_full(data: &GraphData) -> (Vec<u64>, f64, u64) {
    let n = data.nodes.len();
    (vec![0; n], 0.0, if n == 0 { 0 } else { 1 })
}

/// Fallback when `icebug-analytics` is off.
#[cfg(not(feature = "icebug-analytics"))]
pub fn graphr_leiden(data: &GraphData) -> Vec<u64> {
    graphr_leiden_full(data).0
}

/// Read back the communities of an existing `PROJECT_GRAPH` projection via
/// the algo extension's `GDS_LEIDEN()` table function:
///
/// ```cypher
/// CALL PROJECT_GRAPH('G', ['N'], ['E']);
/// CALL GDS_LEIDEN('G') RETURN node, community_id;
/// ```
///
/// Returns `node_id ("table:offset") -> community_id`, mirroring the
/// `gds_leiden.test` assertions (structure, not exact ids: Leiden
/// randomizes, so only community *equality* is meaningful).
pub fn gds_leiden_communities(
    conn: &Connection,
    graph: &str,
    gamma: Option<f64>,
) -> Result<HashMap<String, u64>> {
    let call = match gamma {
        Some(g) => format!("CALL GDS_LEIDEN('{graph}', gamma := {g})"),
        None => format!("CALL GDS_LEIDEN('{graph}')"),
    };
    let mut result = conn
        .query(&format!("{call} RETURN node, community_id"))
        .with_context(|| format!("GDS_LEIDEN call on '{graph}' failed"))?;
    let mut out = HashMap::new();
    for row in &mut result {
        if row.len() < 2 {
            continue;
        }
        let Some(node) = graph_node_from_value(&row[0]) else {
            continue;
        };
        let community: u64 = match &row[1] {
            Value::Int8(v) => (*v).max(0) as u64,
            Value::Int16(v) => (*v).max(0) as u64,
            Value::Int32(v) => (*v).max(0) as u64,
            Value::Int64(v) => (*v).max(0) as u64,
            Value::Int128(v) => (*v).max(0) as u64,
            Value::UInt8(v) => *v as u64,
            Value::UInt16(v) => *v as u64,
            Value::UInt32(v) => *v as u64,
            Value::UInt64(v) => *v,
            _ => continue,
        };
        out.insert(node.id, community);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn open_ephemeral() -> (tempfile::TempDir, Database) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("arrow_pin.lbdb");
        let db = Database::new(path.to_str().unwrap(), SystemConfig::default()).unwrap();
        (dir, db)
    }

    /// Locks the arrow-55 pin: lbug's `id()` batches must decode to the exact
    /// `(table, offset)` pairs through our types. If lbug ever moves to a new
    /// arrow major (or renames the struct fields), this fails loudly instead
    /// of the loader silently showing 0 nodes.
    #[test]
    fn arrow_internal_id_decode_matches_lbug_batches() {
        let (_dir, db) = open_ephemeral();
        let conn = Connection::new(&db).unwrap();
        conn.query("CREATE NODE TABLE N(id INT64 PRIMARY KEY)")
            .unwrap();
        conn.query("CREATE REL TABLE E(FROM N TO N)").unwrap();
        conn.query("CREATE (a:N{id:0}), (b:N{id:1}), (c:N{id:2})")
            .unwrap();
        conn.query("MATCH (x:N{id:0}), (y:N{id:1}) CREATE (x)-[:E]->(y)")
            .unwrap();
        conn.query("MATCH (x:N{id:1}), (y:N{id:2}) CREATE (x)-[:E]->(y)")
            .unwrap();

        // End-to-end through the loader's Arrow tier.
        let mut links = Vec::new();
        let mut node_ids = HashSet::new();
        collect_edges_arrow(&conn, 10_000, &mut links, &mut node_ids).unwrap();
        assert_eq!(links.len(), 2, "both edges decode: {links:?}");
        assert_eq!(node_ids.len(), 3, "all endpoints decode: {node_ids:?}");

        // And the raw batch shape the decoder relies on.
        let mut result = conn
            .query_as_arrow("MATCH (a)-[r]->(b) RETURN id(a) AS src LIMIT 1", 64)
            .unwrap();
        let batches = result.iter_arrow(64).unwrap();
        let mut saw = 0;
        for batch in batches {
            let col = batch
                .column_by_name("src")
                .and_then(|c| c.as_any().downcast_ref::<arrow::array::StructArray>())
                .expect("src decodes as StructArray with our pinned arrow");
            let ids = decode_internal_id(col);
            assert_eq!(ids.len(), batch.num_rows());
            saw += ids.len();
        }
        assert!(saw > 0);
    }
}
