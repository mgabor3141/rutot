//! rutot-core: a yard is a machine. Cars in, a consist out, search in between.

pub mod layout;
pub mod sim;
pub mod solver;
pub mod yard;

pub use layout::{Layout, Polyline, P2};
pub use sim::{Pose, Sim, SimSnapshot};
pub use solver::{benchmark, random_task, solve, Plan, Rng, SolveError, YardStats};
pub use yard::{car_label, plan_cost, CarId, Goal, Move, Side, Siding, SidingId, State, Yard};
