//! Remote LadybugDB over Arrow Flight / ADBC (Flight SQL prepared flow).
//!
//! Mirrors the `ladybug-adbc-rs` client (`LadybugClient::query_flightsql`):
//! `CreatePreparedStatement` action → `GetFlightInfo` with
//! `CommandPreparedStatementQuery` → `DoGet` → `ClosePreparedStatement`.
//! Falls back to plain Flight (`GetFlightInfo(new_cmd(cypher))` → `DoGet`)
//! when the prepared handshake fails.
//!
//! All entry points are blocking (they spin up a short-lived tokio runtime)
//! so the GPUI view code can call them like the local `lbug` backend.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use anyhow::{Context, Result};
use arrow::array::{Array, RecordBatch, StringArray};
use arrow::datatypes::SchemaRef;
use arrow_flight::decode::FlightRecordBatchStream;
use arrow_flight::flight_service_client::FlightServiceClient;
use arrow_flight::{Action, FlightDescriptor, FlightInfo};
use futures::TryStreamExt;
use tonic::transport::Channel;

use crate::backend::{GraphData, GraphLink, GraphNode};

/// Default demo URI hint shown in the connect box.
pub const DEFAULT_URI_HINT: &str = "grpc://127.0.0.1:50051";

/// Max rows rendered into the shell scrollback per query.
pub const SHELL_MAX_ROWS: usize = 100;

/// True for strings that look like a remote URI (`grpc://…` or `http(s)://…`).
pub fn is_remote_uri(s: &str) -> bool {
    let t = s.trim();
    t.starts_with("grpc://") || t.starts_with("http://") || t.starts_with("https://")
}

fn normalize_uri(uri: &str) -> String {
    if let Some(rest) = uri.strip_prefix("grpc://") {
        format!("http://{rest}")
    } else {
        uri.to_string()
    }
}

// ---------------------------------------------------------------------------
// Minimal Flight SQL protobuf codec (copied from ladybug-adbc-rs so this
// crate stays self-contained without a path dependency).
// ---------------------------------------------------------------------------

const SQL_PKG: &str = "arrow.flight.protocol.sql";

fn encode_varint(mut value: u64) -> Vec<u8> {
    let mut out = Vec::new();
    loop {
        let bits = (value & 0x7F) as u8;
        value >>= 7;
        if value != 0 {
            out.push(bits | 0x80);
        } else {
            out.push(bits);
            break;
        }
    }
    out
}

fn encode_ld(field: u32, payload: &[u8]) -> Vec<u8> {
    let mut out = encode_varint(((field << 3) | 2) as u64);
    out.extend_from_slice(&encode_varint(payload.len() as u64));
    out.extend_from_slice(payload);
    out
}

fn decode_fields(blob: &[u8]) -> Vec<(u32, u8, Vec<u8>)> {
    let mut fields = Vec::new();
    let mut i = 0;
    let n = blob.len();
    while i < n {
        let mut tag: u64 = 0;
        let mut shift = 0;
        let mut j = i;
        let mut ok = false;
        while j < n {
            let b = blob[j];
            j += 1;
            tag |= ((b & 0x7F) as u64) << shift;
            shift += 7;
            if b & 0x80 == 0 {
                ok = true;
                break;
            }
            if shift > 64 {
                break;
            }
        }
        if !ok || j > i + 10 {
            break;
        }
        i = j;
        let wire_type = (tag & 0x7) as u8;
        let field_no = (tag >> 3) as u32;
        match wire_type {
            2 => {
                let mut length: u64 = 0;
                let mut shift = 0;
                let mut j = i;
                let mut ok = false;
                while j < n {
                    let b = blob[j];
                    j += 1;
                    length |= ((b & 0x7F) as u64) << shift;
                    shift += 7;
                    if b & 0x80 == 0 {
                        ok = true;
                        break;
                    }
                    if shift > 64 {
                        break;
                    }
                }
                if !ok || length > 50_000_000 || j + length as usize > n {
                    break;
                }
                i = j;
                fields.push((field_no, wire_type, blob[i..i + length as usize].to_vec()));
                i += length as usize;
            }
            0 => {
                let mut j = i;
                while j < n {
                    let b = blob[j];
                    j += 1;
                    if b & 0x80 == 0 {
                        break;
                    }
                }
                fields.push((field_no, wire_type, blob[i..j].to_vec()));
                i = j;
            }
            1 => {
                if i + 8 > n {
                    break;
                }
                fields.push((field_no, wire_type, blob[i..i + 8].to_vec()));
                i += 8;
            }
            5 => {
                if i + 4 > n {
                    break;
                }
                fields.push((field_no, wire_type, blob[i..i + 4].to_vec()));
                i += 4;
            }
            _ => break,
        }
    }
    fields
}

