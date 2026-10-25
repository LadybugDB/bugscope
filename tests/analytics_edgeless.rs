//! Edgeless graphs must never reach the icebug FFI with empty CSR index
//! arrays (SIGSEGV): node-only Cypher results such as
//! `MATCH (n) RETURN n LIMIT 10` and isolated-node 1-hop neighborhoods flow
//! through `refresh_analytics` (PageRank + Leiden) on every display.

use bugscope::backend::{self, GraphData, GraphLink, GraphNode};
use std::collections::HashMap;

fn node(id: &str) -> GraphNode {
    GraphNode {
        id: id.to_string(),
        name: id.to_string(),
        label: "N".to_string(),
        properties: HashMap::new(),
    }
}

fn isolated(n: usize) -> GraphData {
    GraphData {
        nodes: (0..n).map(|i| node(&format!("3:{i}"))).collect(),
        links: vec![],
    }
}

fn self_loops_only(n: usize) -> GraphData {
    GraphData {
        nodes: (0..n).map(|i| node(&format!("3:{i}"))).collect(),
        links: (0..n)
            .map(|i| GraphLink {
                source: format!("3:{i}"),
                target: format!("3:{i}"),
                label: "SELF".to_string(),
                properties: HashMap::new(),
            })
            .collect(),
    }
}

#[test]
fn pagerank_edgeless_returns_empty() {
    // Previously SIGSEGV (signal 11) inside icebug on the empty CSR.
    assert!(backend::graphr_page_rank(&isolated(10)).is_empty());
    assert!(backend::graphr_page_rank(&isolated(1)).is_empty());
    assert!(backend::graphr_page_rank(&GraphData::default()).is_empty());
}

#[test]
fn leiden_edgeless_is_one_community() {
    let (assign, q, count) = backend::graphr_leiden_full(&isolated(10));
    assert_eq!(assign, vec![0; 10]);
    assert_eq!(q, 0.0);
    assert_eq!(count, 1);
}

#[test]
fn self_loop_only_graphs_do_not_crash() {
    // Self-loops are skipped when packing, so the CSR is empty here too.
    let data = self_loops_only(4);
    assert!(backend::graphr_page_rank(&data).is_empty());
    let (assign, _, count) = backend::graphr_leiden_full(&data);
    assert_eq!(assign, vec![0; 4]);
    assert_eq!(count, 1);
}
