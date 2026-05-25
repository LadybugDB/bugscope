use lbug::{Connection, Database, SystemConfig, Value};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
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
struct GraphData {
    nodes: Vec<GraphNode>,
    links: Vec<GraphLink>,
    #[serde(skip_serializing_if = "Option::is_none")]
    csr: Option<GraphCsr>,
}

const SEED_NODE_COUNT: usize = 8;
const EXPAND_BATCH_SIZE: usize = 8;
const EDGE_SCAN_LIMIT: usize = 10_000;
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
        Value::Double(n) => n.to_string(),
        Value::Float(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => format!("{}", val),
    }
}

fn id_to_string(id: &lbug::InternalID) -> String {
    format!("{}:{}", id.table_id, id.offset)
}

fn make_expander_node(parent_id: &str, hidden_count: usize, offset: usize) -> GraphNode {
    GraphNode {
        id: format!("{EXPANDER_PREFIX}node:{parent_id}:{offset}"),
        name: format!("+{hidden_count}"),
        label: "More".to_string(),
        table_id: None,
        rowid: None,
        community: None,
        expansion_kind: Some("node".to_string()),
        expand_node_id: Some(parent_id.to_string()),
        offset: Some(offset),
        hidden_count: Some(hidden_count),
    }
}

fn merge_node(nodes: &mut HashMap<String, GraphNode>, node: GraphNode) {
    nodes.entry(node.id.clone()).or_insert(node);
}

fn merge_link(links: &mut Vec<GraphLink>, seen: &mut HashSet<(String, String, String)>, link: GraphLink) {
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

fn graph_data(nodes: Vec<GraphNode>, links: Vec<GraphLink>) -> GraphData {
    let csr = Some(build_csr(&nodes, &links));
    GraphData { nodes, links, csr }
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
            let name = props
                .iter()
                .find(|(k, _)| k == "name")
                .or_else(|| props.iter().find(|(k, _)| k == "id"))
                .or_else(|| props.iter().find(|(k, _)| k == "title"))
                .map(|(_, val)| value_to_string(val))
                .unwrap_or_else(|| "Node".to_string());
            merge_node(
                &mut nodes,
                GraphNode {
                    id: id_to_string(node_val.get_node_id()),
                    name,
                    label: node_val.get_label_name().clone(),
                    table_id: Some(node_val.get_node_id().table_id),
                    rowid: Some(node_val.get_node_id().offset),
                    community: None,
                    expansion_kind: None,
                    expand_node_id: None,
                    offset: None,
                    hidden_count: None,
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

    Ok(graph_data(nodes.into_values().collect(), links))
}

fn add_expanders(graph: &GraphData, visible_ids: &HashSet<String>, nodes: &mut Vec<GraphNode>, links: &mut Vec<GraphLink>) {
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
                    .filter(|neighbor_id| !visible_ids.contains(*neighbor_id) && known_ids.contains(*neighbor_id))
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

fn seed_graph_from_full(full_graph: GraphData) -> GraphData {
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
    graph_data(nodes, links)
}

fn expand_node_from_full(full_graph: GraphData, node_id: &str, visible_node_ids: &[String], offset: usize) -> GraphData {
    let visible_ids: HashSet<String> = visible_node_ids
        .iter()
        .filter(|id| !id.starts_with(EXPANDER_PREFIX))
        .cloned()
        .collect();

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

    let mut nodes: Vec<GraphNode> = selected_ids
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

    graph_data(nodes, links)
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
        _ => std::env::current_dir().unwrap_or_else(|_| state.data_dir.clone()),
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
fn get_graph(state: State<AppState>, id: usize) -> Result<GraphData, String> {
    let databases = get_all_databases(&state);
    let db_info = databases.get(id).ok_or("Database not found")?;

    let db = Database::new(&db_info.path, SystemConfig::default())
        .map_err(|e| format!("Failed to open database: {}", e))?;
    let conn = Connection::new(&db).map_err(|e| format!("Failed to create connection: {}", e))?;

    collect_edge_graph(&conn, EDGE_SCAN_LIMIT).map(seed_graph_from_full)
}

#[tauri::command]
fn expand_node(
    state: State<AppState>,
    id: usize,
    node_id: String,
    visible_node_ids: Vec<String>,
    offset: Option<usize>,
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
    ))
}

#[tauri::command]
fn execute_query(state: State<AppState>, id: usize, query: String) -> Result<GraphData, String> {
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
                        let name = props
                            .iter()
                            .find(|(k, _)| k == "name")
                            .or_else(|| props.iter().find(|(k, _)| k == "id"))
                            .or_else(|| props.iter().find(|(k, _)| k == "title"))
                            .map(|(_, v)| value_to_string(v))
                            .unwrap_or_else(|| "Node".to_string());

                        let label = node_val.get_label_name().clone();

                        nodes.push(GraphNode {
                            id: node_id,
                            name,
                            label,
                            table_id: Some(node_val.get_node_id().table_id),
                            rowid: Some(node_val.get_node_id().offset),
                            community: None,
                            expansion_kind: None,
                            expand_node_id: None,
                            offset: None,
                            hidden_count: None,
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

    Ok(graph_data(nodes, links))
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
            add_database,
            get_directories,
            get_graph,
            expand_node,
            execute_query,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
