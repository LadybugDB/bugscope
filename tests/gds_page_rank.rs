//! GDS_PAGE_RANK coverage, ported from
//! https://github.com/LadybugDB/extensions/blob/main/algo/test/test_files/gds_page_rank.test
//!
//! Requires the bundled algo extension (`extensions/algo/libalgo.lbug_extension`,
//! built by `scripts/build_algo_extension.sh`, or `BUGSCOPE_ALGO_EXTENSION`).
//! The test skips gracefully when the extension is absent so `cargo test`
//! stays green on machines without the native bundle.

use bugscope::backend::{self, GraphData};
use lbug::{Database, SystemConfig, Value};

fn open_ephemeral() -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("gds.lbdb");
    let db = Database::new(path.to_str().unwrap(), SystemConfig::default()).unwrap();
    (dir, db)
}

fn exec(conn: &lbug::Connection, q: &str) {
    conn.query(q)
        .unwrap_or_else(|e| panic!("query failed: {q}\n{e}"));
}

/// (id, rank) rows from `CALL GDS_PAGE_RANK(...) RETURN node.id, rank ...`.
fn page_rank_rows(conn: &lbug::Connection, graph: &str) -> Vec<(i64, f64)> {
    let mut out = Vec::new();
    let mut result = conn
        .query(&format!(
            "CALL GDS_PAGE_RANK('{graph}') RETURN node.id, rank ORDER BY rank DESC, node.id"
        ))
        .expect("GDS_PAGE_RANK call failed");
    for row in &mut result {
        let (Value::Int64(id), Value::Double(rank)) = (&row[0], &row[1]) else {
            panic!("unexpected GDS_PAGE_RANK row: {row:?}");
        };
        out.push((*id, *rank));
    }
    out
}

fn approx(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-5
}

#[test]
fn gds_page_rank_star() {
    let (_dir, db) = open_ephemeral();
    let conn = lbug::Connection::new(&db).unwrap();
    // INSTALL first: skip only when the extension repo is unreachable
    // (offline). A successful install followed by a failed LOAD is a real
    // failure, so assert instead of skipping.
    if !backend::install_algo_extension(&conn) {
        eprintln!("SKIP gds_page_rank_star: cannot INSTALL algo (offline?)");
        return;
    }
    assert!(
        backend::ensure_algo_extension(&conn),
        "algo installed but LOAD algo failed"
    );

    // --- GDSPageRankStar setup ---
    exec(&conn, "CREATE NODE TABLE N(id INT64 PRIMARY KEY)");
    exec(&conn, "CREATE REL TABLE E(FROM N TO N)");
    exec(
        &conn,
        "CREATE (a:N{id:0}), (b:N{id:1}), (c:N{id:2}), (d:N{id:3})",
    );
    for leaf in [1, 2, 3] {
        exec(
            &conn,
            &format!("MATCH (x:N{{id:{leaf}}}), (y:N{{id:0}}) CREATE (x)-[:E]->(y)"),
        );
    }

    // Plain projection consumes the materialized arrow CSR (zero-copy path).
    exec(&conn, "CALL PROJECT_GRAPH('G', ['N'], ['E'])");
    let rows = page_rank_rows(&conn, "G");
    assert_eq!(rows.len(), 4);
    // Hub (id 0) ranks highest; scores are a sum-to-1 distribution.
    assert_eq!(rows[0].0, 0);
    assert!(approx(rows[0].1, 0.479_730));
    for (_, rank) in rows.iter().skip(1) {
        assert!(approx(*rank, 0.173_423), "leaf rank {rank}");
    }
    let total: f64 = rows.iter().map(|(_, r)| r).sum();
    assert!(approx(total, 1.0), "sum-to-1 distribution, got {total}");

    // Filtered projection goes through the scan fallback: identical ranks.
    exec(
        &conn,
        "CALL PROJECT_GRAPH('GPred', ['N'], {E: 'r.rowid >= 0'})",
    );
    assert_eq!(page_rank_rows(&conn, "GPred"), rows);

    // Cross-check: the in-process icebug PageRank agrees on the ORDERING —
    // hub first, leaves tied. (Exact scores differ: sink/damping conventions
    // vary between implementations; the GDS values above are authoritative.)
    let data: GraphData = backend::run_cypher(&conn, "MATCH (a)-[r]->(b) RETURN a, r, b").unwrap();
    let local = backend::graphr_page_rank(&data);
    assert_eq!(local.len(), 4);
    assert_eq!(local[0].0, "0:0");
    assert!(
        local.iter().skip(1).all(|(_, r)| approx(*r, local[1].1)),
        "leaves tied: {local:?}"
    );
}

#[test]
fn gds_page_rank_stale_projection_serves_live_storage() {
    let (_dir, db) = open_ephemeral();
    let conn = lbug::Connection::new(&db).unwrap();
    if !backend::install_algo_extension(&conn) {
        eprintln!("SKIP gds_page_rank_stale_projection: cannot INSTALL algo (offline?)");
        return;
    }
    assert!(
        backend::ensure_algo_extension(&conn),
        "algo installed but LOAD algo failed"
    );

    exec(&conn, "CREATE NODE TABLE N(id INT64 PRIMARY KEY)");
    exec(&conn, "CREATE REL TABLE E(FROM N TO N)");
    exec(
        &conn,
        "CREATE (a:N{id:0}), (b:N{id:1}), (c:N{id:2}), (d:N{id:3})",
    );
    for leaf in [1, 2] {
        exec(
            &conn,
            &format!("MATCH (x:N{{id:{leaf}}}), (y:N{{id:0}}) CREATE (x)-[:E]->(y)"),
        );
    }
    exec(&conn, "CALL PROJECT_GRAPH('GS', ['N'], ['E'])");
    // Mutate AFTER projection (node cardinality unchanged): the pinned CSR is
    // stale, the epoch check must reject it, and the fallback serves LIVE
    // storage — the full 3-leaf star, not the 2-edge projected graph.
    exec(&conn, "MATCH (x:N{id:3}), (y:N{id:0}) CREATE (x)-[:E]->(y)");
    let rows = page_rank_rows(&conn, "GS");
    assert_eq!(rows.len(), 4);
    assert_eq!(rows[0].0, 0);
    assert!(approx(rows[0].1, 0.479_730));
}