fn encode_any(type_name: &str, value: &[u8]) -> Vec<u8> {
    let type_url = format!("type.googleapis.com/{SQL_PKG}.{type_name}");
    let mut out = encode_ld(1, type_url.as_bytes());
    out.extend_from_slice(&encode_ld(2, value));
    out
}

fn decode_any(blob: &[u8]) -> (String, Vec<u8>) {
    let mut type_url = String::new();
    let mut value = Vec::new();
    for (field_no, wire, payload) in decode_fields(blob) {
        if field_no == 1 && wire == 2 {
            if let Ok(s) = String::from_utf8(payload.clone()) {
                type_url = s;
            }
        } else if field_no == 2 && wire == 2 {
            value = payload;
        }
    }
    (type_url, value)
}

fn get_bytes(fields: &[(u32, u8, Vec<u8>)], field_no: u32) -> Vec<u8> {
    for (f, w, p) in fields {
        if *f == field_no && *w == 2 {
            return p.clone();
        }
    }
    Vec::new()
}

fn any_command_prepared_statement_query(handle: &[u8]) -> Vec<u8> {
    encode_any("CommandPreparedStatementQuery", &encode_ld(1, handle))
}

fn action_create_prepared_statement_request(query: &str) -> Vec<u8> {
    encode_any(
        "ActionCreatePreparedStatementRequest",
        &encode_ld(1, query.as_bytes()),
    )
}

fn parse_create_prepared_statement_result(body: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let (type_url, value) = decode_any(body);
    let inner = if !value.is_empty()
        && (type_url.contains("CreatePreparedStatementResult") || !type_url.is_empty())
    {
        value
    } else {
        body.to_vec()
    };
    let fields = decode_fields(&inner);
    (get_bytes(&fields, 1), get_bytes(&fields, 2))
}

fn action_close_prepared_statement_request(handle: &[u8]) -> Vec<u8> {
    encode_ld(1, handle)
}

// ---------------------------------------------------------------------------
// Async Flight client
// ---------------------------------------------------------------------------

async fn connect(uri: &str) -> Result<FlightServiceClient<Channel>> {
    let endpoint = normalize_uri(uri);
    FlightServiceClient::connect(endpoint)
        .await
        .context("flight connect failed")
}

async fn read_endpoint(
    client: &mut FlightServiceClient<Channel>,
    info: &FlightInfo,
) -> Result<Vec<RecordBatch>> {
    let mut batches = Vec::new();
    for endpoint in &info.endpoint {
        let ticket = endpoint
            .ticket
            .clone()
            .ok_or_else(|| anyhow::anyhow!("flight endpoint missing ticket"))?;
        let resp = client.do_get(ticket).await?;
        let mut stream = FlightRecordBatchStream::new_from_flight_data(
            resp.into_inner()
                .map_err(arrow_flight::error::FlightError::from),
        );
        while let Some(batch) = stream.try_next().await? {
            batches.push(batch);
        }
    }
    Ok(batches)
}

async fn query_flightsql_async(uri: &str, cypher: &str) -> Result<(SchemaRef, Vec<RecordBatch>)> {
    let mut client = connect(uri).await?;
    let mut action_stream = client
        .do_action(Action {
            r#type: "CreatePreparedStatement".to_string(),
            body: action_create_prepared_statement_request(cypher).into(),
        })
        .await?
        .into_inner();
    let mut bodies = Vec::new();
    while let Some(result) = action_stream.message().await? {
        bodies.push(result.body.to_vec());
    }
    let Some(first) = bodies.into_iter().next() else {
        anyhow::bail!("CreatePreparedStatement returned no result");
    };
    let (handle, _) = parse_create_prepared_statement_result(&first);
    if handle.is_empty() {
        anyhow::bail!("CreatePreparedStatement returned empty handle");
    }
    let info = client
        .get_flight_info(FlightDescriptor::new_cmd(
            any_command_prepared_statement_query(&handle),
        ))
        .await?
        .into_inner();
    let schema: SchemaRef = Arc::new(arrow::datatypes::Schema::try_from(info.clone())?);
    let batches = read_endpoint(&mut client, &info).await?;
    let mut close_stream = client
        .do_action(Action {
            r#type: "ClosePreparedStatement".to_string(),
            body: action_close_prepared_statement_request(&handle).into(),
        })
        .await?
        .into_inner();
    while close_stream.message().await?.is_some() {}
    Ok((schema, batches))
}

