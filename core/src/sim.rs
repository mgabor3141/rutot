//! Fixed-tick execution of a plan. Pull/push run as two legs (out to the
//! siding, back to rest) with a dwell for coupling. A run-round is three:
//! the loco leaves its string and goes over the loop, backs along the main
//! to couple on the other end, then draws the string up to rest on its new
//! side. Pure logic; the renderer interpolates between ticks.

use crate::layout::{Layout, Polyline, P2};
use crate::yard::{Goal, Move, State, Yard};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pose {
    pub pos: P2,
    pub heading: f32,
}

#[derive(Clone, Debug)]
pub struct SimSnapshot {
    pub loco: Pose,
    /// One pose per car id (index = CarId).
    pub cars: Vec<Pose>,
}

/// Which polyline the string (or the light loco) is currently measured on.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Track {
    /// The main, oriented away from the loco's side.
    Main,
    /// A siding path (main + siding), oriented away from the loco's side.
    Siding(usize),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    Idle,
    /// String travelling along `Track` toward `target`.
    Travel { target: f32, then: Then },
    /// Loco alone on the loop polyline; cars parked on the main.
    Loop { s: f32, v: f32 },
    /// Loco alone backing along the main (from the far throat) to couple.
    Approach { s: f32, v: f32, target: f32 },
    Dwell { ticks: u32, then: Then },
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Then {
    /// Apply the current pull/push to the state, then head home.
    Exchange,
    /// Loco has coupled on the far end: flip sides, then draw up.
    Recouple,
    /// Arrived home; advance to the next move.
    Home,
}

#[derive(Clone, Debug)]
pub struct Sim {
    pub yard: Yard,
    pub layout: Layout,
    pub state: State,
    pub goal: Goal,
    pub plan: Vec<Move>,
    pub step: usize,
    /// String position (loco centre) along the active track.
    pub s: f32,
    pub v: f32,
    pub v_max: f32,
    pub accel: f32,
    pub dwell_ticks: u32,
    pub ticks: u64,
    track: Track,
    phase: Phase,
}

/// One step of a trapezoidal speed profile toward `target`. Returns the new
/// `(s, v, arrived)`.
fn advance(s: f32, v: f32, target: f32, v_max: f32, accel: f32) -> (f32, f32, bool) {
    let remaining = target - s;
    let dist = remaining.abs();
    let v_brake = (2.0 * accel * dist).sqrt();
    let v = (v + accel).min(v_max).min(v_brake).max(0.15);
    if dist <= v {
        (target, 0.0, true)
    } else {
        (s + v * remaining.signum(), v, false)
    }
}

impl Sim {
    pub fn new(yard: Yard, layout: Layout, state: State, goal: Goal, plan: Vec<Move>) -> Sim {
        let s = layout.rest_s();
        let phase = if plan.is_empty() { Phase::Finished } else { Phase::Idle };
        Sim {
            yard,
            layout,
            state,
            goal,
            plan,
            step: 0,
            s,
            v: 0.0,
            v_max: 6.0,
            accel: 0.25,
            dwell_ticks: 12,
            ticks: 0,
            track: Track::Main,
            phase,
        }
    }

    pub fn finished(&self) -> bool {
        self.phase == Phase::Finished
    }

    pub fn current_move(&self) -> Option<Move> {
        if self.finished() { None } else { self.plan.get(self.step).copied() }
    }

