//! The bounded walk behind an entrypoint's reach (XS-20): the indexer counts it at HEAD and the
//! query layer runs it at any date, over the same function, so the two can't disagree.

/// Hops from the start: a cyclic or very deep graph stops here.
pub const MAX_DEPTH: usize = 64;
/// Nodes visited, start included.
pub const MAX_NODES: usize = 50_000;

/// What a walk found.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Reached {
    /// Every node reached but the start, nearest first.
    pub nodes: Vec<u32>,
    /// A bound stopped the walk before it ran out of edges.
    pub truncated: bool,
}

/// Breadth-first from `start` over a graph of `n` nodes. `step(node, push)` calls `push` once for
/// each node `node` leads to.
pub fn bfs(n: usize, start: u32, mut step: impl FnMut(u32, &mut dyn FnMut(u32))) -> Reached {
    let mut out = Reached::default();
    if start as usize >= n {
        return out;
    }
    let mut seen = vec![0u64; n.div_ceil(64)];
    let mut mark = |i: u32| {
        let (w, b) = (i as usize / 64, 1u64 << (i % 64));
        let fresh = seen[w] & b == 0;
        seen[w] |= b;
        fresh
    };
    mark(start);
    let mut frontier = vec![start];
    let mut depth = 0;
    while !frontier.is_empty() {
        if depth == MAX_DEPTH {
            out.truncated = true;
            break;
        }
        depth += 1;
        let mut next = Vec::new();
        for &node in &frontier {
            step(node, &mut |m| {
                if out.truncated || (m as usize) >= n || !mark(m) {
                    return;
                }
                if out.nodes.len() + 1 >= MAX_NODES {
                    out.truncated = true;
                    return;
                }
                out.nodes.push(m);
                next.push(m);
            });
        }
        if out.truncated {
            break;
        }
        frontier = next;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(edges: &[(u32, u32)], n: usize, start: u32) -> Reached {
        bfs(n, start, |a, push| {
            edges.iter().filter(|e| e.0 == a).for_each(|e| push(e.1));
        })
    }

    #[test]
    fn follows_edges_once_and_survives_a_cycle() {
        let r = run(&[(0, 1), (1, 2), (2, 0), (0, 2)], 4, 0);
        assert_eq!(r.nodes, vec![1, 2]);
        assert!(!r.truncated);
    }

    #[test]
    fn a_chain_longer_than_the_depth_is_cut() {
        let edges: Vec<_> = (0..100).map(|i| (i, i + 1)).collect();
        let r = run(&edges, 101, 0);
        assert_eq!(r.nodes.len(), MAX_DEPTH);
        assert!(r.truncated);
    }

    #[test]
    fn a_wide_graph_stops_at_the_node_cap() {
        let n = MAX_NODES + 10;
        let edges: Vec<_> = (1..n as u32).map(|i| (0, i)).collect();
        let r = run(&edges, n, 0);
        assert_eq!(r.nodes.len(), MAX_NODES - 1);
        assert!(r.truncated);
    }
}
