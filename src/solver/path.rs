// shortest paths over the points joined by links: Dijkstra (all distances from one point) and
// A* (one way to one point, a straight line to the goal guides the search). the links are
// undirected. same input gives the same answer: ties go to the smaller position
use super::data::{Links, Points};
use std::cmp::Ordering;
use std::collections::BinaryHeap;

// what a link costs to walk
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Cost {
    // the distance between its ends now (A* may use the straight line as a guide)
    Length,
    // its rest length
    Rest,
    // one per link
    Hops,
}

// neighbours of every point, in one array (compressed rows): row i is adj[start[i]..start[i + 1]]
pub struct Graph {
    start: Vec<u32>,
    adj: Vec<(u32, f64)>,
}

impl Graph {
    // a link with a broken cost (not finite or negative) or with a missing end is left out
    pub fn build(p: &Points, l: &Links, cost: Cost) -> Graph {
        let n = p.len();
        let mut rows: Vec<Vec<(u32, f64)>> = vec![Vec::new(); n];
        for k in 0..l.len() {
            let (a, b) = (l.a[k] as usize, l.b[k] as usize);
            if a >= n || b >= n || a == b {
                continue;
            }
            let c = match cost {
                Cost::Length => ((p.x[a] - p.x[b]).powi(2) + (p.y[a] - p.y[b]).powi(2)).sqrt(),
                Cost::Rest => l.rest[k],
                Cost::Hops => 1.0,
            };
            if !c.is_finite() || c < 0.0 {
                continue;
            }
            rows[a].push((b as u32, c));
            rows[b].push((a as u32, c));
        }
        let mut start = Vec::with_capacity(n + 1);
        let mut adj = Vec::new();
        for mut r in rows {
            // a fixed order whatever the order of the links
            r.sort_by(|x, y| x.0.cmp(&y.0).then(x.1.total_cmp(&y.1)));
            start.push(adj.len() as u32);
            adj.extend(r);
        }
        start.push(adj.len() as u32);
        Graph { start, adj }
    }

    pub fn len(&self) -> usize {
        self.start.len() - 1
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    fn row(&self, i: usize) -> &[(u32, f64)] {
        &self.adj[self.start[i] as usize..self.start[i + 1] as usize]
    }
}

// the heap gives the smallest key first; equal keys: the smaller position
#[derive(PartialEq)]
struct Item {
    key: f64,
    node: u32,
}

impl Eq for Item {}

impl Ord for Item {
    fn cmp(&self, o: &Item) -> Ordering {
        o.key.total_cmp(&self.key).then(o.node.cmp(&self.node))
    }
}

impl PartialOrd for Item {
    fn partial_cmp(&self, o: &Item) -> Option<Ordering> {
        Some(self.cmp(o))
    }
}

// distance from `from` to every point; f64::INFINITY where there is no way
pub fn distances(g: &Graph, from: usize) -> Vec<f64> {
    let mut dist = vec![f64::INFINITY; g.len()];
    if from >= g.len() {
        return dist;
    }
    dist[from] = 0.0;
    let mut heap = BinaryHeap::new();
    heap.push(Item { key: 0.0, node: from as u32 });
    while let Some(Item { key, node }) = heap.pop() {
        let u = node as usize;
        if key > dist[u] {
            continue;
        }
        for &(v, c) in g.row(u) {
            let nd = key + c;
            if nd < dist[v as usize] {
                dist[v as usize] = nd;
                heap.push(Item { key: nd, node: v });
            }
        }
    }
    dist
}

// the cheapest way from `from` to `to`: the positions in order (both ends included) and its
// cost. `guide` is a straight line to the goal, allowed only when the cost of a link is its
// length (then it never over-estimates and the answer is the cheapest one)
pub fn path(g: &Graph, p: &Points, from: usize, to: usize, guide: bool) -> Option<(Vec<u32>, f64)> {
    let n = g.len();
    if from >= n || to >= n {
        return None;
    }
    let h = |i: usize| if guide { ((p.x[i] - p.x[to]).powi(2) + (p.y[i] - p.y[to]).powi(2)).sqrt() } else { 0.0 };
    let mut dist = vec![f64::INFINITY; n];
    let mut prev = vec![u32::MAX; n];
    dist[from] = 0.0;
    let mut heap = BinaryHeap::new();
    heap.push(Item { key: h(from), node: from as u32 });
    while let Some(Item { key, node }) = heap.pop() {
        let u = node as usize;
        if u == to {
            break;
        }
        if key > dist[u] + h(u) {
            continue;
        }
        for &(v, c) in g.row(u) {
            let nd = dist[u] + c;
            if nd < dist[v as usize] {
                dist[v as usize] = nd;
                prev[v as usize] = u as u32;
                heap.push(Item { key: nd + h(v as usize), node: v });
            }
        }
    }
    if !dist[to].is_finite() {
        return None;
    }
    let mut way = vec![to as u32];
    let mut at = to;
    while at != from {
        at = prev[at] as usize;
        way.push(at as u32);
    }
    way.reverse();
    Some((way, dist[to]))
}

#[cfg(test)]
mod tests {
    use super::*;

