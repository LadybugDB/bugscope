//! Loader regression test: `collect_edge_graph` must return every node and
//! edge of a small database, including isolated nodes.
//!
//! Regression coverage for a real-world failure where the topology scan
//! silently decoded zero rows (Arrow-array version skew between lbug and
//! this crate, plus empty CSR metadata), so the UI showed "0 nodes" for a
//! database holding thousands of them. Whatever fast path / fallback runs,
//! the loader's contract is the full edge-bounded graph.

use bugscope::backend::{self, GraphData};
use lbug::{Database, SystemConfig};
use std::collections::HashSet;

fn open_ephemeral() -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("loader.lbdb");
    let db = Database::new(path.to_str().unwrap(), SystemConfig::default()).unwrap();
    (dir, db)
}

fn exec(conn: &lbug::Connection, q: &str) {
    conn.query(q)
        .unwrap_or_else(|e| panic!("query failed: {q}\n{e}"));
}

#[test]
fn collect_edge_graph_returns_everything() {
    let (_dir, db) = open_ephemeral();
    let conn = lbug::Connection::new(&db).unwrap();
    exec(&conn, "CREATE NODE TABLE N(id INT64 PRIMARY KEY)");
    exec(&conn, "CREATE REL TABLE E(FROM N TO N)");
    // A 4-star (hub 0 <- leaves 1,2,3) plus an isolated node 4.
    exec(
        &conn,
        "CREATE (a:N{id:0}), (b:N{id:1}), (c:N{id:2}), (d:N{id:3}), (e:N{id:4})",
    );
    for leaf in [1, 2, 3] {
        exec(
            &conn,
            &format!("MATCH (x:N{{id:{leaf}}}), (y:N{{id:0}}) CREATE (x)-[:E]->(y)"),
        );
    }

    let data: GraphData = backend::collect_edge_graph(&conn, 10_000).unwrap();
    assert_eq!(data.nodes.len(), 5, "all nodes incl. isolated: {data:?}");
    assert_eq!(data.links.len(), 3, "all edges: {data:?}");

    // Every endpoint resolves to a loaded node; labels survive the trip.
    let ids: HashSet<&str> = data.nodes.iter().map(|n| n.id.as_str()).collect();
    assert_eq!(ids.len(), 5);
    for link in &data.links {
        assert!(
            ids.contains(link.source.as_str()),
            "dangling source {}",
            link.source
        );
        assert!(
            ids.contains(link.target.as_str()),
            "dangling target {}",
            link.target
        );
        assert_eq!(link.label, "E");
    }
    assert!(
        data.nodes.iter().all(|n| n.label == "N"),
        "node labels: {data:?}"
    );
}

#[test]
fn collect_edge_graph_empty_db_is_empty() {
    let (_dir, db) = open_ephemeral();
    let conn = lbug::Connection::new(&db).unwrap();
    exec(&conn, "CREATE NODE TABLE N(id INT64 PRIMARY KEY)");
    exec(&conn, "CREATE REL TABLE E(FROM N TO N)");
    let data: GraphData = backend::collect_edge_graph(&conn, 10_000).unwrap();
    assert!(data.nodes.is_empty());
    assert!(data.links.is_empty());
}
