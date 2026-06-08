use arrow_array::{builder::UInt64Builder, RecordBatch, UInt64Array};
use arrow_ipc::writer::StreamWriter;
use arrow_schema::{DataType, Field, Schema};
#[cfg(feature = "icebug-analytics")]
use icebug::{GraphR, Leiden};
use lbug::{Connection, Database, SystemConfig, Value};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::Mutex;
use std::time::Duration;
use tauri::{Manager, State};
use walkdir::WalkDir;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DatabaseInfo {
    id: usize,
    name: String,
    path: String,
    #[serde(rename = "relativePath")]
    relative_path: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphNode {
    id: String,
    name: String,
    label: String,
    #[serde(skip_serializing_if = "HashMap::is_empty", default)]
    properties: HashMap<String, String>,
    #[serde(rename = "tableId", skip_serializing_if = "Option::is_none")]
    table_id: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    rowid: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    community: Option<u64>,
    #[serde(rename = "expansionKind", skip_serializing_if = "Option::is_none")]
    expansion_kind: Option<String>,
    #[serde(rename = "expandNodeId", skip_serializing_if = "Option::is_none")]
    expand_node_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    offset: Option<usize>,
    #[serde(rename = "hiddenCount", skip_serializing_if = "Option::is_none")]
    hidden_count: Option<usize>,
    #[serde(rename = "colorKey", skip_serializing_if = "Option::is_none")]
    color_key: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphLink {
    source: String,
    target: String,
    label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphCsr {
    indptr: Vec<u64>,
    indices: Vec<u64>,
    #[serde(rename = "edgeIds")]
    edge_ids: Option<Vec<u64>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphCluster {
    #[serde(rename = "clusterId")]
    cluster_id: u64,
    label: String,
    size: usize,
    #[serde(rename = "parentClusterId", skip_serializing_if = "Option::is_none")]
    parent_cluster_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphClusterLevel {
    level: usize,
    membership: Vec<u64>,
    clusters: Vec<GraphCluster>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphClusterDebug {
    enabled: bool,
    status: String,
    message: String,
    #[serde(rename = "nodeCount")]
    node_count: usize,
    #[serde(rename = "edgeCount")]
    edge_count: usize,
    #[serde(rename = "undirectedEdgeCount")]
    undirected_edge_count: usize,
    levels: usize,
    clusters: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphData {
    nodes: Vec<GraphNode>,
    links: Vec<GraphLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    csr: Option<GraphCsr>,
    #[serde(rename = "csrArrowIpc", skip_serializing_if = "Option::is_none")]
    csr_arrow_ipc: Option<Vec<u8>>,
    #[serde(rename = "clusterLevels", skip_serializing_if = "Option::is_none")]
    cluster_levels: Option<Vec<GraphClusterLevel>>,
    #[serde(rename = "clusterDebug")]
    cluster_debug: GraphClusterDebug,
}

#[derive(Debug, Clone, Deserialize)]
#[allow(dead_code)]
struct LlmClusterNamingConfig {
    #[serde(rename = "accessToken")]
    access_token: String,
    endpoint: Option<String>,
    model: Option<String>,
    #[serde(rename = "sampleSize")]
    sample_size: Option<usize>,
}

#[derive(Debug, Clone, Deserialize)]
struct LlmClusterNameRequest {
    key: String,
    #[serde(rename = "clusterId")]
    cluster_id: u64,
    labels: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
struct LlmClusterNameResult {
    key: String,
    name: Option<String>,
    error: Option<String>,
}

impl LlmClusterNamingConfig {
    fn enabled(&self) -> bool {
        !self.access_token.trim().is_empty()
    }

    fn endpoint(&self) -> String {
        let endpoint = self
            .endpoint
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("https://openrouter.ai")
            .trim_end_matches('/')
            .to_string();

        if endpoint.ends_with("/chat/completions") {
            endpoint
        } else if endpoint.ends_with("/v1") || endpoint.ends_with("/api/v1") {
            format!("{endpoint}/chat/completions")
        } else {
            format!("{endpoint}/api/v1/chat/completions")
        }
    }

    fn model(&self) -> String {
        self.model
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .unwrap_or("deepseek-v4-flash")
            .to_string()
    }

    fn sample_size(&self) -> usize {
        self.sample_size.unwrap_or(LLM_CLUSTER_SAMPLE_SIZE).max(1)
    }
}

const SEED_NODE_COUNT: usize = 8;
const EXPAND_BATCH_SIZE: usize = 8;
const EDGE_SCAN_LIMIT: usize = 10_000;
const NODE_SEARCH_SCAN_LIMIT: usize = 10_000;
const SEARCH_RESULT_LIMIT: usize = 30;
const SEARCH_NEIGHBOR_LIMIT: usize = 24;
#[cfg(feature = "icebug-analytics")]
const CLUSTER_LEVEL_LIMIT: usize = 3;
const LLM_CLUSTER_SAMPLE_SIZE: usize = 15;
const LLM_CLUSTER_NAME_LIMIT: usize = 24;
const EXPANDER_PREFIX: &str = "__expand__:";

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DirEntry {
    name: String,
    path: String,
    #[serde(rename = "type")]
    entry_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct DirectoryListing {
    current: String,
    parent: String,
    directories: Vec<DirEntry>,
    files: Vec<DirEntry>,
}

struct AppState {
    custom_databases: Mutex<Vec<DatabaseInfo>>,
    initial_database_path: Option<String>,
    data_dir: PathBuf,
}

fn scan_for_databases(dir: &Path, base_dir: &Path) -> Vec<DatabaseInfo> {
    let mut databases = Vec::new();
    for entry in WalkDir::new(dir).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext == "lbdb" {
                    let name = path
                        .file_stem()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .to_string();
                    let relative_path = path
                        .strip_prefix(base_dir)
                        .unwrap_or(path)
                        .to_string_lossy()
                        .to_string();
                    databases.push(DatabaseInfo {
                        id: 0, // assigned later
                        name,
                        path: path.to_string_lossy().to_string(),
                        relative_path,
                    });
                }
            }
        }
    }
    databases
}

fn get_all_databases(state: &AppState) -> Vec<DatabaseInfo> {
    let scanned = scan_for_databases(&state.data_dir, &state.data_dir);
    let custom = state.custom_databases.lock().unwrap();
    let mut all: Vec<DatabaseInfo> = scanned.into_iter().chain(custom.iter().cloned()).collect();
    for (i, db) in all.iter_mut().enumerate() {
        db.id = i;
    }
    all
}

fn database_info_from_path(file_path: &str) -> Result<DatabaseInfo, String> {
    if file_path.is_empty() {
        return Err("filePath is required".to_string());
    }

    let abs_path = if Path::new(file_path).is_absolute() {
        PathBuf::from(file_path)
    } else {
        std::env::current_dir().unwrap_or_default().join(file_path)
    };

    if !abs_path.exists() {
        return Err("File not found".to_string());
    }

    if abs_path.extension().and_then(|e| e.to_str()) != Some("lbdb") {
        return Err("Only .lbdb files are supported".to_string());
    }

    let abs_path_str = abs_path.to_string_lossy().to_string();
    let name = abs_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    Ok(DatabaseInfo {
        id: 0,
        name,
        path: abs_path_str.clone(),
        relative_path: abs_path_str,
    })
}

fn value_to_string(val: &Value) -> String {
    match val {
        Value::String(s) => s.clone(),
        Value::Int64(n) => n.to_string(),
        Value::Int32(n) => n.to_string(),
        Value::Int16(n) => n.to_string(),
        Value::Int8(n) => n.to_string(),
        Value::UInt64(n) => n.to_string(),
        Value::UInt32(n) => n.to_string(),
        Value::UInt16(n) => n.to_string(),
        Value::UInt8(n) => n.to_string(),
        Value::Double(n) => n.to_string(),
        Value::Float(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => format!("{}", val),
    }
}

fn non_empty_value_to_string(val: &Value) -> Option<String> {
    let value = value_to_string(val);
    if value.trim().is_empty() {
        None
    } else {
        Some(value)
    }
}

fn node_display_name(props: &[(String, Value)]) -> String {
    const PREFERRED_KEYS: &[&str] = &[
        "name",
        "title",
        "display_name",
        "displayname",
        "label",
        "summary",
        "subject",
        "key",
        "slug",
        "path",
        "filename",
        "file",
        "url",
        "uri",
        "email",
        "username",
        "user_name",
    ];

    for preferred_key in PREFERRED_KEYS {
        if let Some((_, value)) = props
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(preferred_key))
        {
            if let Some(name) = non_empty_value_to_string(value) {
                return name;
            }
        }
    }

    if let Some((_, value)) = props
        .iter()
        .find(|(key, value)| !key.eq_ignore_ascii_case("id") && matches!(value, Value::String(_)))
    {
        if let Some(name) = non_empty_value_to_string(value) {
            return name;
        }
    }

    if let Some((_, value)) = props
        .iter()
        .find(|(key, value)| key.eq_ignore_ascii_case("id") && matches!(value, Value::String(_)))
    {
        if let Some(name) = non_empty_value_to_string(value) {
            return name;
        }
    }

    props
        .iter()
        .find_map(|(_, value)| non_empty_value_to_string(value))
        .unwrap_or_else(|| "Node".to_string())
}

fn node_display_properties(props: &[(String, Value)]) -> HashMap<String, String> {
    props
        .iter()
        .filter_map(|(key, value)| {
            non_empty_value_to_string(value).map(|value| (key.clone(), value))
        })
        .collect()
}

fn value_to_score(val: &Value) -> f64 {
    match val {
        Value::Double(n) => *n,
        Value::Float(n) => *n as f64,
        Value::Int64(n) => *n as f64,
        Value::Int32(n) => *n as f64,
        Value::Int16(n) => *n as f64,
        Value::Int8(n) => *n as f64,
        _ => value_to_string(val).parse().unwrap_or(0.0),
    }
}

fn cypher_string(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

fn graph_node_from_value(val: &Value) -> Option<GraphNode> {
    let Value::Node(node_val) = val else {
        return None;
    };

    let props = node_val.get_properties();
    let name = node_display_name(props);
    let properties = node_display_properties(props);

    Some(GraphNode {
        id: id_to_string(node_val.get_node_id()),
        name,
        label: node_val.get_label_name().clone(),
        properties,
        table_id: Some(node_val.get_node_id().table_id),
        rowid: Some(node_val.get_node_id().offset),
        community: None,
        expansion_kind: None,
        expand_node_id: None,
        offset: None,
        hidden_count: None,
        color_key: None,
    })
}

fn node_value_matches_query(val: &Value, query: &str) -> bool {
    let Value::Node(node_val) = val else {
        return false;
    };

    let node_id = id_to_string(node_val.get_node_id());
    let label = node_val.get_label_name();
    let props = node_val.get_properties();
    node_id.to_lowercase().contains(query)
        || label.to_lowercase().contains(query)
        || props.iter().any(|(key, prop_val)| {
            key.to_lowercase().contains(query)
                || value_to_string(prop_val).to_lowercase().contains(query)
        })
}

fn available_fts_indexes(conn: &Connection) -> Vec<(String, String)> {
    let Ok(mut result) = conn.query("CALL SHOW_INDEXES() RETURN *") else {
        return Vec::new();
    };

    let mut indexes = Vec::new();
    for row in &mut result {
        if row.len() < 3 {
            continue;
        }

        let table_name = value_to_string(&row[0]);
        let index_name = value_to_string(&row[1]);
        let index_type = value_to_string(&row[2]);
        let extension_loaded = row
            .get(4)
            .map(|val| matches!(val, Value::Bool(true)))
            .unwrap_or(true);

        if index_type.eq_ignore_ascii_case("FTS") && extension_loaded {
            indexes.push((table_name, index_name));
        }
    }
    indexes
}

fn search_nodes_with_fts(conn: &Connection, query: &str) -> Option<Vec<GraphNode>> {
    let indexes = available_fts_indexes(conn);
    if indexes.is_empty() {
        return None;
    }

    let query_literal = cypher_string(query);
    let mut scored_nodes: Vec<(GraphNode, f64)> = Vec::new();
    let mut seen = HashSet::new();

    for (table_name, index_name) in indexes {
        let cypher = format!(
            "CALL QUERY_FTS_INDEX({}, {}, {}, top := {}) RETURN node, score ORDER BY score DESC",
            cypher_string(&table_name),
            cypher_string(&index_name),
            query_literal,
            SEARCH_RESULT_LIMIT,
        );
        let Ok(mut result) = conn.query(&cypher) else {
            continue;
        };

        for row in &mut result {
            let Some(node) = row.first().and_then(graph_node_from_value) else {
                continue;
            };
            if !seen.insert(node.id.clone()) {
                continue;
            }
            let score = row.get(1).map(value_to_score).unwrap_or(0.0);
            scored_nodes.push((node, score));
        }
    }

    scored_nodes.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    Some(
        scored_nodes
            .into_iter()
            .map(|(node, _)| node)
            .take(SEARCH_RESULT_LIMIT)
            .collect(),
    )
}

fn id_to_string(id: &lbug::InternalID) -> String {
    format!("{}:{}", id.table_id, id.offset)
}

fn make_expander_node(parent_id: &str, hidden_count: usize, offset: usize) -> GraphNode {
    GraphNode {
        id: format!("{EXPANDER_PREFIX}node:{parent_id}:{offset}"),
        name: format!("+{hidden_count}"),
        label: "More".to_string(),
        properties: HashMap::new(),
        table_id: None,
        rowid: None,
        community: None,
        expansion_kind: Some("node".to_string()),
        expand_node_id: Some(parent_id.to_string()),
        offset: Some(offset),
        hidden_count: Some(hidden_count),
        color_key: None,
    }
}

fn merge_node(nodes: &mut HashMap<String, GraphNode>, node: GraphNode) {
    nodes.entry(node.id.clone()).or_insert(node);
}

fn merge_link(
    links: &mut Vec<GraphLink>,
    seen: &mut HashSet<(String, String, String)>,
    link: GraphLink,
) {
    let key = (link.source.clone(), link.target.clone(), link.label.clone());
    if seen.insert(key) {
        links.push(link);
    }
}

fn build_csr(nodes: &[GraphNode], links: &[GraphLink]) -> GraphCsr {
    let node_index: HashMap<&str, usize> = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.as_str(), index))
        .collect();
    let mut outgoing: Vec<Vec<(usize, usize)>> = vec![Vec::new(); nodes.len()];

    for (edge_index, link) in links.iter().enumerate() {
        let Some(&source) = node_index.get(link.source.as_str()) else {
            continue;
        };
        let Some(&target) = node_index.get(link.target.as_str()) else {
            continue;
        };
        outgoing[source].push((target, edge_index));
    }

    let mut indptr = Vec::with_capacity(nodes.len() + 1);
    let mut indices = Vec::new();
    let mut edge_ids = Vec::new();

    for neighbors in outgoing {
        indptr.push(indices.len() as u64);
        for (target, edge_index) in neighbors {
            indices.push(target as u64);
            edge_ids.push(edge_index as u64);
        }
    }
    indptr.push(indices.len() as u64);

    GraphCsr {
        indptr,
        indices,
        edge_ids: Some(edge_ids),
    }
}

fn nullable_u64_column(values: &[u64], row_count: usize) -> UInt64Array {
    let mut builder = UInt64Builder::with_capacity(row_count);
    for index in 0..row_count {
        if let Some(value) = values.get(index) {
            builder.append_value(*value);
        } else {
            builder.append_null();
        }
    }
    builder.finish()
}

fn csr_to_arrow_ipc(csr: &GraphCsr) -> Result<Vec<u8>, String> {
    let row_count = [
        csr.indptr.len(),
        csr.indices.len(),
        csr.edge_ids.as_ref().map(|items| items.len()).unwrap_or(0),
    ]
    .into_iter()
    .max()
    .unwrap_or(0);
    let edge_ids = csr.edge_ids.as_deref().unwrap_or(&[]);
    let schema = Arc::new(Schema::new(vec![
        Field::new("indptr", DataType::UInt64, true),
        Field::new("indices", DataType::UInt64, true),
        Field::new("edge_ids", DataType::UInt64, true),
    ]));
    let batch = RecordBatch::try_new(
        schema.clone(),
        vec![
            Arc::new(nullable_u64_column(&csr.indptr, row_count)),
            Arc::new(nullable_u64_column(&csr.indices, row_count)),
            Arc::new(nullable_u64_column(edge_ids, row_count)),
        ],
    )
    .map_err(|e| format!("Failed to build Arrow CSR batch: {e}"))?;

    let mut output = Cursor::new(Vec::new());
    {
        let mut writer = StreamWriter::try_new(&mut output, &schema)
            .map_err(|e| format!("Failed to create Arrow CSR writer: {e}"))?;
        writer
            .write(&batch)
            .map_err(|e| format!("Failed to write Arrow CSR batch: {e}"))?;
        writer
            .finish()
            .map_err(|e| format!("Failed to finish Arrow CSR stream: {e}"))?;
    }
    Ok(output.into_inner())
}

#[cfg(feature = "icebug-analytics")]
fn build_undirected_csr(
    node_count: usize,
    links: &[GraphLink],
    node_index: &HashMap<String, usize>,
) -> GraphCsr {
    let mut outgoing: Vec<Vec<usize>> = vec![Vec::new(); node_count];
    for link in links {
        let Some(&source) = node_index.get(&link.source) else {
            continue;
        };
        let Some(&target) = node_index.get(&link.target) else {
            continue;
        };
        if source == target {
            continue;
        }
        outgoing[source].push(target);
        outgoing[target].push(source);
    }

    let mut indptr = Vec::with_capacity(node_count + 1);
    let mut indices = Vec::new();
    for mut neighbors in outgoing {
        neighbors.sort_unstable();
        neighbors.dedup();
        indptr.push(indices.len() as u64);
        indices.extend(neighbors.into_iter().map(|target| target as u64));
    }
    indptr.push(indices.len() as u64);

    GraphCsr {
        indptr,
        indices,
        edge_ids: None,
    }
}

#[cfg(feature = "icebug-analytics")]
fn leiden_membership(node_count: usize, csr: &GraphCsr) -> Result<Vec<u64>, String> {
    if node_count == 0 {
        return Ok(Vec::new());
    }
    if csr.indices.is_empty() {
        return Ok((0..node_count as u64).collect());
    }

    let graph = GraphR::from_csr(
        node_count as u64,
        false,
        UInt64Array::from(csr.indices.clone()),
        UInt64Array::from(csr.indptr.clone()),
    )
    .map_err(|e| format!("Failed to create Icebug CSR graph: {e}"))?;
    let mut leiden = Leiden::new(&graph, 3, true, 1.0)
        .map_err(|e| format!("Failed to create Leiden clustering: {e}"))?;
    leiden
        .run()
        .map_err(|e| format!("Leiden clustering failed: {e}"))?;
    let partition = leiden
        .partition()
        .map_err(|e| format!("Failed to read Leiden partition: {e}"))?;
    Ok(partition.membership)
}

#[cfg(feature = "icebug-analytics")]
fn remap_membership(membership: &[u64]) -> (Vec<u64>, HashMap<u64, u64>) {
    let mut ids: Vec<u64> = membership.to_vec();
    ids.sort_unstable();
    ids.dedup();
    let remap: HashMap<u64, u64> = ids
        .into_iter()
        .enumerate()
        .map(|(index, id)| (id, index as u64))
        .collect();
    let mapped = membership
        .iter()
        .map(|id| *remap.get(id).unwrap_or(&0))
        .collect();
    (mapped, remap)
}

#[cfg(feature = "icebug-analytics")]
fn node_cluster_label(node: &GraphNode) -> String {
    let label = node.label.trim();
    let name = node.name.trim();
    if name.is_empty() || name == label {
        label.to_string()
    } else {
        format!("{label}: {name}")
    }
}

fn clean_llm_cluster_name(value: &str) -> Option<String> {
    let name = value
        .lines()
        .next()
        .unwrap_or("")
        .trim()
        .trim_matches(['"', '\'', '.', ':', '-'])
        .trim();
    if name.is_empty() {
        return None;
    }
    Some(name.chars().take(80).collect())
}

fn llm_cluster_name(
    config: &LlmClusterNamingConfig,
    cluster_id: u64,
    sample_labels: &[String],
) -> Result<Option<String>, String> {
    if !config.enabled() || sample_labels.is_empty() {
        return Ok(None);
    }

    let label_list = sample_labels
        .iter()
        .map(|label| format!("- {label}"))
        .collect::<Vec<_>>()
        .join("\n");
    let body = json!({
        "model": config.model(),
        "messages": [
            {
                "role": "system",
                "content": "Name a graph cluster from sampled node labels. Return only a concise 2-5 word name, with no punctuation, quotes, or explanation."
            },
            {
                "role": "user",
                "content": format!("Cluster {cluster_id} sample labels:\n{label_list}")
            }
        ],
        "temperature": 0.2,
        "max_tokens": 24
    });

    let response: serde_json::Value = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()
        .map_err(|err| format!("Failed to create LLM client: {err}"))?
        .post(config.endpoint())
        .bearer_auth(config.access_token.trim())
        .json(&body)
        .send()
        .map_err(|err| format!("LLM request failed: {err}"))?
        .error_for_status()
        .map_err(|err| format!("LLM request returned an error: {err}"))?
        .json()
        .map_err(|err| format!("LLM response was not valid JSON: {err}"))?;

    let content = response
        .get("choices")
        .and_then(|choices| choices.get(0))
        .and_then(|choice| choice.get("message"))
        .and_then(|message| message.get("content"))
        .and_then(|content| content.as_str())
        .and_then(clean_llm_cluster_name);
    Ok(content)
}

#[cfg(feature = "icebug-analytics")]
fn cluster_records(
    membership: &[u64],
    parent_membership: Option<&[u64]>,
    nodes: &[GraphNode],
    llm_config: Option<&LlmClusterNamingConfig>,
    llm_name_budget: &mut usize,
) -> Vec<GraphCluster> {
    let mut counts: HashMap<u64, usize> = HashMap::new();
    let mut parents: HashMap<u64, u64> = HashMap::new();
    let mut labels_by_cluster: HashMap<u64, Vec<String>> = HashMap::new();
    for (index, cluster_id) in membership.iter().enumerate() {
        *counts.entry(*cluster_id).or_insert(0) += 1;
        if let Some(parent_ids) = parent_membership {
            if let Some(parent_id) = parent_ids.get(index) {
                parents.entry(*cluster_id).or_insert(*parent_id);
            }
        }
        if let Some(node) = nodes.get(index) {
            labels_by_cluster
                .entry(*cluster_id)
                .or_default()
                .push(node_cluster_label(node));
        }
    }

    let llm_enabled = llm_config.filter(|config| config.enabled()).is_some();
    let mut llm_cluster_ids = HashSet::new();
    if llm_enabled && *llm_name_budget > 0 {
        let mut candidates: Vec<(u64, usize)> = counts
            .iter()
            .filter_map(|(cluster_id, size)| (*size > 1).then_some((*cluster_id, *size)))
            .collect();
        candidates.sort_by_key(|(cluster_id, size)| (std::cmp::Reverse(*size), *cluster_id));
        let take_count = candidates.len().min(*llm_name_budget);
        llm_cluster_ids.extend(
            candidates
                .into_iter()
                .take(take_count)
                .map(|(cluster_id, _)| cluster_id),
        );
        eprintln!(
            "LLM cluster naming: naming {} cluster(s), {} budgeted call(s) remaining before this level.",
            llm_cluster_ids.len(),
            *llm_name_budget,
        );
    } else if llm_enabled {
        eprintln!("LLM cluster naming: budget exhausted; using fallback names for this level.");
    }

    let mut clusters: Vec<GraphCluster> = counts
        .into_iter()
        .map(|(cluster_id, size)| {
            let fallback_label = format!("Cluster {cluster_id}");
            let label = llm_config
                .filter(|config| config.enabled())
                .filter(|_| llm_cluster_ids.contains(&cluster_id))
                .and_then(|config| {
                    let mut sample = labels_by_cluster
                        .get(&cluster_id)
                        .cloned()
                        .unwrap_or_default();
                    sample.sort();
                    sample.dedup();
                    fastrand::shuffle(&mut sample);
                    sample.truncate(config.sample_size());
                    eprintln!(
                        "LLM cluster naming: requesting name for cluster {cluster_id} with {} sampled label(s).",
                        sample.len(),
                    );
                    *llm_name_budget = llm_name_budget.saturating_sub(1);
                    match llm_cluster_name(config, cluster_id, &sample) {
                        Ok(name) => {
                            if let Some(name) = &name {
                                eprintln!(
                                    "LLM cluster naming: cluster {cluster_id} named {name:?}."
                                );
                            } else {
                                eprintln!(
                                    "LLM cluster naming: cluster {cluster_id} returned no usable name; using fallback."
                                );
                            }
                            name
                        }
                        Err(err) => {
                            eprintln!("Cluster {cluster_id} LLM naming failed: {err}");
                            None
                        }
                    }
                })
                .unwrap_or(fallback_label);

            GraphCluster {
                cluster_id,
                label,
                size,
                parent_cluster_id: parents.get(&cluster_id).copied(),
            }
        })
        .collect();
    clusters.sort_by_key(|cluster| cluster.cluster_id);
    clusters
}

#[cfg(feature = "icebug-analytics")]
fn aggregate_cluster_edges(
    membership: &[u64],
    links: &[GraphLink],
    node_index: &HashMap<String, usize>,
) -> (usize, Vec<GraphLink>) {
    let cluster_count = membership
        .iter()
        .max()
        .map(|id| *id as usize + 1)
        .unwrap_or(0);
    let mut seen = HashSet::new();
    let mut links_out = Vec::new();

    for link in links {
        let Some(&source_index) = node_index.get(&link.source) else {
            continue;
        };
        let Some(&target_index) = node_index.get(&link.target) else {
            continue;
        };
        let source = membership[source_index];
        let target = membership[target_index];
        if source == target {
            continue;
        }
        let key = if source < target {
            (source, target)
        } else {
            (target, source)
        };
        if seen.insert(key) {
            links_out.push(GraphLink {
                source: key.0.to_string(),
                target: key.1.to_string(),
                label: "cluster".to_string(),
            });
        }
    }

    (cluster_count, links_out)
}

fn cluster_debug(
    enabled: bool,
    status: &str,
    message: String,
    node_count: usize,
    edge_count: usize,
    undirected_edge_count: usize,
    levels: usize,
    clusters: usize,
) -> GraphClusterDebug {
    GraphClusterDebug {
        enabled,
        status: status.to_string(),
        message,
        node_count,
        edge_count,
        undirected_edge_count,
        levels,
        clusters,
    }
}

#[cfg(feature = "icebug-analytics")]
fn compute_cluster_levels(
    nodes: &[GraphNode],
    links: &[GraphLink],
    llm_config: Option<&LlmClusterNamingConfig>,
    llm_name_budget: &mut usize,
) -> (Option<Vec<GraphClusterLevel>>, GraphClusterDebug) {
    let node_count = nodes.len();
    if node_count < 2 {
        return (
            None,
            cluster_debug(
                true,
                "skipped",
                "Leiden skipped: fewer than two visible nodes.".to_string(),
                node_count,
                links.len(),
                0,
                0,
                0,
            ),
        );
    }
    if links.is_empty() {
        return (
            None,
            cluster_debug(
                true,
                "skipped",
                "Leiden skipped: the visible graph has no relationships.".to_string(),
                node_count,
                links.len(),
                0,
                0,
                0,
            ),
        );
    }

    let node_index: HashMap<String, usize> = nodes
        .iter()
        .enumerate()
        .map(|(index, node)| (node.id.clone(), index))
        .collect();

    let csr = build_undirected_csr(node_count, links, &node_index);
    let undirected_edge_count = csr.indices.len() / 2;
    if csr.indices.is_empty() {
        return (
            None,
            cluster_debug(
                true,
                "skipped",
                "Leiden skipped: relationships did not connect visible nodes after filtering."
                    .to_string(),
                node_count,
                links.len(),
                undirected_edge_count,
                0,
                0,
            ),
        );
    }

    let level_zero_raw = match leiden_membership(node_count, &csr) {
        Ok(membership) => membership,
        Err(err) => {
            return (
                None,
                cluster_debug(
                    true,
                    "error",
                    format!("Leiden failed: {err}"),
                    node_count,
                    links.len(),
                    undirected_edge_count,
                    0,
                    0,
                ),
            );
        }
    };
    let (level_zero, _) = remap_membership(&level_zero_raw);
    let mut levels = Vec::new();
    levels.push(GraphClusterLevel {
        level: 0,
        membership: level_zero.clone(),
        clusters: cluster_records(&level_zero, None, nodes, llm_config, llm_name_budget),
    });

    let mut node_membership = level_zero.clone();
    let mut graph_membership = level_zero;
    let mut current_links = links.to_vec();
    let mut current_node_index = node_index;

    for level in 1..CLUSTER_LEVEL_LIMIT {
        let (cluster_count, aggregate_links) =
            aggregate_cluster_edges(&graph_membership, &current_links, &current_node_index);
        if cluster_count < 2
            || aggregate_links.is_empty()
            || cluster_count >= graph_membership.len()
        {
            break;
        }

        let cluster_nodes: Vec<GraphNode> = (0..cluster_count)
            .map(|index| GraphNode {
                id: index.to_string(),
                name: format!("Cluster {index}"),
                label: "Cluster".to_string(),
                properties: HashMap::new(),
                table_id: None,
                rowid: None,
                community: Some(index as u64),
                expansion_kind: None,
                expand_node_id: None,
                offset: None,
                hidden_count: None,
                color_key: None,
            })
            .collect();
        let cluster_node_index: HashMap<String, usize> = cluster_nodes
            .iter()
            .enumerate()
            .map(|(index, node)| (node.id.clone(), index))
            .collect();
        let cluster_csr =
            build_undirected_csr(cluster_count, &aggregate_links, &cluster_node_index);
        let cluster_membership_raw = match leiden_membership(cluster_count, &cluster_csr) {
            Ok(membership) => membership,
            Err(err) => {
                eprintln!("Stopping hierarchical Leiden at level {level}: {err}");
                break;
            }
        };
        let (cluster_membership, _) = remap_membership(&cluster_membership_raw);
        let next_membership: Vec<u64> = node_membership
            .iter()
            .map(|cluster_id| cluster_membership[*cluster_id as usize])
            .collect();

        if next_membership == node_membership {
            break;
        }

        if let Some(previous) = levels.last_mut() {
            previous.clusters = cluster_records(
                &node_membership,
                Some(&next_membership),
                nodes,
                llm_config,
                llm_name_budget,
            );
        }
        levels.push(GraphClusterLevel {
            level,
            membership: next_membership.clone(),
            clusters: cluster_records(&next_membership, None, nodes, llm_config, llm_name_budget),
        });

        node_membership = next_membership;
        graph_membership = cluster_membership;
        current_links = aggregate_links;
        current_node_index = cluster_node_index;
    }

    let level_count = levels.len();
    let cluster_count = levels
        .first()
        .map(|level| level.clusters.len())
        .unwrap_or_default();
    let message = format!(
        "Leiden produced {level_count} level(s), with {cluster_count} cluster(s) at level 0 from {node_count} visible node(s), {edge_count} directed edge(s), {undirected_edge_count} undirected edge(s).",
        edge_count = links.len(),
    );

    (
        Some(levels),
        cluster_debug(
            true,
            "ready",
            message,
            node_count,
            links.len(),
            undirected_edge_count,
            level_count,
            cluster_count,
        ),
    )
}

#[cfg(not(feature = "icebug-analytics"))]
fn compute_cluster_levels(
    _nodes: &[GraphNode],
    links: &[GraphLink],
    _llm_config: Option<&LlmClusterNamingConfig>,
    _llm_name_budget: &mut usize,
) -> (Option<Vec<GraphClusterLevel>>, GraphClusterDebug) {
    (
        None,
        cluster_debug(
            false,
            "disabled",
            "Leiden disabled: run with `cargo tauri dev --features icebug-analytics`.".to_string(),
            _nodes.len(),
            links.len(),
            0,
            0,
            0,
        ),
    )
}

fn graph_data(
    nodes: Vec<GraphNode>,
    links: Vec<GraphLink>,
    llm_config: Option<&LlmClusterNamingConfig>,
    context: &str,
) -> GraphData {
    eprintln!(
        "Graph data [{context}]: building response for {} node(s), {} edge(s); visible LLM cluster naming {}.",
        nodes.len(),
        links.len(),
        if llm_config
            .filter(|config| config.enabled())
            .is_some()
        {
            "enabled"
        } else {
            "disabled"
        }
    );
    if let Some(config) = llm_config.filter(|config| config.enabled()) {
        eprintln!(
            "Graph data [{context}]: LLM model={}, endpoint={}, sample_size={}, max_visible_cluster_name_calls={}; naming is deferred until clusters are visible.",
            config.model(),
            config.endpoint(),
            config.sample_size(),
            LLM_CLUSTER_NAME_LIMIT,
        );
    }
    let csr = build_csr(&nodes, &links);
    let csr_arrow_ipc = csr_to_arrow_ipc(&csr)
        .map_err(|err| {
            eprintln!("{err}");
            err
        })
        .ok();
    let mut llm_name_budget = LLM_CLUSTER_NAME_LIMIT;
    let (cluster_levels, cluster_debug) =
        compute_cluster_levels(&nodes, &links, None, &mut llm_name_budget);
    eprintln!("Cluster debug [{context}]: {}", cluster_debug.message);
    GraphData {
        nodes,
        links,
        csr: if csr_arrow_ipc.is_some() {
            None
        } else {
            Some(csr)
        },
        csr_arrow_ipc,
        cluster_levels,
        cluster_debug,
    }
}

fn graph_data_without_clusters(
    nodes: Vec<GraphNode>,
    links: Vec<GraphLink>,
    context: &str,
) -> GraphData {
    eprintln!(
        "Graph data [{context}]: building internal graph for {} node(s), {} edge(s); clustering deferred.",
        nodes.len(),
        links.len(),
    );
    let csr = build_csr(&nodes, &links);
    let csr_arrow_ipc = csr_to_arrow_ipc(&csr)
        .map_err(|err| {
            eprintln!("{err}");
            err
        })
        .ok();
    GraphData {
        nodes,
        links,
        csr: if csr_arrow_ipc.is_some() {
            None
        } else {
            Some(csr)
        },
        csr_arrow_ipc,
        cluster_levels: None,
        cluster_debug: cluster_debug(
            false,
            "deferred",
            "Cluster computation deferred for internal full graph.".to_string(),
            0,
            0,
            0,
            0,
            0,
        ),
    }
}

fn collect_edge_graph(conn: &Connection, limit: usize) -> Result<GraphData, String> {
    let mut result = conn
        .query(&format!("MATCH (a)-[r]->(b) RETURN a, r, b LIMIT {limit}"))
        .map_err(|e| format!("Relationship query failed: {}", e))?;

    let mut nodes = HashMap::new();
    let mut links = Vec::new();
    let mut seen_links = HashSet::new();

    for row in &mut result {
        if row.len() < 3 {
            continue;
        }
        let (source_node, rel, target_node) = match (&row[0], &row[1], &row[2]) {
            (Value::Node(source_node), Value::Rel(rel), Value::Node(target_node)) => {
                (source_node, rel, target_node)
            }
            _ => continue,
        };

        let source = id_to_string(rel.get_src_node());
        let target = id_to_string(rel.get_dst_node());
        for node_val in [source_node, target_node] {
            let props = node_val.get_properties();
            let name = node_display_name(props);
            let properties = node_display_properties(props);
            merge_node(
                &mut nodes,
                GraphNode {
                    id: id_to_string(node_val.get_node_id()),
                    name,
                    label: node_val.get_label_name().clone(),
                    properties,
                    table_id: Some(node_val.get_node_id().table_id),
                    rowid: Some(node_val.get_node_id().offset),
                    community: None,
                    expansion_kind: None,
                    expand_node_id: None,
                    offset: None,
                    hidden_count: None,
                    color_key: None,
                },
            );
        }
        merge_link(
            &mut links,
            &mut seen_links,
            GraphLink {
                source,
                target,
                label: rel.get_label_name().clone(),
            },
        );
    }

    Ok(graph_data_without_clusters(
        nodes.into_values().collect(),
        links,
        "collect_edge_graph",
    ))
}

fn escaped_identifier(value: &str) -> String {
    value.replace('\\', "\\\\").replace('\'', "\\'")
}

fn table_properties(conn: &Connection, table_name: &str) -> Vec<(String, String, bool)> {
    let Ok(mut result) = conn.query(&format!(
        "CALL TABLE_INFO('{}') RETURN *;",
        escaped_identifier(table_name)
    )) else {
        return Vec::new();
    };

    let mut properties = Vec::new();
    for row in &mut result {
        let Some(name) = row.get(1).map(value_to_string) else {
            continue;
        };
        let property_type = row
            .get(2)
            .map(value_to_string)
            .unwrap_or_else(|| "UNKNOWN".to_string());
        let is_primary_key = row
            .get(4)
            .map(|value| matches!(value, Value::Bool(true)))
            .unwrap_or(false);
        properties.push((name, property_type, is_primary_key));
    }
    properties
}

fn schema_node_properties(
    conn: &Connection,
    table_name: &str,
    include_primary_key: bool,
) -> HashMap<String, String> {
    table_properties(conn, table_name)
        .into_iter()
        .map(|(name, property_type, is_primary_key)| {
            let value = if include_primary_key && is_primary_key {
                format!("{property_type} primary key")
            } else {
                property_type
            };
            (name, value)
        })
        .collect()
}

fn collect_schema_graph(conn: &Connection) -> Result<GraphData, String> {
    let mut tables = conn
        .query("CALL show_tables() RETURN *;")
        .map_err(|e| format!("Schema table query failed: {e}"))?;

    let mut node_tables = Vec::new();
    let mut rel_tables = Vec::new();
    for row in &mut tables {
        if row.len() < 3 {
            continue;
        }
        let table_name = value_to_string(&row[1]);
        let table_type = value_to_string(&row[2]);
        if table_type.eq_ignore_ascii_case("NODE") {
            node_tables.push(table_name);
        } else if table_type.eq_ignore_ascii_case("REL") {
            rel_tables.push(table_name);
        }
    }

    node_tables.sort();
    rel_tables.sort();

    let nodes: Vec<GraphNode> = node_tables
        .iter()
        .map(|table_name| GraphNode {
            id: table_name.clone(),
            name: table_name.clone(),
            label: "Node Table".to_string(),
            properties: schema_node_properties(conn, table_name, true),
            table_id: None,
            rowid: None,
            community: None,
            expansion_kind: None,
            expand_node_id: None,
            offset: None,
            hidden_count: None,
            color_key: Some(format!("schema-node:{table_name}")),
        })
        .collect();

    let node_table_set: HashSet<&str> = node_tables.iter().map(String::as_str).collect();
    let mut links = Vec::new();
    let mut seen_links = HashSet::new();

    for rel_table in &rel_tables {
        let Ok(mut result) = conn.query(&format!(
            "CALL SHOW_CONNECTION('{}') RETURN *;",
            escaped_identifier(rel_table)
        )) else {
            continue;
        };

        for row in &mut result {
            if row.len() < 2 {
                continue;
            }
            let source = value_to_string(&row[0]);
            let target = value_to_string(&row[1]);
            if !node_table_set.contains(source.as_str())
                || !node_table_set.contains(target.as_str())
            {
                continue;
            }
            merge_link(
                &mut links,
                &mut seen_links,
                GraphLink {
                    source,
                    target,
                    label: rel_table.clone(),
                },
            );
        }
    }

    Ok(graph_data_without_clusters(nodes, links, "collect_schema_graph"))
}

fn add_expanders(
    graph: &GraphData,
    visible_ids: &HashSet<String>,
    nodes: &mut Vec<GraphNode>,
    links: &mut Vec<GraphLink>,
) {
    let known_ids: HashSet<String> = graph.nodes.iter().map(|node| node.id.clone()).collect();
    let mut neighbors: HashMap<String, HashSet<String>> = HashMap::new();
    for link in &graph.links {
        neighbors
            .entry(link.source.clone())
            .or_default()
            .insert(link.target.clone());
        neighbors
            .entry(link.target.clone())
            .or_default()
            .insert(link.source.clone());
    }

    for node_id in visible_ids {
        let hidden_count = neighbors
            .get(node_id)
            .map(|items| {
                items
                    .iter()
                    .filter(|neighbor_id| {
                        !visible_ids.contains(*neighbor_id) && known_ids.contains(*neighbor_id)
                    })
                    .count()
            })
            .unwrap_or(0);
        if hidden_count == 0 {
            continue;
        }

        let expander = make_expander_node(node_id, hidden_count, 0);
        links.push(GraphLink {
            source: node_id.clone(),
            target: expander.id.clone(),
            label: "more".to_string(),
        });
        nodes.push(expander);
    }
}

fn seed_graph_from_full(
    full_graph: GraphData,
    llm_config: Option<&LlmClusterNamingConfig>,
) -> GraphData {
    let mut degrees: HashMap<String, usize> = HashMap::new();
    for node in &full_graph.nodes {
        degrees.entry(node.id.clone()).or_insert(0);
    }
    for link in &full_graph.links {
        *degrees.entry(link.source.clone()).or_insert(0) += 1;
        *degrees.entry(link.target.clone()).or_insert(0) += 1;
    }

    let mut ranked_nodes = full_graph.nodes.clone();
    ranked_nodes.sort_by_key(|node| std::cmp::Reverse(*degrees.get(&node.id).unwrap_or(&0)));
    let visible_ids: HashSet<String> = ranked_nodes
        .iter()
        .take(SEED_NODE_COUNT)
        .map(|node| node.id.clone())
        .collect();

    let mut nodes: Vec<GraphNode> = ranked_nodes
        .into_iter()
        .filter(|node| visible_ids.contains(&node.id))
        .collect();
    let mut links: Vec<GraphLink> = full_graph
        .links
        .iter()
        .filter(|link| visible_ids.contains(&link.source) && visible_ids.contains(&link.target))
        .cloned()
        .collect();

    add_expanders(&full_graph, &visible_ids, &mut nodes, &mut links);
    graph_data(nodes, links, llm_config, "seed_graph_from_full")
}

fn expand_node_from_full(
    full_graph: GraphData,
    node_id: &str,
    visible_node_ids: &[String],
    offset: usize,
    llm_config: Option<&LlmClusterNamingConfig>,
) -> GraphData {
    let visible_order: Vec<String> = visible_node_ids
        .iter()
        .filter(|id| !id.starts_with(EXPANDER_PREFIX))
        .cloned()
        .collect();
    let visible_ids: HashSet<String> = visible_order.iter().cloned().collect();

    let mut degrees: HashMap<String, usize> = HashMap::new();
    for link in &full_graph.links {
        *degrees.entry(link.source.clone()).or_insert(0) += 1;
        *degrees.entry(link.target.clone()).or_insert(0) += 1;
    }

    let mut neighbor_ids: Vec<String> = full_graph
        .links
        .iter()
        .filter_map(|link| {
            if link.source == node_id {
                Some(link.target.clone())
            } else if link.target == node_id {
                Some(link.source.clone())
            } else {
                None
            }
        })
        .filter(|neighbor_id| !visible_ids.contains(neighbor_id))
        .collect();
    neighbor_ids.sort();
    neighbor_ids.dedup();
    neighbor_ids.sort_by_key(|id| std::cmp::Reverse(*degrees.get(id).unwrap_or(&0)));

    let selected_ids: HashSet<String> = neighbor_ids
        .iter()
        .skip(offset)
        .take(EXPAND_BATCH_SIZE)
        .cloned()
        .collect();

    let full_node_by_id: HashMap<String, GraphNode> = full_graph
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node.clone()))
        .collect();
    let mut return_ids = visible_ids.clone();
    return_ids.extend(selected_ids.iter().cloned());

    let mut node_ids = visible_order;
    for id in &selected_ids {
        if node_ids.contains(id) {
            continue;
        }
        node_ids.push(id.clone());
    }

    let mut nodes: Vec<GraphNode> = node_ids
        .iter()
        .filter_map(|id| full_node_by_id.get(id).cloned())
        .collect();
    let mut links: Vec<GraphLink> = full_graph
        .links
        .iter()
        .filter(|link| return_ids.contains(&link.source) && return_ids.contains(&link.target))
        .cloned()
        .collect();

    let next_offset = offset + selected_ids.len();
    let remaining = neighbor_ids.len().saturating_sub(next_offset);
    if remaining > 0 {
        let expander = make_expander_node(node_id, remaining, next_offset);
        links.push(GraphLink {
            source: node_id.to_string(),
            target: expander.id.clone(),
            label: "more".to_string(),
        });
        nodes.push(expander);
    }

    graph_data(nodes, links, llm_config, "expand_node_from_full")
}

#[tauri::command]
fn get_databases(state: State<AppState>) -> Vec<DatabaseInfo> {
    get_all_databases(&state)
}

#[tauri::command]
fn get_initial_database_id(state: State<AppState>) -> Option<usize> {
    let initial_path = state.initial_database_path.as_ref()?;
    get_all_databases(&state)
        .iter()
        .find(|db| db.path == *initial_path)
        .map(|db| db.id)
}

#[tauri::command]
fn name_visible_clusters(
    llm_config: LlmClusterNamingConfig,
    clusters: Vec<LlmClusterNameRequest>,
) -> Vec<LlmClusterNameResult> {
    if !llm_config.enabled() {
        eprintln!("Visible cluster naming skipped: LLM config is disabled.");
        return Vec::new();
    }

    let request_count = clusters.len().min(LLM_CLUSTER_NAME_LIMIT);
    eprintln!(
        "Visible cluster naming: naming {} visible cluster(s), capped from {} requested cluster(s); model={}, endpoint={}.",
        request_count,
        clusters.len(),
        llm_config.model(),
        llm_config.endpoint(),
    );

    clusters
        .into_iter()
        .take(LLM_CLUSTER_NAME_LIMIT)
        .map(|cluster| {
            let mut sample = cluster.labels;
            sample.sort();
            sample.dedup();
            fastrand::shuffle(&mut sample);
            sample.truncate(llm_config.sample_size());
            eprintln!(
                "Visible cluster naming: requesting name for {} with {} sampled label(s).",
                cluster.key,
                sample.len(),
            );

            match llm_cluster_name(&llm_config, cluster.cluster_id, &sample) {
                Ok(name) => {
                    if let Some(name) = &name {
                        eprintln!(
                            "Visible cluster naming: cluster {} named {name:?}.",
                            cluster.key
                        );
                    } else {
                        eprintln!(
                            "Visible cluster naming: cluster {} returned no usable name.",
                            cluster.key
                        );
                    }
                    LlmClusterNameResult {
                        key: cluster.key,
                        name,
                        error: None,
                    }
                }
                Err(err) => {
                    eprintln!(
                        "Visible cluster naming: cluster {} failed: {err}",
                        cluster.key
                    );
                    LlmClusterNameResult {
                        key: cluster.key,
                        name: None,
                        error: Some(err),
                    }
                }
            }
        })
        .collect()
}

fn add_database_info(state: &AppState, db_info: DatabaseInfo) -> Result<DatabaseInfo, String> {
    let mut custom = state.custom_databases.lock().unwrap();
    if custom.iter().any(|d| d.path == db_info.path) {
        return Err("Database already added".to_string());
    }
    custom.push(db_info.clone());

    // Return with correct id
    drop(custom);
    let all = get_all_databases(&state);
    Ok(all
        .into_iter()
        .find(|db| db.path == db_info.path)
        .unwrap_or(db_info))
}

#[tauri::command]
fn add_database(state: State<AppState>, file_path: String) -> Result<DatabaseInfo, String> {
    let db_info = database_info_from_path(&file_path)?;
    add_database_info(&state, db_info)
}

fn default_file_picker_dir(state: &AppState) -> PathBuf {
    dirs::home_dir()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| state.data_dir.clone())
}

#[tauri::command]
fn get_directories(
    state: State<AppState>,
    path: Option<String>,
) -> Result<DirectoryListing, String> {
    let dir = match &path {
        Some(p) if !p.is_empty() => {
            let resolved = PathBuf::from(p);
            if resolved.is_absolute() {
                resolved
            } else {
                std::env::current_dir()
                    .unwrap_or_else(|_| state.data_dir.clone())
                    .join(p)
            }
        }
        _ => default_file_picker_dir(&state),
    };

    let entries = fs::read_dir(&dir).map_err(|e| e.to_string())?;

    let mut directories = Vec::new();
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let entry_path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        if name.starts_with('.') {
            continue;
        }

        if entry_path.is_dir() {
            directories.push(DirEntry {
                name,
                path: entry_path.to_string_lossy().to_string(),
                entry_type: "directory".to_string(),
            });
        } else if name.ends_with(".lbdb") {
            files.push(DirEntry {
                name: name.trim_end_matches(".lbdb").to_string(),
                path: entry_path.to_string_lossy().to_string(),
                entry_type: "file".to_string(),
            });
        }
    }

    directories.sort_by(|a, b| a.name.cmp(&b.name));
    files.sort_by(|a, b| a.name.cmp(&b.name));

    let parent = dir
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    Ok(DirectoryListing {
        current: dir.to_string_lossy().to_string(),
        parent,
        directories,
        files,
    })
}

#[tauri::command]
fn get_graph(
    state: State<AppState>,
    id: usize,
    llm_config: Option<LlmClusterNamingConfig>,
) -> Result<GraphData, String> {
    let databases = get_all_databases(&state);
    let db_info = databases.get(id).ok_or("Database not found")?;

    let db = Database::new(&db_info.path, SystemConfig::default())
        .map_err(|e| format!("Failed to open database: {}", e))?;
    let conn = Connection::new(&db).map_err(|e| format!("Failed to create connection: {}", e))?;

    collect_edge_graph(&conn, EDGE_SCAN_LIMIT)
        .map(|full_graph| seed_graph_from_full(full_graph, llm_config.as_ref()))
}

#[tauri::command]
fn get_schema_graph(state: State<AppState>, id: usize) -> Result<GraphData, String> {
    let databases = get_all_databases(&state);
    let db_info = databases.get(id).ok_or("Database not found")?;

    let db = Database::new(&db_info.path, SystemConfig::default())
        .map_err(|e| format!("Failed to open database: {}", e))?;
    let conn = Connection::new(&db).map_err(|e| format!("Failed to create connection: {}", e))?;

    collect_schema_graph(&conn)
}

#[tauri::command]
fn search_nodes(
    state: State<AppState>,
    id: usize,
    query: String,
) -> Result<Vec<GraphNode>, String> {
    let query_text = query.trim();
    if query_text.is_empty() {
        return Ok(Vec::new());
    }
    let query = query_text.to_lowercase();

    let databases = get_all_databases(&state);
    let db_info = databases.get(id).ok_or("Database not found")?;

    let db = Database::new(&db_info.path, SystemConfig::default())
        .map_err(|e| format!("Failed to open database: {}", e))?;
    let conn = Connection::new(&db).map_err(|e| format!("Failed to create connection: {}", e))?;

    if let Some(fts_matches) = search_nodes_with_fts(&conn, query_text) {
        return Ok(fts_matches);
    }

    let mut result = conn
        .query(&format!(
            "MATCH (n) RETURN n LIMIT {NODE_SEARCH_SCAN_LIMIT}"
        ))
        .map_err(|e| format!("Node search query failed: {}", e))?;

    let mut matches = Vec::new();
    for row in &mut result {
        for val in row.iter() {
            if node_value_matches_query(val, &query) {
                if let Some(node) = graph_node_from_value(val) {
                    matches.push(node);
                }
            }
        }
    }

    matches.sort_by(|a, b| {
        let a_exact =
            a.name.eq_ignore_ascii_case(query_text) || a.id.eq_ignore_ascii_case(query_text);
        let b_exact =
            b.name.eq_ignore_ascii_case(query_text) || b.id.eq_ignore_ascii_case(query_text);
        b_exact
            .cmp(&a_exact)
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.id.cmp(&b.id))
    });
    matches.truncate(SEARCH_RESULT_LIMIT);
    Ok(matches)
}