async fn query_flight_async(uri: &str, cypher: &str) -> Result<(SchemaRef, Vec<RecordBatch>)> {
    let mut client = connect(uri).await?;
    let info = client
        .get_flight_info(FlightDescriptor::new_cmd(cypher.as_bytes().to_vec()))
        .await?
        .into_inner();
    let schema: SchemaRef = Arc::new(arrow::datatypes::Schema::try_from(info.clone())?);
    let batches = read_endpoint(&mut client, &info).await?;
    Ok((schema, batches))
}

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("tokio runtime")
        .block_on(f)
}

/// Run Cypher over the Flight SQL prepared flow, falling back to plain
/// Flight when the handshake fails. Returns native Arrow batches.
pub fn query_cypher(uri: &str, cypher: &str) -> Result<(SchemaRef, Vec<RecordBatch>)> {
    let uri = uri.trim().to_string();
    let cypher = cypher.trim().to_string();
    if cypher.is_empty() {
        anyhow::bail!("empty query");
    }
    match block_on(query_flightsql_async(&uri, &cypher)) {
        Ok(out) => Ok(out),
        Err(_) => block_on(query_flight_async(&uri, &cypher)),
    }
}

// ---------------------------------------------------------------------------
// Arrow decoding helpers (server speaks arrow 55, same as this crate)
// ---------------------------------------------------------------------------

type NodeId = (u64, u64);

fn decode_internal_id(col: &arrow::array::StructArray) -> Vec<NodeId> {
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
    offsets
        .into_iter()
        .zip(tables)
        .map(|(o, t)| (t.max(0) as u64, o.max(0) as u64))
        .collect()
}

fn string_col(batch: &RecordBatch, name: &str) -> Vec<String> {
    if let Some(col) = batch.column_by_name(name) {
        if let Some(a) = col.as_any().downcast_ref::<StringArray>() {
            return a.iter().map(|v| v.unwrap_or("").to_string()).collect();
        }
        if let Some(a) = col
            .as_any()
            .downcast_ref::<arrow::array::LargeStringArray>()
        {
            return a.iter().map(|v| v.unwrap_or("").to_string()).collect();
        }
    }
    Vec::new()
}

fn struct_col_ids(batch: &RecordBatch, name: &str) -> Vec<NodeId> {
    batch
        .column_by_name(name)
        .and_then(|c| c.as_any().downcast_ref::<arrow::array::StructArray>())
        .map(decode_internal_id)
        .unwrap_or_default()
}

/// `CALL SHOW_TABLES()` over Flight → `(table_id, name, kind)`.
pub fn remote_table_catalog(uri: &str) -> Result<Vec<(u64, String, String)>> {
    let (_schema, batches) = query_cypher(uri, "CALL SHOW_TABLES() RETURN *")?;
    let mut out = Vec::new();
    for batch in &batches {
        let n = batch.num_rows();
        let ids: Vec<u64> = batch
            .column_by_name("id")
            .map(|c| {
                if let Some(a) = c.as_any().downcast_ref::<arrow::array::UInt64Array>() {
                    a.iter().map(|v| v.unwrap_or(0)).collect()
                } else if let Some(a) = c.as_any().downcast_ref::<arrow::array::Int64Array>() {
                    a.iter().map(|v| v.unwrap_or(0).max(0) as u64).collect()
                } else {
                    vec![0; n]
                }
            })
            .unwrap_or_else(|| vec![0; n]);
        let names = string_col(batch, "name");
        let kinds = string_col(batch, "type");
        // Some servers name the kind column `kind`.
        let kinds = if kinds.is_empty() {
            string_col(batch, "kind")
        } else {
            kinds
        };
        for i in 0..n {
            let name = names.get(i).cloned().unwrap_or_default();
            let kind = kinds.get(i).cloned().unwrap_or_default();
            if name.is_empty() {
                continue;
            }
            out.push((*ids.get(i).unwrap_or(&0), name, kind));
        }
    }
    Ok(out)
}

/// Edge scan over Flight: `RETURN id(a) AS src, id(b) AS dst, label(r)…`.
fn remote_edge_scan(uri: &str, limit: usize) -> Result<(Vec<GraphLink>, HashSet<NodeId>)> {
    let (_schema, batches) = query_cypher(
        uri,
        &format!(
            "MATCH (a)-[r]->(b) RETURN id(a) AS src, id(b) AS dst, label(r) AS rel LIMIT {limit}"
        ),
    )?;
    let mut links = Vec::new();
    let mut node_ids = HashSet::new();
    let mut seen = HashSet::new();
    for batch in &batches {
        let srcs = struct_col_ids(batch, "src");
        let dsts = struct_col_ids(batch, "dst");
        let rels = string_col(batch, "rel");
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
                    properties: HashMap::new(),
                });
            }
        }
    }
    Ok((links, node_ids))
}

