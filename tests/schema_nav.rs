//! Schema-navigation regression test: the subset built by
//! `backend::run_schema_subset` must contain only the selected types.
//!
//! Covers the single-type query, the multi-type alternation, the edge-only
//! shape, isolated nodes of selected types, and the empty-selection error
//! (which must never reach the database as a query).

use bugscope::backend::{self, GraphData};
use lbug::{Connection, Database, SystemConfig};

fn open_ephemeral() -> (tempfile::TempDir, Database) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("schema_nav.lbdb");
    let db = Database::new(path.to_str().unwrap(), SystemConfig::default()).unwrap();
    (dir, db)
}

fn exec(conn: &Connection, q: &str) {
    conn.query(q)
        .unwrap_or_else(|e| panic!("query failed: {q}\n{e}"));
}

/// Person -[:KNOWS]-> Person, an unrelated Movie -[:LIKES]-> Person,
/// and an isolated Person (no edges at all).
fn seed(conn: &Connection) {
    exec(conn, "CREATE NODE TABLE Person(id INT64 PRIMARY KEY)");
    exec(conn, "CREATE NODE TABLE Movie(id INT64 PRIMARY KEY)");
    exec(conn, "CREATE REL TABLE KNOWS(FROM Person TO Person)");
    exec(conn, "CREATE REL TABLE LIKES(FROM Movie TO Person)");
    exec(
        conn,
        "CREATE (a:Person{id:0}), (b:Person{id:1}), (m:Movie{id:7})",
    );
    exec(conn, "CREATE (l:Person{id:2})");
    exec(
        conn,
        "MATCH (x:Person{id:0}), (y:Person{id:1}) CREATE (x)-[:KNOWS]->(y)",
    );
    exec(
        conn,
        "MATCH (x:Movie{id:7}), (y:Person{id:1}) CREATE (x)-[:LIKES]->(y)",
    );
}

fn run(conn: &Connection, nodes: &[&str], edges: &[&str]) -> GraphData {
    let n: Vec<String> = nodes.iter().map(|s| s.to_string()).collect();
    let e: Vec<String> = edges.iter().map(|s| s.to_string()).collect();
    backend::run_schema_subset(conn, &n, &e, 10_000)
        .unwrap_or_else(|err| panic!("subset failed: {nodes:?} + {edges:?}\n{err:#}"))
}

/// Every displayed node must carry one of `allowed` labels: no foreign
/// type may leak in through an unbound endpoint.
fn assert_restricted(data: &GraphData, allowed: &[&str]) {
    for n in &data.nodes {
        assert!(
            allowed.contains(&n.label.as_str()),
            "foreign node {} (label {}) in {data:?}",
            n.id,
            n.label
        );
    }
}

#[test]
fn single_node_type_returns_outgoing_neighborhood() {
    let (_dir, db) = open_ephemeral();
    let conn = Connection::new(&db).unwrap();
    seed(&conn);

    // Both endpoints bound (plus the LIMIT UI guard).
    let q = backend::schema_subset_query(&["Person".to_string()], &[], Some(10_000)).unwrap();
    assert_eq!(q, "MATCH (a:Person)-[b]->(c:Person) RETURN * LIMIT 10000");

    let data = run(&conn, &["Person"], &[]);
    assert_eq!(data.links.len(), 1, "one outgoing Person edge: {data:?}");
    assert_eq!(data.links[0].label, "KNOWS");
    // The LIKES edge (Movie source) is excluded; the isolated Person is in.
    assert_restricted(&data, &["Person"]);
    assert_eq!(data.nodes.len(), 3, "edge endpoints + isolated: {data:?}");
}

#[test]
fn multi_type_subset_matches_selected_types() {
    let (_dir, db) = open_ephemeral();
    let conn = Connection::new(&db).unwrap();
    seed(&conn);

    // Both node types, both endpoints restricted.
    let both = run(&conn, &["Person", "Movie"], &[]);
    assert_eq!(both.links.len(), 2, "both outgoing edges: {both:?}");
    assert_restricted(&both, &["Person", "Movie"]);

    // Edge filter narrows to the selected rel type.
    let knows = run(&conn, &["Person", "Movie"], &["KNOWS"]);
    assert_eq!(knows.links.len(), 1, "only KNOWS: {knows:?}");
    assert_eq!(knows.links[0].label, "KNOWS");
    assert_restricted(&knows, &["Person", "Movie"]);

    // Edge-only: endpoints unbound (nothing selected to bind them to).
    let likes = run(&conn, &[], &["LIKES"]);
    assert_eq!(likes.links.len(), 1, "only LIKES: {likes:?}");

    // A node type with no self-edges still excludes foreigners: Movie alone
    // has no Movie–Movie edge, so only its (edgeless here) node set shows.
    let movie = run(&conn, &["Movie"], &[]);
    assert_restricted(&movie, &["Movie"]);
    assert!(
        movie.links.is_empty(),
        "no Movie-Movie edge exists: {movie:?}"
    );

    // Empty selection builds no query and errors instead of hitting the DB.
    assert_eq!(backend::schema_subset_query(&[], &[], Some(10)), None);
    assert!(
        backend::run_schema_subset(&conn, &[], &[], 10).is_err(),
        "empty selection must not query"
    );
}