#[tauri::command]
fn get_node_neighborhood(
    state: State<AppState>,
    id: usize,
    node_id: String,
    llm_config: Option<LlmClusterNamingConfig>,
) -> Result<GraphData, String> {
    let focus_node_id = node_id;
    let databases = get_all_databases(&state);
    let db_info = databases.get(id).ok_or("Database not found")?;

    let db = Database::new(&db_info.path, SystemConfig::default())
        .map_err(|e| format!("Failed to open database: {}", e))?;
    let conn = Connection::new(&db).map_err(|e| format!("Failed to create connection: {}", e))?;
    let full_graph = collect_edge_graph(&conn, EDGE_SCAN_LIMIT)?;

    let mut degrees: HashMap<String, usize> = HashMap::new();
    for link in &full_graph.links {
        *degrees.entry(link.source.clone()).or_insert(0) += 1;
        *degrees.entry(link.target.clone()).or_insert(0) += 1;
    }

    let mut neighbor_ids: Vec<String> = full_graph
        .links
        .iter()
        .filter_map(|link| {
            if link.source == focus_node_id {
                Some(link.target.clone())
            } else if link.target == focus_node_id {
                Some(link.source.clone())
            } else {
                None
            }
        })
        .collect();
    neighbor_ids.sort();
    neighbor_ids.dedup();
    neighbor_ids.sort_by_key(|id| std::cmp::Reverse(*degrees.get(id).unwrap_or(&0)));

    let mut visible_ids: HashSet<String> = neighbor_ids
        .into_iter()
        .take(SEARCH_NEIGHBOR_LIMIT)
        .collect();
    visible_ids.insert(focus_node_id.clone());

    let full_node_by_id: HashMap<String, GraphNode> = full_graph
        .nodes
        .iter()
        .map(|node| (node.id.clone(), node.clone()))
        .collect();
    let mut nodes: Vec<GraphNode> = visible_ids
        .iter()
        .filter_map(|id| full_node_by_id.get(id).cloned())
        .collect();
    nodes.sort_by_key(|node| {
        (
            node.id != focus_node_id,
            std::cmp::Reverse(*degrees.get(&node.id).unwrap_or(&0)),
            node.name.clone(),
        )
    });

    let mut links: Vec<GraphLink> = full_graph
        .links
        .iter()
        .filter(|link| visible_ids.contains(&link.source) && visible_ids.contains(&link.target))
        .cloned()
        .collect();

    add_expanders(&full_graph, &visible_ids, &mut nodes, &mut links);
    Ok(graph_data(
        nodes,
        links,
        llm_config.as_ref(),
        "get_node_neighborhood",
    ))
}

