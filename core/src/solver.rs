//! A* over yard states. Deterministic, dependency-free, allocation-free in the
//! inner loop, and fast enough to run on demand for yards of Inglenook size.

use crate::yard::{plan_cost, Goal, Move, Side, State, Yard};
use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap};
use std::hash::{BuildHasherDefault, Hasher};
use std::time::{Duration, Instant};

/// FxHash-style multiply-rotate hasher. `Packed` is ~40 bytes of dense
/// state; SipHash spends most of the solver's time on it otherwise.
#[derive(Default)]
struct FxHasher(u64);

impl FxHasher {
    #[inline]
    fn add(&mut self, w: u64) {
        self.0 = (self.0.rotate_left(5) ^ w).wrapping_mul(0x517c_c1b7_2722_0a95);
    }
}

impl Hasher for FxHasher {
    #[inline]
    fn write(&mut self, bytes: &[u8]) {
        let (chunks, rem) = bytes.as_chunks::<8>();
        for c in chunks {
            self.add(u64::from_le_bytes(*c));
        }
        if !rem.is_empty() {
            let mut b = [0u8; 8];
            b[..rem.len()].copy_from_slice(rem);
            self.add(u64::from_le_bytes(b));
        }
    }
    #[inline]
    fn write_u8(&mut self, i: u8) {
        self.add(i as u64);
    }
    #[inline]
    fn write_usize(&mut self, i: usize) {
        self.add(i as u64);
    }
    #[inline]
    fn finish(&self) -> u64 {
        self.0
    }
}

type FxMap<K, V> = HashMap<K, V, BuildHasherDefault<FxHasher>>;

#[derive(Clone, Debug)]
pub struct Plan {
    pub moves: Vec<Move>,
    /// Sum of `Move::cost` in legs.
    pub cost: u32,
    pub expanded: usize,
    pub elapsed: Duration,
}

impl Plan {
    fn new(moves: Vec<Move>, expanded: usize, elapsed: Duration) -> Plan {
        Plan { cost: plan_cost(&moves), moves, expanded, elapsed }
    }
    pub fn run_arounds(&self) -> usize {
        self.moves.iter().filter(|m| **m == Move::RunAround).count()
    }
}

#[derive(Clone, Debug)]
pub enum SolveError {
    Unsolvable { expanded: usize },
    Budget { expanded: usize },
}

const MAX_CELLS: usize = 32;
const MAX_LISTS: usize = 8;
const EMPTY: u8 = 0xFF;

/// Fixed-size, `Copy` encoding of a `State` against a given yard: each list
/// (sidings, then held) owns a contiguous cell range of its capacity, filled
/// from the ladder end. Unused cells are `EMPTY` so equal states hash equal.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
struct Packed {
    cells: [u8; MAX_CELLS],
    lens: [u8; MAX_LISTS],
    /// 0 = loco on the left of its string, 1 = right.
    loco: u8,
}

struct Shape {
    /// Cell offset per list (sidings..., held).
    off: [usize; MAX_LISTS],
    cap: [usize; MAX_LISTS],
    /// 0 = Left, 1 = Right, per siding.
    side: [u8; MAX_LISTS],
    n_sidings: usize,
    runaround: bool,
}

fn side_bit(s: Side) -> u8 {
    match s {
        Side::Left => 0,
        Side::Right => 1,
    }
}

