//! rutot - watch a yard work as a machine.
//!
//! Simulation runs in `FixedUpdate` at 30 Hz; rendering interpolates poses
//! between the last two fixed ticks using `Time<Fixed>::overstep_fraction`.
//!
//! Two presentation layers share the same simulation: `view2d` (schematic)
//! and `view3d` ("Hewn Cedar": a wooden tabletop diorama). Pick with
//! `RUTOT_VIEW=2d|3d` (default 3d).

mod view2d;
mod view3d;

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use rutot_core::{benchmark, random_task, solve, CarId, Layout, Move, Pose, Rng, Sim, Yard, YardStats};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Mutex;
use std::time::Duration;

pub const TICK_HZ: f64 = 30.0;
/// Layout units per car. Both views scale from this.
pub const CAR_LEN: f32 = 44.0;
pub const PITCH: f32 = 40.0;
pub const LADDER_DEG: f32 = 30.0;
pub const N_CARS: usize = 8;
pub const GOAL_LEN: usize = 5;
pub const GOAL_SIDING: usize = 0;
pub const STATS_TASKS: usize = 100;

pub const CAR_COLORS: [Color; 10] = [
    Color::srgb(0.94, 0.33, 0.31),
    Color::srgb(0.98, 0.60, 0.20),
    Color::srgb(0.98, 0.83, 0.30),
    Color::srgb(0.55, 0.85, 0.35),
    Color::srgb(0.25, 0.75, 0.62),
    Color::srgb(0.30, 0.72, 0.92),
    Color::srgb(0.42, 0.50, 0.95),
    Color::srgb(0.75, 0.45, 0.92),
    Color::srgb(0.95, 0.45, 0.72),
    Color::srgb(0.65, 0.65, 0.65),
];

fn yards() -> Vec<Yard> {
    vec![
        Yard::inglenook(),
        Yard::inglenook_long_lead(),
        Yard::inglenook_four(),
        Yard::inglenook_with_loop(),
        Yard::timesaver(),
    ]
}

type Planned = (Sim, usize, Duration);

// ---------------------------------------------------------------- resources

#[derive(Resource)]
pub struct Session {
    pub yards: Vec<Yard>,
    pub yard_idx: usize,
    rng: Rng,
    pub task_no: u32,
    pub sim: Sim,
    pub plan_states: usize,
    pub plan_time: Duration,
    /// Ticks spent finished before auto-advancing.
    finished_for: u32,
    /// A task being solved on another thread (Mutex only for `Sync`).
    planning: Option<Mutex<Receiver<Planned>>>,
}

impl Session {
    fn request_task(&mut self) {
        if self.planning.is_some() {
            return;
        }
        let yards = self.yards.clone();
        let idx = self.yard_idx;
        let mut rng = Rng::new(self.rng.next_u64());
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(new_task(&yards, idx, &mut rng));
        });
        self.planning = Some(Mutex::new(rx));
    }

    /// Install a finished plan if one has arrived. Returns true if it did.
    fn take_planned(&mut self) -> bool {
        let Some(rx) = &self.planning else { return false };
        let got = rx.lock().unwrap().try_recv();
        match got {
            Ok((sim, n, t)) => {
                self.sim = sim;
                self.plan_states = n;
                self.plan_time = t;
                self.task_no += 1;
                self.finished_for = 0;
                self.planning = None;
                true
            }
            Err(_) => false,
        }
    }

    pub fn is_planning(&self) -> bool {
        self.planning.is_some()
    }
}

#[derive(Resource)]
pub struct Playback {
    pub paused: bool,
    pub speed: u32,
    pub auto: bool,
}

#[derive(Resource)]
struct Stats {
    by_yard: Vec<Option<YardStats>>,
    inflight: Mutex<Option<(usize, Receiver<YardStats>)>>,
}

/// Set when the task changes; the active view consumes it (despawns and
/// respawns its scene) and clears it.
#[derive(Resource, Default)]
pub struct Rebuild(pub bool);

