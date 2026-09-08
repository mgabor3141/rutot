//! Fixed-tick execution of a plan: the loco physically runs each move as two
//! legs (out to the siding, back to the headshunt) with a dwell for coupling.
//! Pure logic; the renderer interpolates between ticks.

use crate::layout::{Layout, P2};
use crate::yard::{CarId, Goal, Move, State, Yard};

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

#[derive(Clone, Copy, Debug, PartialEq)]
enum Phase {
    /// Between moves, at rest on the headshunt.
    Idle,
    /// Travelling along the active path toward `target`.
    Travel { target: f32, then: Then },
    /// Stopped for `ticks` more ticks, then do `then`.
    Dwell { ticks: u32, then: Then },
    Finished,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Then {
    /// Apply the current move to the state (couple/uncouple), then head home.
    Exchange,
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
    /// Index of the move currently executing (or next to execute).
    pub step: usize,
    /// Which siding's path the string is on (last one visited when idle).
    pub active: usize,
    /// Loco centre along `layout.paths[active]`.
    pub s: f32,
    /// Current speed, path units per tick.
    pub v: f32,
    pub v_max: f32,
    pub accel: f32,
    pub dwell_ticks: u32,
    pub ticks: u64,
    phase: Phase,
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
            active: 0,
            s,
            v: 0.0,
            v_max: 6.0,
            accel: 0.25,
            dwell_ticks: 12,
            ticks: 0,
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
            Phase::Dwell { then: Then::Exchange, .. } => "coupling",
            Phase::Dwell { then: Then::Home, .. } => "home",
            Phase::Finished => "done",
        }
    }

    /// Where the loco must stop so the exchange for `m` lines up.
    fn exchange_target(&self, m: Move) -> f32 {
        let cap = self.yard.sidings[m.siding()].capacity;
        let n_sd = self.state.sidings[m.siding()].len();
        let n_h = self.state.held.len();
        let cl = self.layout.car_len;
        match m {
            Move::Pull { siding, .. } => {
                // Far end of string couples to the ladder-end car (slot cap-n_sd).
                let s_car = self.layout.slot_s(siding, cap - n_sd);
                s_car - (n_h as f32 + 1.0) * cl
            }
            Move::Push { siding, .. } => {
                // Far-end car lands in the next free slot.
                let s_slot = self.layout.slot_s(siding, cap - n_sd - 1);
                s_slot - n_h as f32 * cl
            }
        }
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
                self.active = m.siding();
                self.v = 0.0;
                self.phase = Phase::Travel { target: self.exchange_target(m), then: Then::Exchange };
            }
            Phase::Travel { target, then } => {
                let remaining = target - self.s;
                let dist = remaining.abs();
                // Trapezoidal profile: accelerate, cruise, brake to a stop.
                let v_brake = (2.0 * self.accel * dist).sqrt();
                self.v = (self.v + self.accel).min(self.v_max).min(v_brake).max(0.15);
                if dist <= self.v {
                    self.s = target;
                    self.v = 0.0;
                    self.phase = Phase::Dwell { ticks: self.dwell_ticks, then };
                } else {
                    self.s += self.v * remaining.signum();
                }
            }
            Phase::Dwell { ticks, then } => {
                if ticks > 0 {
                    self.phase = Phase::Dwell { ticks: ticks - 1, then };
                    return;
                }
                match then {
                    Then::Exchange => {
                        let m = self.plan[self.step];
                        self.state = self
                            .yard
                            .apply(&self.state, m)
                            .unwrap_or_else(|| panic!("plan step {} illegal: {m} in {}", self.step, self.state));
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

    pub fn snapshot(&self) -> SimSnapshot {
        let n_cars = self.state.sidings.iter().map(|s| s.len()).sum::<usize>() + self.state.held.len();
        let mut cars = vec![Pose { pos: P2::default(), heading: 0.0 }; n_cars];
        let path = &self.layout.paths[self.active];
        let cl = self.layout.car_len;
        let loco = Pose { pos: path.point_at(self.s), heading: path.heading_at(self.s) };
        for (i, &c) in self.state.held.iter().enumerate() {
            let s = self.s + (i as f32 + 1.0) * cl;
            cars[c as usize] = Pose { pos: path.point_at(s), heading: path.heading_at(s) };
        }
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

#[allow(dead_code)]
fn _car_id_is_u8(_: CarId) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::layout::Layout;
    use crate::solver::{random_task, solve, Rng};

    #[test]
    fn executing_a_plan_reaches_the_goal() {
        let yard = Yard::inglenook();
        let layout = Layout::ladder(&yard, 40.0, 36.0, 30.0);
        let mut rng = Rng::new(3);
        for _ in 0..5 {
            let (state, goal) = random_task(&yard, 8, 5, 0, &mut rng);
            let plan = solve(&yard, &state, &goal, 2_000_000).unwrap().moves;
            let mut sim = Sim::new(yard.clone(), layout.clone(), state, goal.clone(), plan);
            assert!(sim.run_to_end(200_000), "did not finish");
            assert!(goal.satisfied(&sim.state));
            // Everyone is somewhere sensible.
            let snap = sim.snapshot();
            assert_eq!(snap.cars.len(), 8);
        }
    }

    #[test]
    fn coupling_positions_are_adjacent() {
        // After the out-leg of a pull, the far-end held car must sit exactly
        // one car length from the ladder-end siding car.
        let yard = Yard::inglenook();
        let layout = Layout::ladder(&yard, 40.0, 36.0, 30.0);
        let state = State { sidings: vec![vec![0, 1], vec![], vec![]], held: vec![2] };
        let plan = vec![Move::Pull { siding: 0, count: 1 }];
        let mut sim = Sim::new(yard, layout, state, Goal { siding: 1, order: vec![] }, plan);
        while sim.phase_name() != "coupling" {
            sim.tick();
        }
        let snap = sim.snapshot();
        let held_far = snap.cars[2].pos;
        let ladder_end = snap.cars[0].pos;
        let d = ((held_far.x - ladder_end.x).powi(2) + (held_far.y - ladder_end.y).powi(2)).sqrt();
        assert!((d - 40.0).abs() < 1e-3, "gap {d}");
    }
}