impl Shape {
    fn of(yard: &Yard) -> Shape {
        let n = yard.sidings.len();
        assert!(n < MAX_LISTS, "too many sidings for packed state");
        let mut off = [0; MAX_LISTS];
        let mut cap = [0; MAX_LISTS];
        let mut side = [0; MAX_LISTS];
        let mut acc = 0;
        for (i, s) in yard.sidings.iter().enumerate() {
            off[i] = acc;
            cap[i] = s.capacity;
            side[i] = side_bit(s.side);
            acc += s.capacity;
        }
        off[n] = acc;
        cap[n] = yard.headshunt;
        acc += yard.headshunt;
        assert!(acc <= MAX_CELLS, "yard too large for packed state");
        Shape { off, cap, side, n_sidings: n, runaround: yard.runaround }
    }
    fn held(&self) -> usize {
        self.n_sidings
    }
    fn pack(&self, s: &State) -> Packed {
        let mut p = Packed { cells: [EMPTY; MAX_CELLS], lens: [0; MAX_LISTS], loco: side_bit(s.loco) };
        for (i, sd) in s.sidings.iter().enumerate() {
            p.lens[i] = sd.len() as u8;
            p.cells[self.off[i]..self.off[i] + sd.len()].copy_from_slice(sd);
        }
        let h = self.held();
        p.lens[h] = s.held.len() as u8;
        p.cells[self.off[h]..self.off[h] + s.held.len()].copy_from_slice(&s.held);
        p
    }
    fn list<'a>(&self, p: &'a Packed, i: usize) -> &'a [u8] {
        &p.cells[self.off[i]..self.off[i] + p.lens[i] as usize]
    }

    fn apply(&self, p: &Packed, m: Move) -> Packed {
        let mut n = *p;
        let h = self.held();
        let ho = self.off[h];
        let hl = p.lens[h] as usize;
        match m {
            Move::RunAround => {
                n.cells[ho..ho + hl].reverse();
                n.loco ^= 1;
            }
            Move::Pull { siding, count } => {
                let so = self.off[siding];
                let sl = p.lens[siding] as usize;
                // Append the ladder-end `count` cars to the far end of the string.
                n.cells[ho + hl..ho + hl + count].copy_from_slice(&p.cells[so..so + count]);
                // Shift the siding down.
                n.cells[so..so + sl - count].copy_from_slice(&p.cells[so + count..so + sl]);
                n.cells[so + sl - count..so + sl].fill(EMPTY);
                n.lens[h] += count as u8;
                n.lens[siding] -= count as u8;
            }
            Move::Push { siding, count } => {
                let so = self.off[siding];
                let sl = p.lens[siding] as usize;
                // Existing siding cars move deeper.
                n.cells[so + count..so + count + sl].copy_from_slice(&p.cells[so..so + sl]);
                // Far-end `count` cars land at the ladder end, order preserved.
                n.cells[so..so + count].copy_from_slice(&p.cells[ho + hl - count..ho + hl]);
                n.cells[ho + hl - count..ho + hl].fill(EMPTY);
                n.lens[h] -= count as u8;
                n.lens[siding] += count as u8;
            }
        }
        n
    }

    fn for_each_move(&self, p: &Packed, mut f: impl FnMut(Move)) {
        let h = self.held();
        let hl = p.lens[h] as usize;
        let room = self.cap[h] - hl;
        for i in 0..self.n_sidings {
            if self.side[i] == p.loco {
                continue; // wrong end of the string for this siding
            }
            let sl = p.lens[i] as usize;
            for count in 1..=sl.min(room) {
                f(Move::Pull { siding: i, count });
            }
            let free = self.cap[i] - sl;
            for count in 1..=hl.min(free) {
                f(Move::Push { siding: i, count });
            }
        }
        if self.runaround {
            f(Move::RunAround);
        }
    }

    fn satisfied(&self, p: &Packed, goal: &Goal) -> bool {
        self.list(p, goal.siding).starts_with(&goal.order)
    }

    /// Admissible lower bound on remaining cost (legs).
    ///
    /// On the goal siding, the deepest part that can stay is a suffix equal to
    /// `(suffix of order) ++ (cars not in order)`; everything ladder-side of it
    /// must be pulled. Goal cars not in that kept suffix must still be pushed
    /// on. Each pull touches one siding; each push carries ≤ headshunt cars.
    /// Run-rounds move no cars between lists, so they never lower this bound.
    fn heuristic(&self, p: &Packed, goal: &Goal) -> u32 {
        let h_cap = self.cap[self.held()];
        let cur = self.list(p, goal.siding);
        let in_order = |c: u8| goal.order.contains(&c);

        // Smallest k such that cur[k..] is a valid kept suffix.
        let mut kept_goal = 0usize;
        let mut k = cur.len();
        for cand in 0..=cur.len() {
            let rem = &cur[cand..];
            let j = rem.iter().take_while(|&&c| in_order(c)).count();
            let tail_ok = rem[j..].iter().all(|&c| !in_order(c));
            let order_ok = j <= goal.order.len() && rem[..j] == goal.order[goal.order.len() - j..];
            if tail_ok && order_ok {
                kept_goal = j;
                k = cand;
                break;
            }
        }
        let missing = goal.order.len() - kept_goal;
        let missing_set = &goal.order[..missing];

        let mut pulls = k.div_ceil(h_cap) as u32;
        // Which sides the loco must stand on: `need[side]`.
        let mut need = [false; 2];
        let goal_work = self.side[goal.siding] ^ 1;
        if missing > 0 || k > 0 {
            need[goal_work as usize] = true;
        }
        for i in 0..self.n_sidings {
            if i == goal.siding {
                continue;
            }
            if self.list(p, i).iter().any(|c| missing_set.contains(c)) {
                pulls += 1;
                need[(self.side[i] ^ 1) as usize] = true;
            }
        }
        let pushes = missing.div_ceil(h_cap) as u32;

        // The last move is always at `goal_work`. Standing there and needing
        // the other side means out and back (2); standing on the other side
        // and needing `goal_work` means at least one flip.
        let other = (goal_work ^ 1) as usize;
        let rounds = if p.loco == goal_work {
            if need[other] { 2 } else { 0 }
        } else if need[goal_work as usize] {
            1
        } else {
            0
        };
        2 * (pulls + pushes) + 3 * rounds
    }
}

