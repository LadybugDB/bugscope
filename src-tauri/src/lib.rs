use lbug::{Connection, Database, SystemConfig, Value};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::State;
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphLink {
    source: String,
    target: String,
    label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GraphData {
    nodes: Vec<GraphNode>,
    links: Vec<GraphLink>,
}

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
    let mut all: Vec<DatabaseInfo> = scanned
        .into_iter()
        .chain(custom.iter().cloned())
        .collect();
    for (i, db) in all.iter_mut().enumerate() {
        db.id = i;
    }
    all
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

#[tauri::command]
fn get_databases(state: State<AppState>) -> Vec<DatabaseInfo> {
    get_all_databases(&state)
}

#[tauri::command]
fn add_database(state: State<AppState>, file_path: String) -> Result<DatabaseInfo, String> {
    if file_path.is_empty() {
        return Err("filePath is required".to_string());
    }

    let abs_path = if Path::new(&file_path).is_absolute() {
        PathBuf::from(&file_path)
    } else {
        std::env::current_dir()
            .unwrap_or_default()
            .join(&file_path)
    };

    if !abs_path.exists() {
        return Err("File not found".to_string());
    }

    if abs_path.extension().and_then(|e| e.to_str()) != Some("lbdb") {
        return Err("Only .lbdb files are supported".to_string());
    }

    let abs_path_str = abs_path.to_string_lossy().to_string();

    let mut custom = state.custom_databases.lock().unwrap();
    if custom.iter().any(|d| d.path == abs_path_str) {
        return Err("Database already added".to_string());
    }

    let name = abs_path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let db_info = DatabaseInfo {
        id: 0, // will be recalculated
        name,
        path: abs_path_str.clone(),
        relative_path: abs_path_str,
    };
    custom.push(db_info.clone());

    // Return with correct id
    drop(custom);
    let all = get_all_databases(&state);
    Ok(all.last().cloned().unwrap_or(db_info))
}

#[tauri::command]
fn get_directories(state: State<AppState>, path: Option<String>) -> Result<DirectoryListing, String> {
    let dir = match &path {
        Some(p) if !p.is_empty() => {
            let resolved = PathBuf::from(p);
            if resolved.is_absolute() {
                resolved
            } else {
                state.data_dir.join(p)
            }
        }
        _ => state.data_dir.clone(),
    };

    let entries = fs::read_dir(&dir).map_err(|e| e.to_string())?;

    let mut directories = Vec::new();
    let mut files = Vec::new();

    for entry in entries.flatten() {
        let entry_path = entry.path();
        let name = entry
            .file_name()
            .to_string_lossy()
            .to_string();

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
    let conn = Connection::new(&db)
        .map_err(|e| format!("Failed to create connection: {}", e))?;

    // Query nodes
    let mut nodes_result = conn
        .query("MATCH (n) RETURN n, LABEL(n) as label, ID(n) as nodeId LIMIT 500")
        .map_err(|e| format!("Node query failed: {}", e))?;

    let mut nodes = Vec::new();
    for row in &mut nodes_result {
        // row is Vec<Value> with 3 columns: [node, label, nodeId]
        if row.len() < 3 {
            continue;
        }

        let node_id = match &row[2] {
            Value::InternalID(id) => id_to_string(id),
            _ => continue,
        };

        let label = match &row[1] {
            Value::String(s) => s.clone(),
            _ => "Node".to_string(),
        };

        // Extract display name from node properties
        let name = match &row[0] {
            Value::Node(node_val) => {
                let props = node_val.get_properties();
                let name_val = props
                    .iter()
                    .find(|(k, _)| k == "name")
                    .or_else(|| props.iter().find(|(k, _)| k == "id"))
                    .or_else(|| props.iter().find(|(k, _)| k == "title"));
                match name_val {
                    Some((_, val)) => value_to_string(val),
                    None => "Node".to_string(),
                }
            }
            _ => "Node".to_string(),
        };

        nodes.push(GraphNode {
            id: node_id,
            name,
            label,
        });
    }

    // Query links
    let mut links_result = conn
        .query("MATCH (a)-[r]->(b) RETURN ID(a) as src, ID(b) as dst, LABEL(r) as relType LIMIT 500")
        .map_err(|e| format!("Link query failed: {}", e))?;

    let node_id_set: HashSet<String> = nodes.iter().map(|n| n.id.clone()).collect();

    let mut links = Vec::new();
    for row in &mut links_result {
        // row is Vec<Value> with 3 columns: [src, dst, relType]
        if row.len() < 3 {
            continue;
        }

        let source = match &row[0] {
            Value::InternalID(id) => id_to_string(id),
            _ => continue,
        };

        let target = match &row[1] {
            Value::InternalID(id) => id_to_string(id),
            _ => continue,
        };

        let label = match &row[2] {
            Value::String(s) => s.clone(),
            _ => String::new(),
        };

        // Only include links where both endpoints exist in our node set
        if node_id_set.contains(&source) && node_id_set.contains(&target) {
            links.push(GraphLink {
                source,
                target,
                label,
            });
        }
    }

    Ok(GraphData { nodes, links })
}

#[tauri::command]
fn execute_query(state: State<AppState>, id: usize, query: String) -> Result<GraphData, String> {
    let databases = get_all_databases(&state);
    let db_info = databases.get(id).ok_or("Database not found")?;

    let db = Database::new(&db_info.path, SystemConfig::default())
        .map_err(|e| format!("Failed to open database: {}", e))?;
    let conn = Connection::new(&db)
        .map_err(|e| format!("Failed to create connection: {}", e))?;

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

    Ok(GraphData { nodes, links })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let data_dir = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));

    tauri::Builder::default()
        .manage(AppState {
            custom_databases: Mutex::new(Vec::new()),
            data_dir,
        })
        .invoke_handler(tauri::generate_handler![
            get_databases,
            add_database,
            get_directories,
            get_graph,
            execute_query,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
