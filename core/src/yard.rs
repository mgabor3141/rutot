//! Yard topology and the shunting state machine.
//!
//! Model: a *main* (the headshunt/lead) holding the loco and its string,
//! with stub-ended *sidings* fanning off a ladder at either end. A siding on
//! the `Right` is worked by a loco standing on the `Left` of its string and
//! vice versa. Three primitive moves exist:
//!
//! - `Pull { siding, count }`: back onto `siding`, couple to the outermost
//!   `count` cars, pull them onto the main.
//! - `Push { siding, count }`: shove the far-end `count` cars of the held
//!   string into `siding`, uncouple, return to the main.
//! - `RunAround`: (needs a loop) the loco leaves its string, runs round it,
//!   and couples on the other end. Flips the loco's side; the string is now
//!   ordered the other way relative to the loco.
//!
//! Every siding list is ordered from the ladder end inward. The held string
//! is ordered from the loco outward, so pull/push are side-independent.

use std::fmt;

pub type CarId = u8;
pub type SidingId = usize;

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Side {
    Left,
    Right,
}

impl Side {
    pub fn opposite(self) -> Side {
        match self {
            Side::Left => Side::Right,
            Side::Right => Side::Left,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Siding {
    pub name: String,
    pub capacity: usize,
    /// Which end of the main this siding branches from.
    pub side: Side,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Yard {
    pub name: String,
    pub sidings: Vec<Siding>,
    /// Number of cars the loco can hold on the main (loco excluded).
    pub headshunt: usize,
    /// Is there a loop the loco can use to run round its string?
    pub runaround: bool,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct State {
    /// Per siding, cars from the ladder end inward.
    pub sidings: Vec<Vec<CarId>>,
    /// Cars coupled to the loco, loco-adjacent first.
    pub held: Vec<CarId>,
    /// Which end of its string the loco is on.
    pub loco: Side,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Move {
    Pull { siding: SidingId, count: usize },
    Push { siding: SidingId, count: usize },
    RunAround,
}

impl Move {
    pub fn siding(&self) -> Option<SidingId> {
        match *self {
            Move::Pull { siding, .. } | Move::Push { siding, .. } => Some(siding),
            Move::RunAround => None,
        }
    }
    pub fn count(&self) -> usize {
        match *self {
            Move::Pull { count, .. } | Move::Push { count, .. } => count,
            Move::RunAround => 0,
        }
    }
    /// Abstract cost in *legs* (one loco journey). Pull/push are out and
    /// back; a run-round is round the loop, back to couple, and draw up.
    pub fn cost(&self) -> u32 {
        match *self {
            Move::Pull { .. } | Move::Push { .. } => 2,
            Move::RunAround => 3,
        }
    }
}

pub fn plan_cost(moves: &[Move]) -> u32 {
    moves.iter().map(Move::cost).sum()
}

/// The product the yard must assemble: `order` must be the ladder-end prefix
/// of `siding`, so a road loco can couple on and depart.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Goal {
    pub siding: SidingId,
    pub order: Vec<CarId>,
}

impl Goal {
    pub fn satisfied(&self, s: &State) -> bool {
        s.sidings[self.siding].starts_with(&self.order)
    }
}

impl Yard {
    /// The classic Inglenook Sidings puzzle: 8 cars, sidings of 5/3/3, a
    /// headshunt for loco + 3. Departure track is siding 0.
    pub fn inglenook() -> Self {
        let r = |name: &str, capacity| Siding { name: name.into(), capacity, side: Side::Right };
        Yard {
            name: "Inglenook 5-3-3 / lead 3".into(),
            sidings: vec![r("Main", 5), r("Upper", 3), r("Lower", 3)],
            headshunt: 3,
            runaround: false,
        }
    }

    /// Inglenook plus a loop. Prediction: the solver never uses it, because
    /// with every siding on one side the loco can't work anything from the
    /// far end of its string.
    pub fn inglenook_with_loop() -> Self {
        let mut y = Self::inglenook();
        y.name = "Inglenook 5-3-3 / lead 3 + loop".into();
        y.runaround = true;
        y
    }

    /// Timesaver-style: sidings facing both ways, so some cars are on the
    /// wrong end of the loco and the loop is the only way round.
    pub fn timesaver() -> Self {
        let r = |name: &str, capacity| Siding { name: name.into(), capacity, side: Side::Right };
        let l = |name: &str, capacity| Siding { name: name.into(), capacity, side: Side::Left };
        Yard {
            name: "Timesaver 5-2 | 3-2 / main 3 + loop".into(),
            sidings: vec![r("Main", 5), r("Spur", 2), l("Long", 3), l("Short", 2)],
            headshunt: 3,
            runaround: true,
        }
    }

    /// Same track as `timesaver` but no loop: are the left sidings just
    /// dead storage now?
    pub fn timesaver_no_loop() -> Self {
        let mut y = Self::timesaver();
        y.name = "Timesaver 5-2 | 3-2 / main 3, no loop".into();
        y.runaround = false;
        y
    }

    /// Same sidings, longer lead. A pure topology change: does it pay off?
    pub fn inglenook_long_lead() -> Self {
        let mut y = Self::inglenook();
        y.name = "Inglenook 5-3-3 / lead 4".into();
        y.headshunt = 4;
        y
    }

    /// One more short siding. Costs land, saves moves?
    pub fn inglenook_four() -> Self {
        let mut y = Self::inglenook();
        y.name = "Inglenook 5-3-3-2 / lead 3".into();
        y.sidings.push(Siding { name: "Spur".into(), capacity: 2, side: Side::Right });
        y
    }

    pub fn total_capacity(&self) -> usize {
        self.sidings.iter().map(|s| s.capacity).sum::<usize>() + self.headshunt
    }

    pub fn empty_state(&self) -> State {
        State {
            sidings: vec![Vec::new(); self.sidings.len()],
            held: Vec::new(),
            loco: Side::Left,
        }
    }

    pub fn has_side(&self, side: Side) -> bool {
        self.sidings.iter().any(|s| s.side == side)
    }

    /// Can a loco on `loco` side work siding `i`? It must stand on the
    /// opposite end of its string from the siding.
    pub fn can_work(&self, loco: Side, i: SidingId) -> bool {
        self.sidings[i].side != loco
    }

    pub fn legal_moves(&self, s: &State) -> Vec<Move> {
        let mut out = Vec::new();
        let room = self.headshunt.saturating_sub(s.held.len());
        for (i, sd) in s.sidings.iter().enumerate() {
            if !self.can_work(s.loco, i) {
                continue;
            }
            let cap = self.sidings[i].capacity;
            for count in 1..=sd.len().min(room) {
                out.push(Move::Pull { siding: i, count });
            }
            let free = cap.saturating_sub(sd.len());
            for count in 1..=s.held.len().min(free) {
                out.push(Move::Push { siding: i, count });
            }
        }
        if self.runaround {
            out.push(Move::RunAround);
        }
        out
    }

    /// Apply a move, returning the new state, or `None` if illegal.
    pub fn apply(&self, s: &State, m: Move) -> Option<State> {
        let mut n = s.clone();
        match m {
            Move::RunAround => {
                if !self.runaround {
                    return None;
                }
                n.held.reverse();
                n.loco = n.loco.opposite();
            }
            Move::Pull { siding, count } => {
                if !self.can_work(s.loco, siding) {
                    return None;
                }
                let sd = n.sidings.get_mut(siding)?;
                if count == 0 || count > sd.len() || n.held.len() + count > self.headshunt {
                    return None;
                }
                // Loco backs on; its far end couples to the ladder-end car.
                n.held.extend(sd.drain(..count));
            }
            Move::Push { siding, count } => {
                if !self.can_work(s.loco, siding) {
                    return None;
                }
                let cap = self.sidings.get(siding)?.capacity;
                let sd = &mut n.sidings[siding];
                if count == 0 || count > n.held.len() || sd.len() + count > cap {
                    return None;
                }
                // Far-end cars enter first and go deepest; the car nearest
                // the loco among those pushed ends up nearest the ladder.
                let at = n.held.len() - count;
                let pushed: Vec<CarId> = n.held.drain(at..).collect();
                sd.splice(0..0, pushed);
            }
        }
        Some(n)
    }

    pub fn validate(&self, s: &State) -> Result<(), String> {
        if s.sidings.len() != self.sidings.len() {
            return Err("siding count mismatch".into());
        }
        for (i, sd) in s.sidings.iter().enumerate() {
            if sd.len() > self.sidings[i].capacity {
                return Err(format!("siding {i} over capacity"));
            }
        }
        if s.held.len() > self.headshunt {
            return Err("headshunt over capacity".into());
        }
        let mut seen = std::collections::HashSet::new();
        for c in s.sidings.iter().flatten().chain(s.held.iter()) {
            if !seen.insert(*c) {
                return Err(format!("car {c} appears twice"));
            }
        }
        Ok(())
    }
}

pub fn car_label(c: CarId) -> char {
    (b'A' + c) as char
}

impl fmt::Display for Move {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Move::Pull { siding, count } => write!(f, "pull {count} from #{siding}"),
            Move::Push { siding, count } => write!(f, "push {count} onto #{siding}"),
            Move::RunAround => write!(f, "run round"),
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let show = |v: &[CarId]| v.iter().map(|&c| car_label(c)).collect::<String>();
        match self.loco {
            Side::Left => write!(f, "loco>[{}]", show(&self.held))?,
            Side::Right => write!(f, "[{}]<loco", show(self.held.iter().rev().copied().collect::<Vec<_>>().as_slice()))?,
        }
        for (i, sd) in self.sidings.iter().enumerate() {
            write!(f, " #{i}[{}]", show(sd))?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn st(sidings: &[&[CarId]], held: &[CarId]) -> State {
        State {
            sidings: sidings.iter().map(|s| s.to_vec()).collect(),
            held: held.to_vec(),
            loco: Side::Left,
        }
    }

    #[test]
    fn run_around_reverses_string_and_flips_side() {
        let y = Yard::timesaver();
        let s = State { sidings: vec![vec![], vec![], vec![], vec![]], held: vec![1, 2, 3], loco: Side::Left };
        let n = y.apply(&s, Move::RunAround).unwrap();
        assert_eq!(n.held, vec![3, 2, 1]);
        assert_eq!(n.loco, Side::Right);
        assert_eq!(y.apply(&n, Move::RunAround).unwrap(), s);
        assert!(Yard::inglenook().apply(&s, Move::RunAround).is_none());
    }

    #[test]
    fn sidings_are_gated_by_side() {
        let y = Yard::timesaver();
        let s = State { sidings: vec![vec![0], vec![], vec![1], vec![]], held: vec![], loco: Side::Left };
        assert!(y.apply(&s, Move::Pull { siding: 0, count: 1 }).is_some(), "right siding from left");
        assert!(y.apply(&s, Move::Pull { siding: 2, count: 1 }).is_none(), "left siding from left");
        let r = y.apply(&s, Move::RunAround).unwrap();
        assert!(r.loco == Side::Right);
        assert!(y.apply(&r, Move::Pull { siding: 2, count: 1 }).is_some(), "left siding from right");
        assert!(y.apply(&r, Move::Pull { siding: 0, count: 1 }).is_none());
    }

    #[test]
    fn pull_push_from_left_is_identity_too() {
        let y = Yard::timesaver();
        let s = State { sidings: vec![vec![], vec![], vec![0, 1, 2], vec![]], held: vec![5], loco: Side::Right };
        let a = y.apply(&s, Move::Pull { siding: 2, count: 2 }).unwrap();
        assert_eq!(a.held, vec![5, 0, 1]);
        let b = y.apply(&a, Move::Push { siding: 2, count: 2 }).unwrap();
        assert_eq!(b, s);
    }

    #[test]
    fn pull_appends_ladder_end_cars_to_far_end_of_string() {
        let y = Yard::inglenook();
        let s = st(&[&[0, 1, 2], &[], &[]], &[7]);
        let n = y.apply(&s, Move::Pull { siding: 0, count: 2 }).unwrap();
        assert_eq!(n.held, vec![7, 0, 1]);
        assert_eq!(n.sidings[0], vec![2]);
    }

    #[test]
    fn push_puts_far_end_cars_deepest() {
        let y = Yard::inglenook();
        let s = st(&[&[], &[5], &[]], &[7, 0, 1]);
        let n = y.apply(&s, Move::Push { siding: 1, count: 2 }).unwrap();
        // 1 was farthest from loco -> deepest (but behind existing 5).
        assert_eq!(n.sidings[1], vec![0, 1, 5]);
        assert_eq!(n.held, vec![7]);
    }

    #[test]
    fn pull_then_push_is_identity() {
        let y = Yard::inglenook();
        let s = st(&[&[0, 1, 2], &[3], &[4, 5]], &[]);
        for m in y.legal_moves(&s) {
            if let Move::Pull { siding, count } = m {
                let a = y.apply(&s, m).unwrap();
                let b = y.apply(&a, Move::Push { siding, count }).unwrap();
                assert_eq!(b, s, "{m} then push back should restore");
            }
        }
    }

    #[test]
    fn capacity_is_enforced() {
        let y = Yard::inglenook();
        let s = st(&[&[], &[0, 1, 2], &[]], &[3, 4, 5]);
        assert!(y.apply(&s, Move::Pull { siding: 1, count: 1 }).is_none());
        assert!(y.apply(&s, Move::Push { siding: 1, count: 1 }).is_none());
        assert!(y.apply(&s, Move::Push { siding: 0, count: 3 }).is_some());
    }

    #[test]
    fn goal_is_prefix() {
        let g = Goal { siding: 0, order: vec![2, 0] };
        assert!(g.satisfied(&st(&[&[2, 0, 5], &[], &[]], &[])));
        assert!(!g.satisfied(&st(&[&[5, 2, 0], &[], &[]], &[])));
    }
}
