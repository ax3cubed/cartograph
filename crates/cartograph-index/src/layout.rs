//! Layout at index time (IDX-25, I2): every service, module and file gets a circle in one plane,
//! stored in `layout` and read by the app instead of laying out in the browser.
//!
//! Top-down: services in the workspace, then each service's modules (and its loose files) inside
//! its circle, then each module's files inside the module's circle, so a change among files can't
//! move a module or a service. Each parent's children are laid out on their own, in parallel, with
//! our own force layout: pairwise repulsion (Barnes-Hut above [`BH_MIN`] children), springs along
//! the links between siblings, a pull to the parent's centre, and a clamp inside the parent. Fixed
//! seed, fixed iteration count, children in key order: two machines place nodes alike. Positions
//! are kept out of the fact hash.
//!
//! Stability: a node that has a position keeps it. Only new nodes move, starting at their parent's
//! centre, and they settle among their pinned siblings. A parent whose children and weights are
//! unchanged (its inputs key, IDX-21) is skipped. A file that moved to another module is placed
//! again there. A parent with more than [`GRID_ABOVE`] children is packed on a grid.
//!
//! Positions are intervals: a node's first row starts at the step it first exists, so scrubbing
//! back reads them with the same as-of rule as links.
//!
//! ponytail: one layout per run (I2's "snapshot", proposed), not one per calendar month of a first
//! index, and a parent never grows: new children are clamped into its circle as first sized. A
//! service that outgrows its circle is laid out again by `cartograph index --rebuild`, which carries
//! every other position over by node key.

use std::collections::{BTreeMap, HashMap};

use cartograph_store::derived::{KeyInputs, changed, inputs_key, record};
use cartograph_store::rusqlite::{self, Connection, params};
use rayon::prelude::*;

/// Force iterations per parent (fixed, so the cost and the result are fixed).
pub const ITERATIONS: usize = 60;
/// Barnes-Hut replaces pairwise repulsion above this many children.
pub const BH_MIN: usize = 256;
/// Above this many children a parent is packed on a grid (IDX-25 security).
pub const GRID_ABOVE: usize = 20_000;
/// Bump when the algorithm changes what it places: every parent is laid out again.
const VERSION: &str = "layout 1";

/// A circle: centre and radius.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Circle {
    pub x: f64,
    pub y: f64,
    pub r: f64,
}

/// A child to place: its key, its radius and, when it keeps its place, its centre.
pub type Child = (String, f64, Option<(f64, f64)>);

/// One parent's children to place.
#[derive(Debug, Clone)]
pub struct Group {
    /// The parent's circle.
    pub area: Circle,
    /// Children in key order: (key, radius, pinned position).
    pub children: Vec<Child>,
    /// Springs between children: (index, index, weight).
    pub springs: Vec<(usize, usize, f64)>,
}

/// A small deterministic generator (splitmix64), seeded by the parent's key.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> f64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        (z >> 11) as f64 / (1u64 << 53) as f64
    }
}