    pub fn phase_name(&self) -> &'static str {
        match self.phase {
            Phase::Idle => "idle",
            Phase::Travel { then: Then::Exchange, .. } => "out",
            Phase::Travel { then: Then::Home, .. } => "back",
            Phase::Travel { then: Then::Recouple, .. } => "back",
            Phase::Loop { .. } => "round the loop",
            Phase::Approach { .. } => "backing on",
            Phase::Dwell { then: Then::Exchange, .. } => "coupling",
            Phase::Dwell { then: Then::Recouple, .. } => "coupling",
            Phase::Dwell { then: Then::Home, .. } => "home",
            Phase::Finished => "done",
        }
    }

    fn track_polyline(&self) -> Polyline {
        match self.track {
            Track::Main => self.layout.main_from(self.state.loco),
            Track::Siding(i) => self.layout.paths[i].clone(),
        }
    }

    /// Where the loco must stop so the exchange for `m` lines up.
    fn exchange_target(&self, siding: usize, m: Move) -> f32 {
        let cap = self.yard.sidings[siding].capacity;
        let n_sd = self.state.sidings[siding].len();
        let n_h = self.state.held.len();
        let cl = self.layout.car_len;
        match m {
            Move::Pull { .. } => {
                // Far end of string couples to the ladder-end car (slot cap-n_sd).
                self.layout.slot_s(siding, cap - n_sd) - (n_h as f32 + 1.0) * cl
            }
            Move::Push { .. } => {
                // Far-end car lands in the next free slot.
                self.layout.slot_s(siding, cap - n_sd - 1) - n_h as f32 * cl
            }
            Move::RunAround => unreachable!(),
        }
    }

    /// Where, along the main measured from the *far* throat, the loco must
    /// stop to couple onto the far end of its parked string.
    fn recouple_target(&self) -> f32 {
        let n_h = self.state.held.len() as f32;
        self.layout.main_len() - (n_h + 1.5) * self.layout.car_len
    }

    pub fn tick(&mut self) {
        self.ticks += 1;
        match self.phase {
            Phase::Finished => {}
            Phase::Idle => {
                let Some(m) = self.plan.get(self.step).copied() else {
                    self.phase = Phase::Finished;
                    return;
                };
                self.v = 0.0;
                match m {
                    Move::RunAround => {
                        debug_assert!(self.layout.loop_track.is_some(), "run-round without a loop");
                        self.track = Track::Main;
                        self.s = self.layout.rest_s();
                        self.phase = Phase::Loop { s: 0.0, v: 0.0 };
                    }
                    _ => {
                        let siding = m.siding().unwrap();
                        self.track = Track::Siding(siding);
                        self.phase = Phase::Travel { target: self.exchange_target(siding, m), then: Then::Exchange };
                    }
                }
            }
            Phase::Travel { target, then } => {
                let (s, v, arrived) = advance(self.s, self.v, target, self.v_max, self.accel);
                self.s = s;
                self.v = v;
                if arrived {
                    self.phase = Phase::Dwell { ticks: self.dwell_ticks, then };
                }
            }
            Phase::Loop { s, v } => {
                let len = self.layout.loop_path(self.state.loco).map(|p| p.length()).unwrap_or(0.0);
                let (s, v, arrived) = advance(s, v, len, self.v_max, self.accel);
                self.phase = if arrived {
                    Phase::Approach { s: 0.0, v: 0.0, target: self.recouple_target() }
                } else {
                    Phase::Loop { s, v }
                };
            }
            Phase::Approach { s, v, target } => {
                let (s, v, arrived) = advance(s, v, target, self.v_max, self.accel);
                self.phase = if arrived {
                    Phase::Dwell { ticks: self.dwell_ticks, then: Then::Recouple }
                } else {
                    Phase::Approach { s, v, target }
                };
            }
            Phase::Dwell { ticks, then } => {
                if ticks > 0 {
                    self.phase = Phase::Dwell { ticks: ticks - 1, then };
                    return;
                }
                match then {
                    Then::Exchange => {
                        self.apply_current();
                        self.v = 0.0;
                        self.phase = Phase::Travel { target: self.layout.rest_s(), then: Then::Home };
                    }
                    Then::Recouple => {
                        // Loco is now on the far side; the string is measured
                        // from that end. Its position there is exactly where
                        // the approach ended.
                        let at = self.recouple_target();
                        self.apply_current();
                        self.track = Track::Main;
                        self.s = at;
                        self.v = 0.0;
                        self.phase = Phase::Travel { target: self.layout.rest_s(), then: Then::Home };
                    }
                    Then::Home => {
                        self.step += 1;
                        self.phase = if self.step >= self.plan.len() { Phase::Finished } else { Phase::Idle };
                    }
                }
            }
        }
    }

    fn apply_current(&mut self) {
        let m = self.plan[self.step];
        self.state = self
            .yard
            .apply(&self.state, m)
            .unwrap_or_else(|| panic!("plan step {} illegal: {m} in {}", self.step, self.state));
    }

    pub fn snapshot(&self) -> SimSnapshot {
        let n_cars = self.state.sidings.iter().map(|s| s.len()).sum::<usize>() + self.state.held.len();
        let mut cars = vec![Pose { pos: P2::default(), heading: 0.0 }; n_cars];
        let cl = self.layout.car_len;

        // String (cars, and the loco unless it's off on its own).
        let path = self.track_polyline();
        let pose_at = |pl: &Polyline, s: f32| Pose { pos: pl.point_at(s), heading: pl.heading_at(s) };
        for (i, &c) in self.state.held.iter().enumerate() {
            cars[c as usize] = pose_at(&path, self.s + (i as f32 + 1.0) * cl);
        }
        let loco = match self.phase {
            Phase::Loop { s, .. } => {
                let lp = self.layout.loop_path(self.state.loco).expect("loop");
                pose_at(&lp, s)
            }
            Phase::Approach { s, .. } => pose_at(&self.layout.main_from(self.state.loco.opposite()), s),
            Phase::Dwell { then: Then::Recouple, .. } => {
                pose_at(&self.layout.main_from(self.state.loco.opposite()), self.recouple_target())
            }
            _ => pose_at(&path, self.s),
        };

        for (si, sd) in self.state.sidings.iter().enumerate() {
            let cap = self.yard.sidings[si].capacity;
            for (j, &c) in sd.iter().enumerate() {
                let (pos, heading) = self.layout.slot_pose(si, cap - sd.len() + j);
                cars[c as usize] = Pose { pos, heading };
            }
        }
        SimSnapshot { loco, cars }
    }

    /// Convenience for tests / headless runs.
    pub fn run_to_end(&mut self, max_ticks: u64) -> bool {
        while !self.finished() && self.ticks < max_ticks {
            self.tick();
        }
        self.finished()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layout;
    use crate::solver::{random_task, solve, Rng};
    use crate::yard::Side;

    fn dist(a: P2, b: P2) -> f32 {
        ((a.x - b.x).powi(2) + (a.y - b.y).powi(2)).sqrt()
    }

    #[test]
    fn executing_plans_reaches_the_goal() {
        for yard in [Yard::inglenook(), Yard::timesaver()] {
            let layout = Layout::ladder(&yard, 40.0, 36.0, 30.0);
            let mut rng = Rng::new(3);
            for _ in 0..8 {
                let (state, goal) = random_task(&yard, 8, 5, 0, &mut rng);
                let Ok(plan) = solve(&yard, &state, &goal, 2_000_000) else { continue };
                let mut sim = Sim::new(yard.clone(), layout.clone(), state, goal.clone(), plan.moves);
                assert!(sim.run_to_end(400_000), "did not finish");
                assert!(goal.satisfied(&sim.state));
                assert_eq!(sim.snapshot().cars.len(), 8);
            }
        }
    }

    #[test]
    fn coupling_positions_are_adjacent() {
        let yard = Yard::inglenook();
        let layout = Layout::ladder(&yard, 40.0, 36.0, 30.0);
        let state = State { sidings: vec![vec![0, 1], vec![], vec![]], held: vec![2], loco: Side::Left };
        let plan = vec![Move::Pull { siding: 0, count: 1 }];
        let mut sim = Sim::new(yard, layout, state, Goal { siding: 1, order: vec![] }, plan);
        while sim.phase_name() != "coupling" {
            sim.tick();
        }
        let snap = sim.snapshot();
        let d = dist(snap.cars[2].pos, snap.cars[0].pos);
        assert!((d - 40.0).abs() < 1e-3, "gap {d}");
    }

    #[test]
    fn run_round_recouples_adjacent_and_cars_never_jump() {
        let yard = Yard::timesaver();
        let layout = Layout::ladder(&yard, 40.0, 36.0, 30.0);
        let state = State { sidings: vec![vec![], vec![], vec![], vec![]], held: vec![0, 1, 2], loco: Side::Left };
        let plan = vec![Move::RunAround, Move::RunAround];
        let mut sim = Sim::new(yard, layout, state, Goal { siding: 1, order: vec![] }, plan);
        let mut prev = sim.snapshot();
        let mut saw_coupling = 0;
        while !sim.finished() {
            sim.tick();
            let snap = sim.snapshot();
            for (a, b) in prev.cars.iter().zip(&snap.cars) {
                assert!(dist(a.pos, b.pos) <= sim.v_max + 1e-3, "car jumped {:?} -> {:?}", a.pos, b.pos);
            }
            assert!(dist(prev.loco.pos, snap.loco.pos) <= sim.v_max + 1e-3, "loco jumped");
            if sim.phase_name() == "coupling" {
                // Loco is one car length from the near end of the string (car 2
                // before the first flip, car 0 after).
                let near = if sim.state.loco == Side::Left { 2 } else { 0 };
                let d = dist(snap.loco.pos, snap.cars[near].pos);
                assert!((d - 40.0).abs() < 1e-2, "recouple gap {d}");
                saw_coupling += 1;
            }
            prev = snap;
        }
        assert!(saw_coupling > 0);
        assert_eq!(sim.state.loco, Side::Left);
        assert_eq!(sim.state.held, vec![0, 1, 2]);
    }
}
