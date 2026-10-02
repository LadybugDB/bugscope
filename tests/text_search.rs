//! Text-search regression test: `search_nodes` pushes the predicate into
//! Cypher per node table instead of materializing every node, but results
//! must match the legacy semantics exactly — case-insensitive substring over
//! display name, label, and all properties, exact matches first.

use bugscope::backend;
use lbug::{Connection, Database, SystemConfig, Value};

fn open_ephemeral() -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("text_search.lbdb");
    let db = Database::new(path.to_str().unwrap(), SystemConfig::default()).unwrap();
    (dir, db)
}

fn exec(conn: &Connection, q: &str) {
    conn.query(q)
        .unwrap_or_else(|e| panic!("query failed: {q}\n{e}"));
}

fn seed(conn: &Connection) {
    exec(
        conn,
        "CREATE NODE TABLE Doc(title STRING, views INT64, PRIMARY KEY(title))",
    );
    exec(conn, "CREATE (d:Doc{title:'Rust', views:10})");
    exec(conn, "CREATE (d:Doc{title:'Rust Book', views:100})");
    exec(conn, "CREATE (d:Doc{title:\"O'Brien Memoir\", views:7})");
    exec(conn, "CREATE (d:Doc{title:'Fluent Rust', views:25})");
}

fn names(conn: &Connection, term: &str) -> Vec<String> {
    backend::search_nodes(conn, term)
        .unwrap_or_else(|e| panic!("search failed for {term:?}\n{e:#}"))
        .iter()
        .map(|n| n.name.clone())
        .collect()
}

#[test]
fn server_side_search_matches_legacy_semantics() {
    let (_dir, db) = open_ephemeral();
    let conn = Connection::new(&db).unwrap();
    seed(&conn);

    // Substring, exact match first, then alphabetical.
    assert_eq!(
        names(&conn, "rust"),
        vec!["Rust", "Fluent Rust", "Rust Book"],
        "exact first, then by name"
    );
    // Case-insensitive.
    assert_eq!(
        names(&conn, "RUST"),
        vec!["Rust", "Fluent Rust", "Rust Book"]
    );
    // Quote in the term must not break the generated Cypher literal.
    assert_eq!(names(&conn, "o'brien"), vec!["O'Brien Memoir"]);
    assert_eq!(names(&conn, "brien"), vec!["O'Brien Memoir"]);
    // Numeric property rendered as text.
    assert_eq!(names(&conn, "25"), vec!["Fluent Rust"]);
    // Table label matches every node of the table.
    assert_eq!(names(&conn, "doc").len(), 4, "label match");
    // Empty term matches nothing.
    assert!(names(&conn, "   ").is_empty());
    // No match at all.
    assert!(names(&conn, "zzz-no-such-thing").is_empty());
}

#[test]
fn internal_id_search_still_works() {
    let (_dir, db) = open_ephemeral();
    let conn = Connection::new(&db).unwrap();
    seed(&conn);

    // Grab a real internal id ("table:offset") and search for it: terms with
    // ':' keep the legacy scan, and the exact-id sort puts it first.
    let all = backend::search_nodes(&conn, "doc").unwrap();
    assert_eq!(all.len(), 4);
    let target = all[1].id.clone();
    assert!(target.contains(':'), "internal id shape: {target}");
    let hits = backend::search_nodes(&conn, &target).unwrap();
    let hit_ids: Vec<&str> = hits.iter().map(|n| n.id.as_str()).collect();
    assert!(
        hit_ids.contains(&target.as_str()),
        "id {target} found in {hit_ids:?}"
    );
    assert_eq!(hits[0].id, target, "exact id sorts first");
}

/// Index types present in `SHOW_INDEXES`: the app may load the FTS
/// extension and read existing indexes, but must never build one.
fn index_types(conn: &Connection) -> Vec<String> {
    let mut out = Vec::new();
    let mut r = conn.query("CALL SHOW_INDEXES() RETURN *").unwrap();
    for row in &mut r {
        if row.len() >= 3 {
            if let Value::String(kind) = &row[2] {
                out.push(kind.clone());
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

#[test]
fn searches_never_build_fts_indexes() {
    let (_dir, db) = open_ephemeral();
    let conn = Connection::new(&db).unwrap();
    seed(&conn);
    assert_eq!(index_types(&conn), vec!["HASH".to_string()]);

    // Exercise every search tier: plain terms, quoted terms, id terms.
    for term in [
        "rust",
        "RUST",
        "o'brien",
        "25",
        "doc",
        "0:0",
        "zzz-no-such-thing",
    ] {
        let _ = backend::search_nodes(&conn, term).unwrap();
    }
    assert_eq!(
        index_types(&conn),
        vec!["HASH".to_string()],
        "search must not CREATE_FTS_INDEX"
    );
}

fn seed_art(conn: &Connection) {
    // NOTE: the ART index must live on a non-PK column — creating one on
    // the primary key itself is rejected ("already has a HASH primary-key
    // index"). PK on `id`, ART on `name` is the realistic setup.
    exec(
        conn,
        "CREATE NODE TABLE Person(id INT64, name STRING, PRIMARY KEY(id))",
    );
    exec(conn, "CREATE (p:Person{id:1, name:'smith'})");
    exec(conn, "CREATE (p:Person{id:2, name:'smithson'})");
    exec(conn, "CREATE (p:Person{id:3, name:'blacksmith'})");
    exec(conn, "CREATE (p:Person{id:4, name:'Smith'})");
    // Proven safe in this environment (unlike CREATE_FTS_INDEX, which
    // aborts the process): builds a real ART index to search through.
    exec(conn, "CREATE ART INDEX pname FOR (n:Person) ON (n.name)");
}

#[test]
fn art_index_is_first_priority() {
    let (_dir, db) = open_ephemeral();
    let conn = Connection::new(&db).unwrap();
    seed_art(&conn);

    // Discovery sees the index (read-only; the search below builds none).
    let arts = backend::art_columns(&conn);
    assert!(
        arts.iter()
            .any(|c| c.table == "Person" && c.property == "name"),
        "ART discovery: {arts:?}"
    );

    // Exact term: ART equality serves it without scanning.
    assert_eq!(names(&conn, "smithson"), vec!["smithson"]);
    // Exact + prefix share one tier query: both come back together.
    assert_eq!(names(&conn, "smith"), vec!["smith", "smithson"]);
    // Uppercase term still hits via the lowercased constant.
    assert_eq!(names(&conn, "SMITH"), vec!["smith", "smithson"]);
    // First non-empty tier wins: the ART hit above skips the scan, so the
    // case-variant "Smith" and the infix-only "blacksmith" are absent even
    // though a substring scan would find them. That is the documented cost
    // of preferring the index.
    assert!(!names(&conn, "smith").contains(&"Smith".to_string()));
    assert!(!names(&conn, "smith").contains(&"blacksmith".to_string()));
}

#[test]
fn art_miss_falls_through_to_scan() {
    let (_dir, db) = open_ephemeral();
    let conn = Connection::new(&db).unwrap();
    seed_art(&conn);

    // "black" matches nothing by exact/prefix, so the ART tier misses and
    // the CONTAINS scan still finds the infix-only node.
    assert_eq!(names(&conn, "black"), vec!["blacksmith"]);
    // Total miss stays empty without drama.
    assert!(names(&conn, "zzz-no-such-thing").is_empty());
}
