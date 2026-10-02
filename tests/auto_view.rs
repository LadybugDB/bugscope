use bugscope::ui::{auto_view_mode, ViewMode, TREEMAP_AUTO_EDGES};

#[test]
fn treemap_is_default_above_threshold() {
    assert_eq!(auto_view_mode(0), ViewMode::Graph);
    assert_eq!(auto_view_mode(TREEMAP_AUTO_EDGES), ViewMode::Graph);
    assert_eq!(auto_view_mode(TREEMAP_AUTO_EDGES + 1), ViewMode::Treemap);
}