    // a w x h grid of points 10 apart, every neighbour linked; a wall removes the links of the
    // listed points
    fn grid(w: usize, h: usize, walls: &[usize]) -> (Points, Links) {
        let (mut p, mut l) = (Points::default(), Links::default());
        for j in 0..h {
            for i in 0..w {
                p.add(i as f64 * 10.0, j as f64 * 10.0, 1.0, 1.0);
            }
        }
        for j in 0..h {
            for i in 0..w {
                let a = j * w + i;
                if walls.contains(&a) {
                    continue;
                }
                if i + 1 < w && !walls.contains(&(a + 1)) {
                    l.add(a, a + 1, 10.0, 0.0, 0);
                }
                if j + 1 < h && !walls.contains(&(a + w)) {
                    l.add(a, a + w, 10.0, 0.0, 0);
                }
            }
        }
        (p, l)
    }

    #[test]
    fn distances_on_a_grid_are_manhattan() {
        let (p, l) = grid(5, 4, &[]);
        let d = distances(&Graph::build(&p, &l, Cost::Length), 0);
        assert_eq!(d[4], 40.0);
        assert_eq!(d[19], 70.0);
    }

    #[test]
    fn astar_finds_the_same_cost_as_dijkstra_around_a_wall() {
        // a wall in the middle column with one gap at the top
        let (p, l) = grid(7, 5, &[10, 17, 24, 31]);
        let g = Graph::build(&p, &l, Cost::Length);
        let d = distances(&g, 0);
        let (way, cost) = path(&g, &p, 0, 6 + 28, true).unwrap();
        assert!((cost - d[34]).abs() < 1e-9);
        assert_eq!(way[0], 0);
        assert_eq!(*way.last().unwrap(), 34);
        let (_, cost0) = path(&g, &p, 0, 34, false).unwrap();
        assert!((cost0 - cost).abs() < 1e-9);
    }

    #[test]
    fn no_way_is_none_and_the_same_point_is_a_way_of_one() {
        let (p, l) = grid(3, 3, &[1, 4, 7]);
        let g = Graph::build(&p, &l, Cost::Length);
        assert!(path(&g, &p, 0, 2, true).is_none());
        assert_eq!(distances(&g, 0)[2], f64::INFINITY);
        assert_eq!(path(&g, &p, 0, 0, true).unwrap(), (vec![0], 0.0));
        assert!(path(&g, &p, 0, 99, true).is_none());
    }

    #[test]
    fn costs_length_rest_and_hops() {
        let mut p = Points::default();
        for x in [0.0, 100.0, 200.0] {
            p.add(x, 0.0, 1.0, 1.0);
        }
        let mut l = Links::default();
        l.add(0, 1, 5.0, 0.0, 0);
        l.add(1, 2, 7.0, 0.0, 0);
        assert_eq!(distances(&Graph::build(&p, &l, Cost::Length), 0)[2], 200.0);
        assert_eq!(distances(&Graph::build(&p, &l, Cost::Rest), 0)[2], 12.0);
        assert_eq!(distances(&Graph::build(&p, &l, Cost::Hops), 0)[2], 2.0);
    }

    #[test]
    fn two_equal_ways_always_give_the_same_one() {
        let (p, l) = grid(3, 3, &[]);
        let g = Graph::build(&p, &l, Cost::Length);
        let first = path(&g, &p, 0, 8, true).unwrap();
        for _ in 0..5 {
            assert_eq!(path(&g, &p, 0, 8, true).unwrap(), first);
        }
        assert_eq!(first.1, 40.0);
    }

    #[test]
    fn broken_links_are_left_out() {
        let (p, mut l) = grid(2, 1, &[]);
        l.rest[0] = f64::NAN;
        let g = Graph::build(&p, &l, Cost::Rest);
        assert!(path(&g, &p, 0, 1, false).is_none());
    }

    #[test]
    fn a_thousand_points_in_a_line() {
        let (p, l) = grid(1000, 1, &[]);
        let g = Graph::build(&p, &l, Cost::Length);
        let (way, cost) = path(&g, &p, 0, 999, true).unwrap();
        assert_eq!(way.len(), 1000);
        assert_eq!(cost, 9990.0);
    }
}