fn seed(key: &str) -> u64 {
    key.bytes().fold(0xcbf2_9ce4_8422_2325u64, |h, b| {
        (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The radius of a parent holding children of these radii: their area at about 45% packing.
pub fn radius_for(children: impl Iterator<Item = f64>) -> f64 {
    let area: f64 = children.map(|r| r * r).sum();
    (area / 0.45).sqrt().max(1.0) + 1.0
}

/// Places a group's unpinned children; pinned ones stay. Returns each child's centre.
pub fn place(key: &str, g: &Group) -> Vec<(f64, f64)> {
    let n = g.children.len();
    let Circle {
        x: cx,
        y: cy,
        r: area,
    } = g.area;
    let mut rng = Rng(seed(key));
    if n > GRID_ABOVE {
        let side = (n as f64).sqrt().ceil();
        let cell = area * std::f64::consts::SQRT_2 / side;
        let start = -area / std::f64::consts::SQRT_2 + cell / 2.0;
        return (0..n)
            .map(|i| match g.children[i].2 {
                Some(p) => p,
                None => (
                    cx + start + (i as f64 % side) * cell,
                    cy + start + (i as f64 / side).floor() * cell,
                ),
            })
            .collect();
    }
    // New children start at the centre, spread a little so no two coincide.
    let mut pos: Vec<(f64, f64)> = g
        .children
        .iter()
        .map(|(_, _, p)| {
            p.unwrap_or_else(|| {
                let a = rng.next() * std::f64::consts::TAU;
                let d = rng.next().sqrt() * area * 0.3;
                (cx + d * a.cos(), cy + d * a.sin())
            })
        })
        .collect();
    let radius: Vec<f64> = g.children.iter().map(|c| c.1).collect();
    let pinned: Vec<bool> = g.children.iter().map(|c| c.2.is_some()).collect();
    if pinned.iter().all(|p| *p) {
        return pos;
    }
    let mut disp = vec![(0.0f64, 0.0f64); n];
    for t in 0..ITERATIONS {
        let cool = 1.0 - t as f64 / ITERATIONS as f64;
        disp.iter_mut().for_each(|d| *d = (0.0, 0.0));
        if n > BH_MIN {
            let tree = Quad::build(&pos, &radius);
            for i in 0..n {
                if !pinned[i] {
                    let (fx, fy) = tree.force(pos[i], radius[i], i);
                    disp[i].0 += fx;
                    disp[i].1 += fy;
                }
            }
        } else {
            for i in 0..n {
                for j in i + 1..n {
                    let (fx, fy) = repel(pos[i], pos[j], radius[i], radius[j]);
                    disp[i].0 += fx;
                    disp[i].1 += fy;
                    disp[j].0 -= fx;
                    disp[j].1 -= fy;
                }
            }
        }
        for &(i, j, w) in &g.springs {
            let (dx, dy) = (pos[j].0 - pos[i].0, pos[j].1 - pos[i].1);
            let d = (dx * dx + dy * dy).sqrt().max(1e-9);
            let rest = (radius[i] + radius[j]) * 1.5;
            let f = 0.05 * w.min(10.0) * (d - rest) / d;
            disp[i].0 += dx * f;
            disp[i].1 += dy * f;
            disp[j].0 -= dx * f;
            disp[j].1 -= dy * f;
        }
        let step = area * 0.1 * cool + 1e-3;
        for i in 0..n {
            if pinned[i] {
                continue;
            }
            // A pull to the centre keeps the group round.
            let (gx, gy) = ((cx - pos[i].0) * 0.05, (cy - pos[i].1) * 0.05);
            let (dx, dy) = (disp[i].0 + gx, disp[i].1 + gy);
            let len = (dx * dx + dy * dy).sqrt();
            let s = if len > step { step / len } else { 1.0 };
            pos[i] = clamp((pos[i].0 + dx * s, pos[i].1 + dy * s), radius[i], g.area);
        }
    }
    pos
}

fn repel(a: (f64, f64), b: (f64, f64), ra: f64, rb: f64) -> (f64, f64) {
    let (dx, dy) = (a.0 - b.0, a.1 - b.1);
    let d2 = (dx * dx + dy * dy).max(1e-6);
    let k = (ra + rb) * (ra + rb);
    (dx * k / d2, dy * k / d2)
}

/// Keeps a child's circle inside its parent's.
fn clamp(p: (f64, f64), r: f64, area: Circle) -> (f64, f64) {
    let (dx, dy) = (p.0 - area.x, p.1 - area.y);
    let d = (dx * dx + dy * dy).sqrt();
    let max = (area.r - r).max(0.0);
    if d <= max {
        p
    } else if d < 1e-12 {
        (area.x, area.y)
    } else {
        (area.x + dx / d * max, area.y + dy / d * max)
    }
}

/// A Barnes-Hut quadtree: each cell's mass (sum of r²) at its centre of mass.
struct Quad {
    cells: Vec<Cell>,
}

struct Cell {
    cx: f64,
    cy: f64,
    half: f64,
    mass: f64,
    mx: f64,
    my: f64,
    /// A leaf's body, or the first of four children.
    body: Option<usize>,
    kids: Option<usize>,
}

impl Quad {
    fn build(pos: &[(f64, f64)], radius: &[f64]) -> Self {
        let (mut x0, mut y0, mut x1, mut y1) = (f64::MAX, f64::MAX, f64::MIN, f64::MIN);
        for p in pos {
            x0 = x0.min(p.0);
            y0 = y0.min(p.1);
            x1 = x1.max(p.0);
            y1 = y1.max(p.1);
        }
        let half = ((x1 - x0).max(y1 - y0) / 2.0).max(1e-6) * 1.01;
        let mut q = Quad {
            cells: vec![Cell::new((x0 + x1) / 2.0, (y0 + y1) / 2.0, half)],
        };
        for i in 0..pos.len() {
            q.insert(0, i, pos, radius, 0);
        }
        q
    }

    fn insert(&mut self, c: usize, i: usize, pos: &[(f64, f64)], radius: &[f64], depth: u32) {
        let m = radius[i] * radius[i];
        {
            let cell = &mut self.cells[c];
            let total = cell.mass + m;
            cell.mx = (cell.mx * cell.mass + pos[i].0 * m) / total;
            cell.my = (cell.my * cell.mass + pos[i].1 * m) / total;
            cell.mass = total;
        }
        // Coincident bodies past a depth share a leaf, so the tree stays finite.
        if depth > 40 {
            return;
        }
        match (self.cells[c].kids, self.cells[c].body) {
            (Some(k), _) => {
                let q = self.quadrant(c, pos[i]);
                self.insert(k + q, i, pos, radius, depth + 1);
            }
            (None, None) if self.cells[c].mass == m => self.cells[c].body = Some(i),
            (None, old) => {
                let (cx, cy, h) = (self.cells[c].cx, self.cells[c].cy, self.cells[c].half / 2.0);
                let k = self.cells.len();
                for (dx, dy) in [(-1.0, -1.0), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
                    self.cells.push(Cell::new(cx + dx * h, cy + dy * h, h));
                }
                self.cells[c].kids = Some(k);
                self.cells[c].body = None;
                if let Some(o) = old {
                    let q = self.quadrant(c, pos[o]);
                    self.insert(k + q, o, pos, radius, depth + 1);
                }
                let q = self.quadrant(c, pos[i]);
                self.insert(k + q, i, pos, radius, depth + 1);
            }
        }
    }

    fn quadrant(&self, c: usize, p: (f64, f64)) -> usize {
        let cell = &self.cells[c];
        usize::from(p.0 >= cell.cx) + 2 * usize::from(p.1 >= cell.cy)
    }

    /// The repulsion on body `i` at `p`, approximating far cells (theta 0.8).
    fn force(&self, p: (f64, f64), r: f64, i: usize) -> (f64, f64) {
        let mut f = (0.0, 0.0);
        let mut stack = vec![0usize];
        while let Some(c) = stack.pop() {
            let cell = &self.cells[c];
            if cell.mass == 0.0 || cell.body == Some(i) {
                continue;
            }
            let (dx, dy) = (p.0 - cell.mx, p.1 - cell.my);
            let d = (dx * dx + dy * dy).sqrt();
            match cell.kids {
                Some(k) if cell.half * 2.0 / d.max(1e-9) > 0.8 => stack.extend(k..k + 4),
                _ => {
                    let other = cell.mass.sqrt();
                    let (fx, fy) = repel(p, (cell.mx, cell.my), r, other);
                    f.0 += fx;
                    f.1 += fy;
                }
            }
        }
        f
    }
}

impl Cell {
    fn new(cx: f64, cy: f64, half: f64) -> Self {
        Self {
            cx,
            cy,
            half,
            mass: 0.0,
            mx: cx,
            my: cy,
            body: None,
            kids: None,
        }
    }
}

/// A live node of the tree being laid out.
struct Live {
    id: i64,
    key: String,
    /// The node one level up; `None` for a service or library.
    parent: Option<i64>,
    first_seq: i64,
}

/// Lays out what changed since the last run and writes the new positions at `seq`. `carried`
/// seeds nodes that have no position yet by node key (a rebuild, IDX-16). Returns the parents laid
/// out again.
pub fn run(
    conn: &Connection,
    seq: i64,
    carried: Option<&HashMap<String, Circle>>,
) -> rusqlite::Result<usize> {
    // Live nodes: services and libraries, modules, and files, each with its key and first step.
    let mut live: Vec<Live> = Vec::new();
    let mut st = conn.prepare(
        "SELECT n.id, n.key, n.parent, (SELECT min(from_seq) FROM node_span s WHERE s.node = n.id)
         FROM node n WHERE n.kind != 'file'
           AND EXISTS (SELECT 1 FROM node_span s WHERE s.node = n.id AND s.to_seq IS NULL)",
    )?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        live.push(Live {
            id: r.get(0)?,
            key: r.get(1)?,
            parent: r.get(2)?,
            first_seq: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
        });
    }
    let mut st = conn.prepare(
        "SELECT n.id, (SELECT path FROM file_path p WHERE p.file = n.file
                       ORDER BY from_seq, path LIMIT 1),
                n.parent, (SELECT min(from_seq) FROM file_path p WHERE p.file = n.file)
         FROM node n JOIN file_path fp ON fp.file = n.file AND fp.to_seq IS NULL
         WHERE n.kind = 'file' AND n.parent IS NOT NULL",
    )?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        live.push(Live {
            id: r.get(0)?,
            key: format!("file:{}", r.get::<_, String>(1)?),
            parent: r.get(2)?,
            first_seq: r.get::<_, Option<i64>>(3)?.unwrap_or(0),
        });
    }
    live.sort_by(|a, b| a.key.cmp(&b.key));
    let index: HashMap<i64, usize> = live.iter().enumerate().map(|(i, l)| (l.id, i)).collect();
    let mut stored: HashMap<i64, Circle> = HashMap::new();
    let mut st = conn.prepare("SELECT node, x, y, r FROM layout WHERE to_seq IS NULL")?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        stored.insert(
            r.get(0)?,
            Circle {
                x: r.get(1)?,
                y: r.get(2)?,
                r: r.get(3)?,
            },
        );
    }
    drop(rows);

    // Children per parent (None: the workspace), and links between siblings.
    let mut children: BTreeMap<Option<i64>, Vec<usize>> = BTreeMap::new();
    for (i, l) in live.iter().enumerate() {
        let p = l.parent.filter(|p| index.contains_key(p));
        children.entry(p).or_default().push(i);
    }
    // The child of `parent` that holds node `i`, if any.
    let ancestor_under = |mut i: usize, parent: Option<i64>| -> Option<usize> {
        for _ in 0..4 {
            let p = live[i].parent.filter(|p| index.contains_key(p));
            if p == parent {
                return Some(i);
            }
            i = index[&p?];
        }
        None
    };
    let mut weights: HashMap<Option<i64>, BTreeMap<(usize, usize), f64>> = HashMap::new();
    let mut st = conn.prepare(
        "SELECT l.src, l.dst FROM interval i JOIN link l ON l.id = i.link WHERE i.to_seq IS NULL",
    )?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        let (Some(&a), Some(&b)) = (index.get(&r.get(0)?), index.get(&r.get(1)?)) else {
            continue;
        };
        // The link counts once, at the deepest parent holding both ends apart.
        let chain = |mut i: usize| {
            let mut v = vec![i];
            while let Some(p) = live[i].parent.and_then(|p| index.get(&p)) {
                i = *p;
                v.push(i);
            }
            v
        };
        let (ca, cb) = (chain(a), chain(b));
        let common = ca.iter().find(|x| cb.contains(x)).map(|x| live[*x].id);
        let (Some(x), Some(y)) = (ancestor_under(a, common), ancestor_under(b, common)) else {
            continue;
        };
        if x != y {
            *weights
                .entry(common)
                .or_default()
                .entry((x.min(y), x.max(y)))
                .or_default() += 1.0;
        }
    }
    drop(rows);

    // Radii bottom-up: a file is 1; a parent fits its children. A stored radius is kept.
    let mut radius: Vec<f64> = vec![1.0; live.len()];
    let mut depth = |i: usize| {
        let mut d = 0;
        let mut j = i;
        while let Some(p) = live[j].parent.and_then(|p| index.get(&p)) {
            j = *p;
            d += 1;
        }
        d
    };
    let mut order: Vec<usize> = (0..live.len()).collect();
    let depths: Vec<usize> = (0..live.len()).map(&mut depth).collect();
    order.sort_by_key(|i| std::cmp::Reverse(depths[*i]));
    for &i in &order {
        if let Some(c) = stored
            .get(&live[i].id)
            .or_else(|| carried.and_then(|c| c.get(&live[i].key)))
        {
            radius[i] = c.r;
        } else if let Some(kids) = children.get(&Some(live[i].id)) {
            radius[i] = radius_for(kids.iter().map(|k| radius[*k]));
        }
    }

    // Top-down, one level at a time; the parents of a level in parallel.
    let mut placed: HashMap<i64, Circle> = HashMap::new();
    let mut moved: Vec<(usize, Circle)> = Vec::new();
    let mut laid = 0;
    let total = radius_for(
        children
            .get(&None)
            .map(|k| k.iter().map(|i| radius[*i]).collect::<Vec<_>>())
            .unwrap_or_default()
            .into_iter(),
    );
    let mut level: Vec<Option<i64>> = vec![None];
    while !level.is_empty() {
        let jobs: Vec<(Option<i64>, String, Group)> = level
            .iter()
            .filter_map(|p| {
                let kids = children.get(p)?;
                let area = match p {
                    None => Circle {
                        x: 0.0,
                        y: 0.0,
                        r: total,
                    },
                    Some(id) => placed.get(id).copied()?,
                };
                let inside = |c: &Circle| {
                    let (dx, dy) = (c.x - area.x, c.y - area.y);
                    (dx * dx + dy * dy).sqrt() + c.r <= area.r + 1e-6
                };
                let pin = |i: usize| {
                    let l = &live[i];
                    stored
                        .get(&l.id)
                        .or_else(|| carried.and_then(|c| c.get(&l.key)))
                        .filter(|c| inside(c))
                        .map(|c| (c.x, c.y))
                };
                let pos: HashMap<usize, usize> =
                    kids.iter().enumerate().map(|(n, i)| (*i, n)).collect();
                let springs = weights
                    .get(p)
                    .map(|w| {
                        w.iter()
                            .filter_map(|((a, b), w)| Some((pos[a], *pos.get(b)?, *w)))
                            .collect()
                    })
                    .unwrap_or_default();
                let g = Group {
                    area,
                    children: kids
                        .iter()
                        .map(|i| (live[*i].key.clone(), radius[*i], pin(*i)))
                        .collect(),
                    springs,
                };
                let key =
                    p.map_or_else(|| "workspace".to_owned(), |id| live[index[&id]].key.clone());
                Some((*p, key, g))
            })
            .collect();
        let results: Vec<Vec<(f64, f64)>> =
            jobs.par_iter().map(|(_, key, g)| place(key, g)).collect();
        let mut next = Vec::new();
        for ((p, key, g), pos) in jobs.iter().zip(results) {
            let k = group_key(g);
            if changed(conn, "layout", key, &k)? {
                record(conn, "layout", key, &k)?;
            }
            if g.children.iter().any(|c| c.2.is_none()) {
                laid += 1;
            }
            for (n, i) in children[p].iter().enumerate() {
                let c = Circle {
                    x: pos[n].0,
                    y: pos[n].1,
                    r: radius[*i],
                };
                if stored.get(&live[*i].id) != Some(&c) {
                    moved.push((*i, c));
                }
                placed.insert(live[*i].id, c);
                next.push(Some(live[*i].id));
            }
        }
        level = next;
    }
    moved.sort_by_key(|(i, _)| *i);
    for (i, c) in moved {
        let l = &live[i];
        let had = stored.contains_key(&l.id);
        if had {
            conn.prepare_cached(
                "UPDATE layout SET to_seq = ?2 WHERE node = ?1 AND to_seq IS NULL",
            )?
            .execute(params![l.id, seq])?;
        }
        let from = if had { seq } else { l.first_seq };
        conn.prepare_cached(
            "INSERT INTO layout (node, x, y, r, from_seq) VALUES (?1, ?2, ?3, ?4, ?5)",
        )?
        .execute(params![l.id, c.x, c.y, c.r, from])?;
    }
    Ok(laid)
}