/// `RUTOT_SHOT_AFTER=<secs>` takes a screenshot then exits; handy for
/// CI/agents. `RUTOT_SHOT_PHASE=<phase name>` instead fires
/// `RUTOT_SHOT_PHASE_TICKS` sim ticks into the first occurrence of that sim
/// phase (e.g. "round the loop").
#[derive(Resource)]
struct AutoShot {
    at: f32,
    phase: Option<String>,
    phase_ticks: u32,
    taken: bool,
    taken_at: f32,
    shots: u32,
}

// --------------------------------------------------------------- components

#[derive(Component)]
pub struct Loco;

#[derive(Component)]
pub struct Car(pub CarId);

/// Poses at the last two fixed ticks; views interpolate between them.
#[derive(Component, Clone, Copy)]
pub struct Interp {
    pub prev: Pose,
    pub curr: Pose,
}

/// Anything the view rebuilds per task.
#[derive(Component)]
pub struct SceneObject;

#[derive(Component)]
struct Hud;

// ----------------------------------------------------------------- helpers

fn new_task(yards: &[Yard], idx: usize, rng: &mut Rng) -> Planned {
    let yard = yards[idx].clone();
    let layout = Layout::ladder(&yard, CAR_LEN, PITCH, LADDER_DEG);
    loop {
        let (state, goal) = random_task(&yard, N_CARS, GOAL_LEN, GOAL_SIDING, rng);
        match solve(&yard, &state, &goal, 2_000_000) {
            Ok(plan) if !plan.moves.is_empty() => {
                let (n, t) = (plan.expanded, plan.elapsed);
                return (Sim::new(yard, layout, state, goal, plan.moves), n, t);
            }
            _ => continue,
        }
    }
}

pub fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let mut d = b - a;
    while d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    while d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    a + d * t
}

pub fn car_color(c: CarId) -> Color {
    CAR_COLORS[c as usize % CAR_COLORS.len()]
}

pub fn env_f32(name: &str, default: f32) -> f32 {
    std::env::var(name).ok().and_then(|v| v.parse().ok()).unwrap_or(default)
}

// ------------------------------------------------------------------- main

fn main() {
    let yards = yards();
    let n = yards.len();
    let yard_idx = (env_f32("RUTOT_YARD", 0.0) as usize).min(n - 1);
    let seed = std::env::var("RUTOT_SEED").ok().and_then(|v| v.parse().ok()).unwrap_or(0x5eed);
    let mut rng = Rng::new(seed);
    let (sim, plan_states, plan_time) = new_task(&yards, yard_idx, &mut rng);
    let view3d = std::env::var("RUTOT_VIEW").map(|v| v != "2d").unwrap_or(true);

    let mut app = App::new();
    app.add_plugins(DefaultPlugins.set(WindowPlugin {
        primary_window: Some(Window {
            title: "rutot - the yard is a machine".into(),
            resolution: (1280, 800).into(),
            ..default()
        }),
        ..default()
    }))
    .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
    .insert_resource(Session {
        yards,
        yard_idx,
        rng,
        task_no: 1,
        sim,
        plan_states,
        plan_time,
        finished_for: 0,
        planning: None,
    })
    .insert_resource(Playback { paused: false, speed: 1, auto: true })
    .insert_resource(Stats { by_yard: vec![None; n], inflight: Mutex::new(None) })
    .insert_resource(Rebuild(true))
    .insert_resource(AutoShot {
        at: env_f32("RUTOT_SHOT_AFTER", -1.0),
        phase: std::env::var("RUTOT_SHOT_PHASE").ok(),
        phase_ticks: 0,
        taken: false,
        taken_at: 0.0,
        shots: 0,
    })
    .add_systems(Startup, setup_hud)
    .add_systems(FixedUpdate, step_sim)
    .add_systems(Update, (handle_input, screenshots, poll_stats, update_hud).chain());

    if view3d {
        app.add_plugins(view3d::View3dPlugin);
    } else {
        app.add_plugins(view2d::View2dPlugin);
    }
    app.run();
}