pub fn solve(yard: &Yard, start: &State, goal: &Goal, max_states: usize) -> Result<Plan, SolveError> {
    let t0 = Instant::now();
    let shape = Shape::of(yard);
    let s0 = shape.pack(start);
    if shape.satisfied(&s0, goal) {
        return Ok(Plan::new(vec![], 0, t0.elapsed()));
    }

    // Arena: state, best g, parent index, move from parent.
    let mut states: Vec<Packed> = vec![s0];
    let mut g_best: Vec<u32> = vec![0];
    let mut parent: Vec<(usize, Move)> = vec![(0, Move::Pull { siding: 0, count: 0 })];
    let mut index: FxMap<Packed, usize> = FxMap::with_capacity_and_hasher(1 << 14, Default::default());
    index.insert(s0, 0);
    // Min-heap on f, then prefer deeper g (ties broken toward the goal).
    let mut open: BinaryHeap<(Reverse<u32>, u32, usize)> = BinaryHeap::new();
    open.push((Reverse(shape.heuristic(&s0, goal)), 0, 0));

    let finish = |idx: usize, states: &Vec<Packed>, parent: &Vec<(usize, Move)>| {
        let mut moves = Vec::new();
        let mut at = idx;
        while at != 0 {
            let (p, mv) = parent[at];
            moves.push(mv);
            at = p;
        }
        moves.reverse();
        Plan::new(moves, states.len(), t0.elapsed())
    };

    while let Some((Reverse(f), g, idx)) = open.pop() {
        if g > g_best[idx] {
            continue; // stale entry
        }
        let cur = states[idx];
        if shape.satisfied(&cur, goal) {
            return Ok(finish(idx, &states, &parent));
        }
        if states.len() >= max_states {
            return Err(SolveError::Budget { expanded: states.len() });
        }
        let _ = f;
        shape.for_each_move(&cur, |m| {
            let ng = g + m.cost();
            let next = shape.apply(&cur, m);
            match index.get(&next) {
                Some(&ni) if g_best[ni] <= ng => {}
                Some(&ni) => {
                    g_best[ni] = ng;
                    parent[ni] = (idx, m);
                    open.push((Reverse(ng + shape.heuristic(&next, goal)), ng, ni));
                }
                None => {
                    let ni = states.len();
                    states.push(next);
                    g_best.push(ng);
                    parent.push((idx, m));
                    index.insert(next, ni);
                    open.push((Reverse(ng + shape.heuristic(&next, goal)), ng, ni));
                }
            }
        });
    }
    Err(SolveError::Unsolvable { expanded: states.len() })
}

