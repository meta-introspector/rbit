//! Commit graph pages with tracks that continue across page boundaries.

use crate::{urls::RepoUrls, views::repo::CommitView};
use rbit_git::{RefSummary, reader::BranchHistoryPage};
use std::collections::HashMap;

type BranchLinks = Vec<(String, String)>;

#[derive(Debug)]
pub struct GraphEdge {
    pub path: String,
    pub lane: usize,
    pub color: usize,
}

#[derive(Debug)]
pub struct GraphRow {
    pub commit: CommitView,
    pub branches: Vec<(String, String)>,
    pub more_branches: usize,
    pub edges: Vec<GraphEdge>,
    pub x: usize,
    pub lane: usize,
    pub color: usize,
    pub is_merge: bool,
}

#[derive(Debug, Default)]
pub struct BranchGraph {
    pub rows: Vec<GraphRow>,
    pub width: usize,
    pub snapshot: String,
    pub start_url: String,
    pub next_url: Option<String>,
}

impl BranchGraph {
    pub fn new(history: &BranchHistoryPage, refs: &[RefSummary], urls: &RepoUrls) -> Self {
        let mut labels: HashMap<&str, (BranchLinks, usize)> = HashMap::new();
        for reference in refs {
            let (visible, extra) = labels.entry(&reference.target).or_default();
            if visible.len() < 8 {
                visible.push((reference.name.clone(), urls.tree(&format!("refs/heads/{}", reference.name), "")));
            } else {
                *extra += 1;
            }
        }
        // A track keeps its identity as columns compact. The first parent keeps
        // its child's color; additional merge parents get separate tracks.
        let mut lanes: Vec<(&str, usize)> = Vec::new();
        let mut next_lane = 0;
        let mut rows = Vec::new();
        let mut max_lanes = 1;
        let preceding = history.preceding.iter().map(|commit| (&commit.id, &commit.parents, None));
        let visible = history.commits.iter().map(|commit| (&commit.id, &commit.parent_ids, Some(commit)));
        for (id, parents, commit) in preceding.chain(visible) {
            let incoming = lanes.len();
            let column = lanes.iter().position(|(target, _)| *target == id.as_str()).unwrap_or_else(|| {
                lanes.push((id, next_lane));
                next_lane += 1;
                lanes.len() - 1
            });
            let lane = lanes[column].1;
            max_lanes = max_lanes.max(lanes.len());
            let before = commit.map(|_| lanes.clone());
            lanes.remove(column);
            for (offset, parent) in parents.iter().enumerate() {
                if !lanes.iter().any(|(target, _)| *target == parent.as_str()) {
                    let parent_lane = if offset == 0 {
                        lane
                    } else {
                        let id = next_lane;
                        next_lane += 1;
                        id
                    };
                    lanes.insert((column + offset).min(lanes.len()), (parent, parent_lane));
                }
            }
            max_lanes = max_lanes.max(lanes.len());
            let (Some(commit), Some(before)) = (commit, before) else { continue };
            let mut edges: Vec<_> = before
                .iter()
                .enumerate()
                .take(incoming)
                .map(|(column, (_, lane))| GraphEdge { path: format!("M{} 0V36", x(column)), lane: *lane, color: lane % 8 })
                .collect();
            let destinations: HashMap<_, _> = lanes.iter().enumerate().map(|(i, (id, _))| (*id, i)).collect();
            for (source, (id, source_lane)) in before.iter().enumerate() {
                if source == column {
                    for parent in &commit.parent_ids {
                        if let Some(&destination) = destinations.get(parent.as_str()) {
                            let parent_lane = lanes[destination].1;
                            edges.push(edge(source, destination, parent_lane));
                        }
                    }
                } else if let Some(&destination) = destinations.get(id) {
                    edges.push(edge(source, destination, *source_lane));
                }
            }
            let (branches, more_branches) = labels.remove(commit.id.as_str()).unwrap_or_default();
            rows.push(GraphRow {
                commit: CommitView::new(commit, urls),
                branches,
                more_branches,
                edges,
                x: x(column),
                lane,
                color: lane % 8,
                is_merge: commit.parent_ids.len() > 1,
            });
        }
        Self { rows, width: max_lanes * 20 + 12, ..Self::default() }
    }
}

