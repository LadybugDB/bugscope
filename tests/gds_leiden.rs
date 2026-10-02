//! GDS_LEIDEN coverage, ported from
//! https://github.com/LadybugDB/extensions/blob/bf74ab654af6bd40a0e3208ac4e8bcf5cb45a7a0/algo/test/test_files/gds_leiden.test
//!
//! Requires the bundled algo extension (`extensions/algo/libalgo.lbug_extension`,
//! built by `scripts/build_algo_extension.sh`, or `BUGSCOPE_ALGO_EXTENSION`).
//! The GDS assertions skip gracefully when the extension is absent so
//! `cargo test` stays green on machines without the native bundle. The local
//! icebug Leiden cross-check always runs (no extension needed).

use bugscope::backend::{self, GraphData};
use lbug::{Database, SystemConfig, Value};

fn open_ephemeral() -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("leiden.lbdb");
    let db = Database::new(path.to_str().unwrap(), SystemConfig::default()).unwrap();
    (dir, db)
}

fn exec(conn: &lbug::Connection, q: &str) {
    conn.query(q)
        .unwrap_or_else(|e| panic!("query failed: {q}\n{e}"));
}

/// `CALL GDS_LEIDEN(graph) RETURN COUNT(DISTINCT community_id)`, with an
/// optional `WITH node, community_id WHERE ...` cohesion filter.
fn distinct_communities(conn: &lbug::Connection, graph: &str, filter: &str) -> i64 {
    let where_clause = if filter.is_empty() {
        String::new()
    } else {
        format!("WITH node, community_id WHERE {filter} ")
    };
    let mut result = conn
        .query(&format!(
            "CALL GDS_LEIDEN('{graph}') {where_clause}RETURN COUNT(DISTINCT community_id)"
        ))
        .expect("GDS_LEIDEN call failed");
    let row = result.next().expect("GDS_LEIDEN returned no rows");
    if row.len() != 1 {
        panic!("unexpected GDS_LEIDEN count row: {row:?}");
    }
    match &row[0] {
        Value::Int64(v) => *v,
        Value::UInt64(v) => *v as i64,
        other => panic!("unexpected GDS_LEIDEN count row: {other:?}"),
    }
}

/// Two 4-cliques joined by a single bridge edge — the `GDSLeidenTwoCliques`
/// case from `gds_leiden.test`.
fn build_two_cliques(conn: &lbug::Connection) {
    exec(conn, "CREATE NODE TABLE Node(id INT64 PRIMARY KEY)");
    exec(conn, "CREATE REL TABLE Edge(FROM Node to Node)");
    exec(
        conn,
        "CREATE (u0:Node {id: 0}), (u1:Node {id: 1}), (u2:Node {id: 2}), (u3:Node {id: 3}),
         (u4:Node {id: 4}), (u5:Node {id: 5}), (u6:Node {id: 6}), (u7:Node {id: 7})",
    );
    let edges = [
        (0, 1),
        (0, 2),
        (1, 2),
        (0, 3),
        (1, 3),
        (2, 3),
        (4, 5),
        (4, 6),
        (5, 6),
        (4, 7),
        (5, 7),
        (6, 7),
        (3, 4), // the bridge
    ];
    for (s, t) in edges {
        exec(
            conn,
            &format!("MATCH (a:Node {{id:{s}}}), (b:Node {{id:{t}}}) CREATE (a)-[:Edge]->(b)"),
        );
    }
    exec(conn, "CALL PROJECT_GRAPH('Graph', ['Node'], ['Edge'])");
}

#[test]
fn gds_leiden_two_cliques() {
    let (_dir, db) = open_ephemeral();
    let conn = lbug::Connection::new(&db).unwrap();
    if !backend::install_algo_extension(&conn) {
        eprintln!("SKIP gds_leiden_two_cliques: cannot INSTALL algo (offline?)");
        return;
    }
    assert!(
        backend::ensure_algo_extension(&conn),
        "algo installed but LOAD algo failed"
    );
    build_two_cliques(&conn);

    // Leiden community IDs are arbitrary and the algorithm randomizes, so
    // assert structure, not exact IDs: two communities, one per clique.
    assert_eq!(distinct_communities(&conn, "Graph", ""), 2);
    assert_eq!(
        distinct_communities(&conn, "Graph", "node.id IN [0, 1, 2, 3]"),
        1
    );
    assert_eq!(
        distinct_communities(&conn, "Graph", "node.id IN [4, 5, 6, 7]"),
        1
    );

    // The Rust helper reads the same rows into id -> community.
    let map = backend::gds_leiden_communities(&conn, "Graph", None).unwrap();
    assert_eq!(map.len(), 8);
    let comm = |id: i64| {
        let key = map
            .keys()
            .find(|k| k.ends_with(&format!(":{id}")))
            .unwrap_or_else(|| panic!("node id {id} missing from {map:?}"));
        map[key]
    };
    assert!(comm(0) == comm(1) && comm(1) == comm(2) && comm(2) == comm(3));
    assert!(comm(4) == comm(5) && comm(5) == comm(6) && comm(6) == comm(7));
    assert_ne!(comm(0), comm(4));

    // Gamma must be positive; unknown params rejected.
    let err = conn
        .query("CALL GDS_LEIDEN('Graph', gamma := -1.0) RETURN COUNT(DISTINCT community_id)")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("Gamma must be positive"),
        "unexpected error: {err}"
    );
    let err = conn
        .query("CALL GDS_LEIDEN('Graph', bogus := 1) RETURN COUNT(DISTINCT community_id)")
        .unwrap_err()
        .to_string();
    assert!(
        err.contains("Unknown optional parameter"),
        "unexpected error: {err}"
    );
}

/// In-process icebug Leiden on the same two-clique shape: needs no extension
/// and must agree on structure. This is what the treemap visualizes.
#[test]
fn local_leiden_two_cliques() {
    let nodes: Vec<bugscope::backend::GraphNode> = (0..8)
        .map(|i| bugscope::backend::GraphNode {
            id: format!("0:{i}"),
            name: format!("u{i}"),
            label: "Node".to_string(),
            properties: std::collections::HashMap::new(),
        })
        .collect();
    let edges = [
        (0, 1),
        (0, 2),
        (1, 2),
        (0, 3),
        (1, 3),
        (2, 3),
        (4, 5),
        (4, 6),
        (5, 6),
        (4, 7),
        (5, 7),
        (6, 7),
        (3, 4),
    ];
    let links = edges
        .iter()
        .map(|(s, t)| bugscope::backend::GraphLink {
            source: format!("0:{s}"),
            target: format!("0:{t}"),
            label: "Edge".to_string(),
            properties: Default::default(),
        })
        .collect();
    let data = GraphData { nodes, links };
    let (assignment, modularity, count) = backend::graphr_leiden_full(&data);
    assert_eq!(assignment.len(), 8);
    assert_eq!(count, 2, "two cliques, two communities: {assignment:?}");
    assert!(
        assignment[0] == assignment[1]
            && assignment[1] == assignment[2]
            && assignment[2] == assignment[3]
    );
    assert!(
        assignment[4] == assignment[5]
            && assignment[5] == assignment[6]
            && assignment[6] == assignment[7]
    );
    assert_ne!(assignment[0], assignment[4]);
    assert!(modularity > 0.3, "dense cliques score well: {modularity}");
}