fn setup_hud(mut commands: Commands) {
    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(15.0), ..default() },
        TextColor(Color::srgb(0.92, 0.90, 0.86)),
        TextShadow { offset: Vec2::splat(1.0), color: Color::BLACK.with_alpha(0.6) },
        Node {
            position_type: PositionType::Absolute,
            top: Val::Px(12.0),
            left: Val::Px(12.0),
            padding: UiRect::axes(Val::Px(12.0), Val::Px(9.0)),
            border_radius: BorderRadius::all(Val::Px(8.0)),
            ..default()
        },
        BackgroundColor(Color::srgba(0.04, 0.03, 0.04, 0.55)),
        Hud,
    ));
}

// ----------------------------------------------------------------- systems

fn handle_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut session: ResMut<Session>,
    mut playback: ResMut<Playback>,
    mut rebuild: ResMut<Rebuild>,
) {
    if keys.just_pressed(KeyCode::Space) {
        playback.paused = !playback.paused;
    }
    if keys.just_pressed(KeyCode::KeyA) {
        playback.auto = !playback.auto;
    }
    if keys.just_pressed(KeyCode::Digit1) {
        playback.speed = 1;
    }
    if keys.just_pressed(KeyCode::Digit2) {
        playback.speed = 2;
    }
    if keys.just_pressed(KeyCode::Digit3) {
        playback.speed = 4;
    }
    let mut new_task_wanted = keys.just_pressed(KeyCode::KeyR);
    if keys.just_pressed(KeyCode::KeyY) {
        session.yard_idx = (session.yard_idx + 1) % session.yards.len();
        new_task_wanted = true;
    }
    if new_task_wanted {
        session.request_task();
    }
    if session.take_planned() {
        rebuild.0 = true;
    }
}

fn step_sim(
    mut session: ResMut<Session>,
    playback: Res<Playback>,
    mut movers: Query<(&mut Interp, Option<&Loco>, Option<&Car>)>,
) {
    if playback.paused {
        return;
    }
    for (mut it, _, _) in &mut movers {
        it.prev = it.curr;
    }
    for _ in 0..playback.speed {
        session.sim.tick();
    }
    if session.sim.finished() {
        session.finished_for += playback.speed;
        if playback.auto && session.finished_for > (TICK_HZ as u32) * 2 {
            session.request_task();
        }
    }
    let snap = session.sim.snapshot();
    for (mut it, loco, car) in &mut movers {
        if loco.is_some() {
            it.curr = snap.loco;
        } else if let Some(Car(id)) = car {
            it.curr = snap.cars[*id as usize];
        }
    }
}

fn poll_stats(session: Res<Session>, mut stats: ResMut<Stats>) {
    let idx = session.yard_idx;
    let mut guard = stats.inflight.lock().unwrap();
    if let Some((job_idx, rx)) = guard.as_ref() {
        if let Ok(result) = rx.try_recv() {
            let ji = *job_idx;
            drop(guard);
            stats.by_yard[ji] = Some(result);
        }
        return;
    }
    if stats.by_yard[idx].is_none() {
        let yard = session.yards[idx].clone();
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let _ = tx.send(benchmark(&yard, N_CARS, GOAL_LEN, GOAL_SIDING, STATS_TASKS, 1234));
        });
        *guard = Some((idx, rx));
    }
}