/// Tiny xorshift so the core stays dependency-free and runs are reproducible.
#[derive(Clone, Debug)]
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }
    pub fn shuffle<T>(&mut self, v: &mut [T]) {
        for i in (1..v.len()).rev() {
            let j = self.below(i + 1);
            v.swap(i, j);
        }
    }
}

/// A random task: `n_cars` scattered over the sidings (loco light), and a
/// goal of `goal_len` distinct cars in random order on `goal_siding`.
pub fn random_task(yard: &Yard, n_cars: usize, goal_len: usize, goal_siding: usize, rng: &mut Rng) -> (State, Goal) {
    assert!(n_cars <= yard.sidings.iter().map(|s| s.capacity).sum::<usize>());
    assert!(goal_len <= n_cars && goal_len <= yard.sidings[goal_siding].capacity);
    let mut cars: Vec<u8> = (0..n_cars as u8).collect();
    rng.shuffle(&mut cars);
    let mut state = yard.empty_state();
    for c in &cars {
        loop {
            let i = rng.below(yard.sidings.len());
            if state.sidings[i].len() < yard.sidings[i].capacity {
                state.sidings[i].push(*c);
                break;
            }
        }
    }
    let mut order: Vec<u8> = (0..n_cars as u8).collect();
    rng.shuffle(&mut order);
    order.truncate(goal_len);
    (state, Goal { siding: goal_siding, order })
}

/// Aggregate plan-length statistics over many random tasks: the yard's
/// "recipe time".
#[derive(Clone, Debug, Default)]
pub struct YardStats {
    pub tasks: usize,
    pub solved: usize,
    pub total_moves: usize,
    pub total_cost: u64,
    pub total_run_arounds: usize,
    pub max_moves: usize,
    pub total_expanded: usize,
    /// Tasks abandoned at the state budget (vs. proven unsolvable).
    pub budget_hits: usize,
    pub elapsed: Duration,
}

impl YardStats {
    pub fn mean_moves(&self) -> f64 {
        if self.solved == 0 { 0.0 } else { self.total_moves as f64 / self.solved as f64 }
    }
    pub fn mean_cost(&self) -> f64 {
        if self.solved == 0 { 0.0 } else { self.total_cost as f64 / self.solved as f64 }
    }
    pub fn mean_run_arounds(&self) -> f64 {
        if self.solved == 0 { 0.0 } else { self.total_run_arounds as f64 / self.solved as f64 }
    }
}

pub fn benchmark(yard: &Yard, n_cars: usize, goal_len: usize, goal_siding: usize, tasks: usize, seed: u64) -> YardStats {
    let mut rng = Rng::new(seed);
    let mut st = YardStats { tasks, ..Default::default() };
    let t0 = Instant::now();
    for _ in 0..tasks {
        let (s, g) = random_task(yard, n_cars, goal_len, goal_siding, &mut rng);
        match solve(yard, &s, &g, 2_000_000) {
            Ok(p) => {
                st.solved += 1;
                st.total_moves += p.moves.len();
                st.total_cost += p.cost as u64;
                st.total_run_arounds += p.run_arounds();
                st.max_moves = st.max_moves.max(p.moves.len());
                st.total_expanded += p.expanded;
            }
            Err(SolveError::Unsolvable { expanded }) => {
                st.total_expanded += expanded;
            }
            Err(SolveError::Budget { expanded }) => {
                st.total_expanded += expanded;
                st.budget_hits += 1;
            }
        }
    }
    st.elapsed = t0.elapsed();
    st
}

#[cfg(test)]
mod tests {
    use super::*;

    fn replay(y: &Yard, s: &State, p: &Plan) -> State {
        let mut cur = s.clone();
        for m in &p.moves {
            cur = y.apply(&cur, *m).expect("legal");
        }
        cur
    }