fn edge(source: usize, destination: usize, lane: usize) -> GraphEdge {
    GraphEdge { path: format!("M{} 36C{} 54 {} 54 {} 72", x(source), x(source), x(destination), x(destination)), lane, color: lane % 8 }
}

fn x(column: usize) -> usize {
    column * 20 + 16
}

#[cfg(test)]
mod tests {
    use super::*;
    use rbit_git::{CommitSummary, reader::CommitParents};

    fn commit(id: &str, parents: &[&str]) -> CommitSummary {
        CommitSummary {
            id: id.into(),
            short_id: id.into(),
            summary: id.into(),
            body: None,
            author_name: "Alice".into(),
            author_email: "alice@example.com".into(),
            authored_at: rbit_core::Timestamp(1_700_000_000),
            parent_ids: parents.iter().map(|id| (*id).into()).collect(),
        }
    }

    #[test]
    fn graph_preserves_both_merge_edges() {
        let commits = vec![commit("merge", &["left", "right"]), commit("right", &["root"]), commit("left", &["root"]), commit("root", &[])];
        let graph = BranchGraph::new(&BranchHistoryPage { commits, ..Default::default() }, &[], &RepoUrls::new("acme", "rocket"));
        assert_eq!(graph.rows.iter().map(|row| row.commit.id.as_str()).collect::<Vec<_>>(), ["merge", "right", "left", "root"]);
        assert_eq!(
            graph.rows[0].edges.iter().map(|edge| edge.path.as_str()).collect::<Vec<_>>(),
            ["M16 36C16 54 16 54 16 72", "M16 36C16 54 36 54 36 72"]
        );
        assert_eq!(graph.rows[3].edges[0].path, "M16 0V36");
        assert!(graph.rows[0].is_merge);
        assert!(graph.width >= 52);
    }

    #[test]
    fn track_colors_follow_first_parents_and_survive_column_compaction() {
        let commits = vec![commit("merge", &["left", "right"]), commit("left", &["root"]), commit("right", &["root"]), commit("root", &[])];
        let graph = BranchGraph::new(&BranchHistoryPage { commits, ..Default::default() }, &[], &RepoUrls::new("o", "r"));
        let [merge, left, right, root] = graph.rows.as_slice() else { panic!("expected four rows") };
        assert_eq!(merge.lane, left.lane);
        assert_eq!(left.lane, root.lane);
        assert_ne!(left.color, right.color);
        assert_eq!(right.edges[0].lane, root.lane);
        assert!(right.edges.iter().any(|edge| edge.path == "M36 36C36 54 16 54 16 72" && edge.lane == root.lane));
        assert_eq!(root.x, 16);
    }

    #[test]
    fn later_pages_continue_tracks_from_skipped_merges() {
        let history = BranchHistoryPage {
            preceding: vec![
                CommitParents { id: "merge".into(), parents: vec!["left".into(), "right".into()] },
                CommitParents { id: "left".into(), parents: vec!["root".into()] },
            ],
            commits: vec![commit("right", &["root"]), commit("root", &[])],
            has_more: false,
        };
        let graph = BranchGraph::new(&history, &[], &RepoUrls::new("o", "r"));
        assert_eq!(graph.rows[0].lane, 1);
        assert_eq!(graph.rows[0].x, 36);
        assert_eq!(graph.rows[0].edges[0].path, "M16 0V36");
        assert_eq!(graph.rows[1].lane, 0);
    }

    #[test]
    fn branch_labels_are_bounded_without_losing_the_count() {
        let refs = (0..1020)
            .map(|i| RefSummary { name: format!("branch-{i}"), kind: rbit_git::RefKind::Branch, target: "root".into() })
            .collect::<Vec<_>>();
        let graph = BranchGraph::new(
            &BranchHistoryPage { commits: vec![commit("root", &[])], ..Default::default() },
            &refs,
            &RepoUrls::new("o", "r"),
        );
        assert_eq!(graph.rows[0].branches.len(), 8);
        assert_eq!(graph.rows[0].more_branches, 1012);
    }
}
