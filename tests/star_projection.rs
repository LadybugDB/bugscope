//! `RETURN a.*, b.*, c.*` flattens to identity-free scalar columns, so the
//! graph extractor alone finds nothing. `run_cypher` re-runs the same
//! (read-only) MATCH with bare aliases and merges, so the explorer shows
//! the matched subgraph.

use bugscope::backend;
use lbug::{Connection, Database, SystemConfig};

fn open_graph() -> (tempfile::TempDir, &'static Database, Connection<'static>) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("star.lbdb");
    let db = Database::new(path.to_str().unwrap(), SystemConfig::default()).unwrap();
    let db: &'static Database = Box::leak(Box::new(db));
    let conn = Connection::new(db).unwrap();
    conn.query("CREATE NODE TABLE P(pid INT64 PRIMARY KEY, name STRING)")
        .unwrap();
    conn.query("CREATE REL TABLE L(FROM P TO P, w DOUBLE)")
        .unwrap();
    for i in 1..=4 {
        conn.query(&format!("CREATE (:P {{pid: {i}, name: 'n{i}'}})"))
            .unwrap();
    }
    // 10 distinct directed edges.
    for (s, d) in [
        (1, 2),
        (2, 3),
        (3, 4),
        (1, 3),
        (2, 4),
        (1, 4),
        (4, 1),
        (3, 1),
        (2, 1),
        (4, 2),
    ] {
        conn.query(&format!(
            "MATCH (a:P {{pid:{s}}}), (b:P {{pid:{d}}}) CREATE (a)-[:L {{w: 0.5}}]->(b)"
        ))
        .unwrap();
    }
    (dir, db, conn)
}

fn count_pid(conn: &Connection, pid: i64) -> i64 {
    let mut r = conn
        .query(&format!("MATCH (n:P {{pid:{pid}}}) RETURN count(n)"))
        .unwrap();
    let row = r.next().unwrap();
    match &row[0] {
        lbug::Value::Int64(v) => *v,
        lbug::Value::UInt64(v) => *v as i64,
        other => panic!("unexpected count value: {other:?}"),
    }
}

#[test]
fn star_projection_yields_the_matched_subgraph() {
    let (_dir, _db, conn) = open_graph();
    let g =
        backend::run_cypher(&conn, "MATCH (a)-[b]->(c) RETURN a.*, b.*, c.* LIMIT 10;").unwrap();
    assert_eq!(g.links.len(), 10, "links: {:?}", g.links);
    assert_eq!(g.nodes.len(), 4, "nodes: {:?}", g.nodes);
    // Parity with the bare-alias form (same MATCH + LIMIT).
    let bare = backend::run_cypher(&conn, "MATCH (a)-[b]->(c) RETURN a, b, c LIMIT 10;").unwrap();
    assert_eq!(bare.links.len(), g.links.len());
    assert_eq!(bare.nodes.len(), g.nodes.len());
}

#[test]
fn plain_scalars_still_yield_no_graph() {
    let (_dir, _db, conn) = open_graph();
    let g = backend::run_cypher(&conn, "MATCH (n:P) RETURN n.pid LIMIT 5;").unwrap();
    assert!(g.nodes.is_empty() && g.links.is_empty());
}

#[test]
fn writes_are_never_executed_twice() {
    let (_dir, _db, conn) = open_graph();
    assert_eq!(count_pid(&conn, 99), 0);
    // The `.*` fallback must NOT re-run the CREATE (that would duplicate
    // the node).
    let g = backend::run_cypher(&conn, "CREATE (n:P {pid: 99, name: 'dup'}) RETURN n.*;").unwrap();
    // `n.*` flattens to scalars (no graph) and the CREATE vetoes the
    // re-run, so the graph stays empty — but the node exists exactly once.
    assert!(g.nodes.is_empty() && g.links.is_empty());
    assert_eq!(count_pid(&conn, 99), 1, "CREATE ran exactly once");
}
