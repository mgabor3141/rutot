//! Yard topology and the shunting state machine.
//!
//! Model: a set of stub-ended *sidings* fanning off a ladder, plus a
//! *headshunt* (lead) where the locomotive works. The loco is always on the
//! headshunt between moves. Two primitive moves exist:
//!
//! - `Pull { siding, count }`: back onto `siding`, couple to the outermost
//!   `count` cars, pull them onto the headshunt.
//! - `Push { siding, count }`: shove the far-end `count` cars of the held
//!   string into `siding`, uncouple, return to the headshunt.
//!
//! Every list is ordered from the ladder end inward. The held string is
//! ordered from the loco outward.

use std::fmt;

pub type CarId = u8;
pub type SidingId = usize;

#[derive(Clone, Debug, PartialEq)]
pub struct Siding {
    pub name: String,
    pub capacity: usize,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Yard {
    pub name: String,
    pub sidings: Vec<Siding>,
    /// Number of cars the loco can hold on the headshunt (loco excluded).
    pub headshunt: usize,
}

#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct State {
    /// Per siding, cars from the ladder end inward.
    pub sidings: Vec<Vec<CarId>>,
    /// Cars coupled to the loco, loco-adjacent first.
    pub held: Vec<CarId>,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub enum Move {
    Pull { siding: SidingId, count: usize },
    Push { siding: SidingId, count: usize },
}

impl Move {
    pub fn siding(&self) -> SidingId {
        match *self {
            Move::Pull { siding, .. } | Move::Push { siding, .. } => siding,
        }
    }
    pub fn count(&self) -> usize {
        match *self {
            Move::Pull { count, .. } | Move::Push { count, .. } => count,
        }
    }
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
        Yard {
            name: "Inglenook 5-3-3 / lead 3".into(),
            sidings: vec![
                Siding { name: "Main".into(), capacity: 5 },
                Siding { name: "Upper".into(), capacity: 3 },
                Siding { name: "Lower".into(), capacity: 3 },
            ],
            headshunt: 3,
        }
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
        y.sidings.push(Siding { name: "Spur".into(), capacity: 2 });
        y
    }

    pub fn total_capacity(&self) -> usize {
        self.sidings.iter().map(|s| s.capacity).sum::<usize>() + self.headshunt
    }

    pub fn empty_state(&self) -> State {
        State {
            sidings: vec![Vec::new(); self.sidings.len()],
            held: Vec::new(),
        }
    }

    pub fn legal_moves(&self, s: &State) -> Vec<Move> {
        let mut out = Vec::new();
        let room = self.headshunt.saturating_sub(s.held.len());
        for (i, sd) in s.sidings.iter().enumerate() {
            let cap = self.sidings[i].capacity;
            for count in 1..=sd.len().min(room) {
                out.push(Move::Pull { siding: i, count });
            }
            let free = cap.saturating_sub(sd.len());
            for count in 1..=s.held.len().min(free) {
                out.push(Move::Push { siding: i, count });
            }
        }
        out
    }

    /// Apply a move, returning the new state, or `None` if illegal.
    pub fn apply(&self, s: &State, m: Move) -> Option<State> {
        let mut n = s.clone();
        match m {
            Move::Pull { siding, count } => {
                let sd = n.sidings.get_mut(siding)?;
                if count == 0 || count > sd.len() || n.held.len() + count > self.headshunt {
                    return None;
                }
                // Loco backs on; its far end couples to the ladder-end car.
                n.held.extend(sd.drain(..count));
            }
            Move::Push { siding, count } => {
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
        }
    }
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let show = |v: &[CarId]| v.iter().map(|&c| car_label(c)).collect::<String>();
        write!(f, "loco[{}]", show(&self.held))?;
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
        }
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