/// A parent's inputs key: its children (key and radius) and the weights between them.
fn group_key(g: &Group) -> [u8; 32] {
    let mut inputs: Vec<Vec<u8>> = g
        .children
        .iter()
        .map(|(k, r, _)| format!("{k}\0{r}").into_bytes())
        .collect();
    inputs.extend(
        g.springs.iter().map(|(a, b, w)| {
            format!("{}\0{}\0{w}", g.children[*a].0, g.children[*b].0).into_bytes()
        }),
    );
    inputs_key(
        "layout",
        &KeyInputs {
            inputs,
            extractors: vec![("layout", VERSION)],
            ..Default::default()
        },
    )
}

/// Every node's current position by node key, to carry into a rebuild.
pub fn positions_by_key(conn: &Connection) -> rusqlite::Result<HashMap<String, Circle>> {
    let mut out = HashMap::new();
    let mut st = conn.prepare(
        "SELECT coalesce(n.key, 'file:' || (SELECT path FROM file_path p WHERE p.file = n.file
                  ORDER BY from_seq, path LIMIT 1)), l.x, l.y, l.r
         FROM layout l JOIN node n ON n.id = l.node WHERE l.to_seq IS NULL",
    )?;
    let mut rows = st.query([])?;
    while let Some(r) = rows.next()? {
        if let Some(k) = r.get::<_, Option<String>>(0)? {
            out.insert(
                k,
                Circle {
                    x: r.get(1)?,
                    y: r.get(2)?,
                    r: r.get(3)?,
                },
            );
        }
    }
    Ok(out)
}