fn update_hud(session: Res<Session>, playback: Res<Playback>, stats: Res<Stats>, mut hud: Query<&mut Text, With<Hud>>) {
    let Ok(mut text) = hud.single_mut() else { return };
    let sim = &session.sim;
    let yard = &session.yards[session.yard_idx];
    let plan_len = sim.plan.len();
    let mv = match sim.current_move() {
        Some(m) => format!("move {}/{}: {}  ({})", sim.step + 1, plan_len, describe(m, yard), sim.phase_name()),
        None if session.is_planning() => "planning next task...".to_string(),
        None => format!("done in {plan_len} moves - {}", if playback.auto { "next task shortly" } else { "[R] for a new task" }),
    };
    let legs = rutot_core::plan_cost(&sim.plan);
    let rounds = sim.plan.iter().filter(|m| **m == Move::RunAround).count();
    let stat = match &stats.by_yard[session.yard_idx] {
        Some(s) => format!(
            "{} random tasks: mean {:.1} moves / {:.1} legs, {:.1} run-rounds, max {} moves, {} unsolved, {:.0} ms/solve",
            s.tasks,
            s.mean_moves(),
            s.mean_cost(),
            s.mean_run_arounds(),
            s.max_moves,
            s.tasks - s.solved,
            s.elapsed.as_secs_f64() * 1000.0 / s.tasks as f64
        ),
        None => "computing yard statistics...".to_string(),
    };
    let all: Vec<String> = session
        .yards
        .iter()
        .enumerate()
        .map(|(i, y)| {
            let mark = if i == session.yard_idx { ">" } else { " " };
            match &stats.by_yard[i] {
                Some(s) => format!(
                    "{mark} {:<38} {:>5.1} legs  {:>4.1} moves  {:>3.1} rounds",
                    y.name,
                    s.mean_cost(),
                    s.mean_moves(),
                    s.mean_run_arounds()
                ),
                None => format!("{mark} {:<38} ...", y.name),
            }
        })
        .collect();
    let state = format!("{}", sim.state);
    text.0 = format!(
        "rutot - the yard is a machine\n\
         \n\
         yard: {}      task #{}\n\
         plan: {} moves = {} legs ({} run-rounds), optimal  ({} states searched in {:.1} ms)\n\
         {}\n\
         {}\n\
         \n\
         {}\n\
         \n\
         yard throughput (recipe time, lower is better):\n{}\n\
         \n\
         [space] {}   [1/2/3] speed x{}   [R] new task   [Y] next yard   [A] auto {}   [P] screenshot   drag: orbit   wheel: zoom",
        yard.name,
        session.task_no,
        plan_len,
        legs,
        rounds,
        session.plan_states,
        session.plan_time.as_secs_f64() * 1000.0,
        mv,
        state,
        stat,
        all.join("\n"),
        if playback.paused { "resume" } else { "pause" },
        playback.speed,
        if playback.auto { "on" } else { "off" },
    );
}

fn screenshots(
    mut commands: Commands,
    keys: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    session: Res<Session>,
    mut auto: ResMut<AutoShot>,
    mut exit: MessageWriter<AppExit>,
) {
    let mut take = keys.just_pressed(KeyCode::KeyP);
    let automatic = auto.at >= 0.0 || auto.phase.is_some();
    if automatic && !auto.taken {
        let timed = auto.at >= 0.0 && time.elapsed_secs() >= auto.at;
        let phased = match &auto.phase {
            Some(p) if session.sim.phase_name() == p => {
                if auto.phase_ticks == 0 {
                    auto.phase_ticks = session.sim.ticks as u32;
                }
                let want = env_f32("RUTOT_SHOT_PHASE_TICKS", 30.0) as u64;
                session.sim.ticks - auto.phase_ticks as u64 >= want
            }
            _ => false,
        };
        if timed || phased {
            auto.taken = true;
            auto.taken_at = time.elapsed_secs();
            take = true;
        }
    }
    if automatic && auto.taken && time.elapsed_secs() >= auto.taken_at + 1.5 {
        exit.write(AppExit::Success);
    }
    if take {
        auto.shots += 1;
        let path = format!("rutot-{:03}.png", auto.shots);
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
    }
}

fn describe(m: Move, yard: &Yard) -> String {
    let name = |i: usize| yard.sidings[i].name.as_str();
    match m {
        Move::Pull { siding, count } => format!("pull {count} from {} (#{siding})", name(siding)),
        Move::Push { siding, count } => format!("push {count} onto {} (#{siding})", name(siding)),
        Move::RunAround => "run round the string".to_string(),
    }
}