    #[test]
    fn packed_apply_matches_reference_apply() {
        let y = Yard::inglenook();
        let shape = Shape::of(&y);
        let mut rng = Rng::new(7);
        for _ in 0..200 {
            let (mut s, _) = random_task(&y, 8, 5, 0, &mut rng);
            for _ in 0..12 {
                let moves = y.legal_moves(&s);
                if moves.is_empty() {
                    break;
                }
                let m = moves[rng.below(moves.len())];
                let ref_next = y.apply(&s, m).unwrap();
                let packed_next = shape.apply(&shape.pack(&s), m);
                assert_eq!(packed_next, shape.pack(&ref_next), "{s} --{m}-->");
                s = ref_next;
            }
        }
    }

    #[test]
    fn solves_a_known_inglenook_task() {
        let y = Yard::inglenook();
        let s = State {
            sidings: vec![vec![0, 1, 2, 3, 4], vec![5, 6, 7], vec![]],
            held: vec![],
            loco: Side::Left,
        };
        let g = Goal { siding: 0, order: vec![7, 3, 0, 5, 1] };
        let p = solve(&y, &s, &g, 5_000_000).expect("solvable");
        assert!(g.satisfied(&replay(&y, &s, &p)));
        assert_eq!(p.moves.len(), 13, "BFS reference found 13");
        eprintln!("{} moves, {} states, {:?}", p.moves.len(), p.expanded, p.elapsed);
    }

    #[test]
    fn trivial_tasks() {
        let y = Yard::inglenook();
        let s = State { sidings: vec![vec![], vec![0], vec![]], held: vec![], loco: Side::Left };
        let g = Goal { siding: 1, order: vec![0] };
        assert_eq!(solve(&y, &s, &g, 1000).unwrap().moves.len(), 0);
        let g = Goal { siding: 0, order: vec![0] };
        assert_eq!(solve(&y, &s, &g, 1000).unwrap().moves.len(), 2);
    }

    #[test]
    fn loop_is_never_used_in_a_one_sided_yard() {
        let y = Yard::inglenook_with_loop();
        let st = benchmark(&y, 8, 5, 0, 30, 9);
        assert_eq!(st.solved, 30);
        assert_eq!(st.total_run_arounds, 0, "run-rounds can't help when all sidings face one way");
        let plain = benchmark(&Yard::inglenook(), 8, 5, 0, 30, 9);
        assert_eq!(st.total_cost, plain.total_cost);
    }

    #[test]
    fn timesaver_needs_the_loop() {
        // A goal car sits in a left-hand siding; only a run-round reaches it.
        let y = Yard::timesaver();
        let s = State {
            sidings: vec![vec![0, 1], vec![], vec![2], vec![]],
            held: vec![],
            loco: Side::Left,
        };
        let g = Goal { siding: 0, order: vec![2, 0] };
        let p = solve(&y, &s, &g, 2_000_000).expect("solvable with loop");
        assert!(p.run_arounds() >= 1, "{:?}", p.moves);
        assert!(g.satisfied(&replay(&y, &s, &p)));
        let ny = Yard::timesaver_no_loop();
        assert!(matches!(solve(&ny, &s, &g, 2_000_000), Err(SolveError::Unsolvable { .. })));
    }

    #[test]
    fn benchmark_runs_fast() {
        for y in [Yard::inglenook(), Yard::inglenook_long_lead(), Yard::inglenook_four(), Yard::timesaver()] {
            let st = benchmark(&y, 8, 5, 0, 50, 42);
            eprintln!(
                "{:<36} {}/{} solved ({} budget), mean {:.2} moves / {:.2} legs, {:.2} run-rounds, max {}, {} states, {:?}",
                y.name, st.solved, st.tasks, st.budget_hits, st.mean_moves(), st.mean_cost(), st.mean_run_arounds(), st.max_moves, st.total_expanded, st.elapsed
            );
            assert!(st.solved >= st.tasks * 9 / 10, "{}: {}/{}", y.name, st.solved, st.tasks);
        }
    }
}