#[tauri::command]
fn expand_node(
    state: State<AppState>,
    id: usize,
    node_id: String,
    visible_node_ids: Vec<String>,
    offset: Option<usize>,
    llm_config: Option<LlmClusterNamingConfig>,
) -> Result<GraphData, String> {
    let databases = get_all_databases(&state);
    let db_info = databases.get(id).ok_or("Database not found")?;

    let db = Database::new(&db_info.path, SystemConfig::default())
        .map_err(|e| format!("Failed to open database: {}", e))?;
    let conn = Connection::new(&db).map_err(|e| format!("Failed to create connection: {}", e))?;
    let full_graph = collect_edge_graph(&conn, EDGE_SCAN_LIMIT)?;

    Ok(expand_node_from_full(
        full_graph,
        &node_id,
        &visible_node_ids,
        offset.unwrap_or(0),
        llm_config.as_ref(),
    ))
}

#[tauri::command]
fn execute_query(
    state: State<AppState>,
    id: usize,
    query: String,
    llm_config: Option<LlmClusterNamingConfig>,
) -> Result<GraphData, String> {
    let databases = get_all_databases(&state);
    let db_info = databases.get(id).ok_or("Database not found")?;

    let db = Database::new(&db_info.path, SystemConfig::default())
        .map_err(|e| format!("Failed to open database: {}", e))?;
    let conn = Connection::new(&db).map_err(|e| format!("Failed to create connection: {}", e))?;

    // Execute user query
    let mut result = conn
        .query(&query)
        .map_err(|e| format!("Query failed: {}", e))?;

    let mut nodes = Vec::new();
    let mut links = Vec::new();
    let mut node_id_set: HashSet<String> = HashSet::new();

    for row in &mut result {
        for val in row.iter() {
            match val {
                Value::Node(node_val) => {
                    let node_id = id_to_string(node_val.get_node_id());

                    if !node_id_set.contains(&node_id) {
                        node_id_set.insert(node_id.clone());
                        let props = node_val.get_properties();
                        let name = node_display_name(props);
                        let properties = node_display_properties(props);

                        let label = node_val.get_label_name().clone();

                        nodes.push(GraphNode {
                            id: node_id,
                            name,
                            label,
                            properties,
                            table_id: Some(node_val.get_node_id().table_id),
                            rowid: Some(node_val.get_node_id().offset),
                            community: None,
                            expansion_kind: None,
                            expand_node_id: None,
                            offset: None,
                            hidden_count: None,
                            color_key: None,
                        });
                    }
                }
                Value::Rel(rel_val) => {
                    let source = id_to_string(rel_val.get_src_node());
                    let target = id_to_string(rel_val.get_dst_node());
                    let label = rel_val.get_label_name().clone();

                    links.push(GraphLink {
                        source,
                        target,
                        label,
                    });
                }
                _ => {}
            }
        }
    }

    // Filter links to only include those where both endpoints exist
    let links = links
        .into_iter()
        .filter(|link| node_id_set.contains(&link.source) && node_id_set.contains(&link.target))
        .collect();

    Ok(graph_data(
        nodes,
        links,
        llm_config.as_ref(),
        "execute_query",
    ))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let data_dir = app
                .path()
                .app_data_dir()
                .expect("Failed to get app data dir");
            std::fs::create_dir_all(&data_dir).ok();
            let initial_database = std::env::args()
                .skip(1)
                .find(|arg| !arg.starts_with('-'))
                .and_then(|arg| match database_info_from_path(&arg) {
                    Ok(db_info) => Some(db_info),
                    Err(err) => {
                        eprintln!("Ignoring database path {arg:?}: {err}");
                        None
                    }
                });
            let initial_database_path = initial_database.as_ref().map(|db| db.path.clone());
            app.manage(AppState {
                custom_databases: Mutex::new(initial_database.into_iter().collect()),
                initial_database_path,
                data_dir,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_databases,
            get_initial_database_id,
            name_visible_clusters,
            add_database,
            get_directories,
            get_graph,
            get_schema_graph,
            search_nodes,
            get_node_neighborhood,
            expand_node,
            execute_query,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_display_name_prefers_non_id_string_over_integer_id() {
        let props = vec![
            ("id".to_string(), Value::Int64(42)),
            ("bug_key".to_string(), Value::String("BUG-42".to_string())),
        ];

        assert_eq!(node_display_name(&props), "BUG-42");
    }

    #[test]
    fn node_display_name_prefers_named_columns_before_id() {
        let props = vec![
            ("id".to_string(), Value::String("42".to_string())),
            (
                "title".to_string(),
                Value::String("Crash on launch".to_string()),
            ),
        ];

        assert_eq!(node_display_name(&props), "Crash on launch");
    }

    #[test]
    fn node_display_name_uses_string_id_when_it_is_best_available_name() {
        let props = vec![("id".to_string(), Value::String("BUG-42".to_string()))];

        assert_eq!(node_display_name(&props), "BUG-42");
    }
}