/// Full graph load over Flight: catalog + edge scan, nodes seeded with
/// `id + label` (no per-node property fetch — that stays a local-only
/// enrichment; the graph still lays out and focuses correctly).
pub fn remote_collect_edge_graph(uri: &str, limit: usize) -> Result<GraphData> {
    let catalog = remote_table_catalog(uri).unwrap_or_default();
    let label_of_table: HashMap<u64, String> = catalog
        .iter()
        .filter(|(_, _, kind)| {
            kind.eq_ignore_ascii_case("NODE") || kind.eq_ignore_ascii_case("node")
        })
        .map(|(id, name, _)| (*id, name.clone()))
        .collect();
    let (links, node_ids) = remote_edge_scan(uri, limit)?;
    if links.is_empty() {
        anyhow::bail!("remote edge scan returned no edges");
    }
    let nodes = node_ids
        .into_iter()
        .map(|(t, o)| {
            let id = format!("{t}:{o}");
            let label = label_of_table.get(&t).cloned().unwrap_or_default();
            GraphNode {
                name: id.clone(),
                id: id.clone(),
                label,
                properties: HashMap::new(),
            }
        })
        .collect();
    Ok(GraphData { nodes, links })
}

/// Schema graph over Flight: node tables become nodes, one sampled edge per
/// rel table resolves connectivity (all scalar queries — no structs).
pub fn remote_collect_schema_graph(uri: &str) -> Result<GraphData> {
    let catalog = remote_table_catalog(uri)?;
    let node_tables: Vec<String> = catalog
        .iter()
        .filter(|(_, _, kind)| kind.eq_ignore_ascii_case("NODE"))
        .map(|(_, name, _)| name.clone())
        .collect();
    let rel_tables: Vec<String> = catalog
        .iter()
        .filter(|(_, _, kind)| kind.eq_ignore_ascii_case("REL"))
        .map(|(_, name, _)| name.clone())
        .collect();
    let mut links = Vec::new();
    let mut seen = HashSet::new();
    for rel in &rel_tables {
        let esc = rel.replace('`', "``");
        let q = format!("MATCH (a)-[r:`{esc}`]->(b) RETURN label(a) AS s, label(b) AS t LIMIT 1");
        let Ok((_schema, batches)) = query_cypher(uri, &q) else {
            continue;
        };
        for batch in &batches {
            let ss = string_col(batch, "s");
            let ts = string_col(batch, "t");
            for (s, t) in ss.into_iter().zip(ts) {
                if s.is_empty() || t.is_empty() {
                    continue;
                }
                if seen.insert((s.clone(), t.clone(), rel.clone())) {
                    links.push(GraphLink {
                        source: format!("schema:{s}"),
                        target: format!("schema:{t}"),
                        label: rel.clone(),
                        properties: HashMap::new(),
                    });
                }
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

// ---------------------------------------------------------------------------
// Shell table formatting
// ---------------------------------------------------------------------------

/// Render Arrow batches as an `lbug`-style box table, capped at `max_rows`.
/// Pure function over owned strings so the shell can show local and remote
/// results through the same path.
pub fn format_table(
    columns: &[String],
    rows: &[Vec<String>],
    total_rows: usize,
    max_rows: usize,
) -> String {
    if columns.is_empty() {
        return "(0 columns)\n(0 tuples)\n".to_string();
    }
    let shown = rows.len().min(max_rows);
    let mut widths: Vec<usize> = columns.iter().map(|c| c.chars().count()).collect();
    for row in rows.iter().take(shown) {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.chars().count().min(60));
            }
        }
    }
    let trunc = |s: &str, w: usize| -> String {
        let n = s.chars().count();
        if n <= w {
            format!("{s}{}", " ".repeat(w - n))
        } else if w <= 1 {
            "…".to_string()
        } else {
            format!("{}…", s.chars().take(w - 1).collect::<String>())
        }
    };
    let mut out = String::new();
    let border = format!(
        "┌{}┐\n",
        widths
            .iter()
            .map(|w| "─".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("┬")
    );
    out.push_str(&border);
    out.push_str(&format!(
        "│{}│\n",
        columns
            .iter()
            .enumerate()
            .map(|(i, c)| format!(" {} ", trunc(c, widths[i])))
            .collect::<Vec<_>>()
            .join("│")
    ));
    out.push_str(&format!(
        "├{}┤\n",
        widths
            .iter()
            .map(|w| "─".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("┼")
    ));
    for row in rows.iter().take(shown) {
        let mut cells = Vec::new();
        for (i, w) in widths.iter().enumerate().take(columns.len()) {
            let c = row.get(i).map(|s| s.as_str()).unwrap_or("");
            cells.push(format!(" {} ", trunc(c, *w)));
        }
        out.push_str(&format!("│{}│\n", cells.join("│")));
    }
    out.push_str(&format!(
        "└{}┘\n",
        widths
            .iter()
            .map(|w| "─".repeat(w + 2))
            .collect::<Vec<_>>()
            .join("┴")
    ));
    if total_rows > shown {
        out.push_str(&format!("({total_rows} tuples, showing first {shown})\n"));
    } else {
        out.push_str(&format!("({total_rows} tuples)\n"));
    }
    out.push_str(&format!("({} columns)\n", columns.len()));
    out
}

/// Arrow batches → box table, capped at `max_rows` for display.
pub fn format_batches(schema: &SchemaRef, batches: &[RecordBatch], max_rows: usize) -> String {
    let columns: Vec<String> = schema.fields().iter().map(|f| f.name().clone()).collect();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut total = 0usize;
    for batch in batches {
        total += batch.num_rows();
        if rows.len() >= max_rows {
            continue;
        }
        for r in 0..batch.num_rows() {
            if rows.len() >= max_rows {
                break;
            }
            let mut row = Vec::new();
            for c in 0..batch.num_columns() {
                row.push(cell_string(batch.column(c).as_ref(), r));
            }
            rows.push(row);
        }
    }
    format_table(&columns, &rows, total, max_rows)
}

/// One Arrow cell as text (best-effort across common types).
fn cell_string(col: &dyn Array, row: usize) -> String {
    use arrow::array::*;
    use arrow::datatypes::DataType;
    match col.data_type() {
        DataType::Utf8 => col
            .as_any()
            .downcast_ref::<StringArray>()
            .map(|a| {
                if a.is_null(row) {
                    String::new()
                } else {
                    a.value(row).to_string()
                }
            })
            .unwrap_or_default(),
        DataType::LargeUtf8 => col
            .as_any()
            .downcast_ref::<LargeStringArray>()
            .map(|a| {
                if a.is_null(row) {
                    String::new()
                } else {
                    a.value(row).to_string()
                }
            })
            .unwrap_or_default(),
        _ => {
            // Generic fallback: probe numeric/bool arrays, else debug.
            if let Some(a) = col.as_any().downcast_ref::<Int64Array>() {
                return if a.is_null(row) {
                    String::new()
                } else {
                    a.value(row).to_string()
                };
            }
            if let Some(a) = col.as_any().downcast_ref::<Int32Array>() {
                return if a.is_null(row) {
                    String::new()
                } else {
                    a.value(row).to_string()
                };
            }
            if let Some(a) = col.as_any().downcast_ref::<UInt64Array>() {
                return if a.is_null(row) {
                    String::new()
                } else {
                    a.value(row).to_string()
                };
            }
            if let Some(a) = col.as_any().downcast_ref::<Float64Array>() {
                return if a.is_null(row) {
                    String::new()
                } else {
                    a.value(row).to_string()
                };
            }
            if let Some(a) = col.as_any().downcast_ref::<BooleanArray>() {
                return if a.is_null(row) {
                    String::new()
                } else {
                    a.value(row).to_string()
                };
            }
            format!("{col:?}")
                .lines()
                .nth(row)
                .unwrap_or("")
                .to_string()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_remote_uris() {
        assert!(is_remote_uri("grpc://127.0.0.1:50051"));
        assert!(is_remote_uri("http://host:50051"));
        assert!(!is_remote_uri("/tmp/graph.lbdb"));
        assert!(!is_remote_uri("graph.lbdb"));
    }

    #[test]
    fn formats_empty_table() {
        let s = format_table(&[], &[], 0, 20);
        assert!(s.contains("0 tuples"));
    }

    #[test]
    fn formats_small_table() {
        let s = format_table(
            &["name".to_string(), "age".to_string()],
            &[vec!["Alice".to_string(), "30".to_string()]],
            1,
            20,
        );
        assert!(s.contains("Alice"));
        assert!(s.contains("1 tuples"));
    }
}
