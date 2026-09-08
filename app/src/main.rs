//! rutot - watch a yard work as a machine.
//!
//! Simulation runs in `FixedUpdate` at 30 Hz; rendering interpolates poses
//! between the last two fixed ticks using `Time<Fixed>::overstep_fraction`.

use bevy::prelude::*;
use bevy::render::view::screenshot::{save_to_disk, Screenshot};
use bevy::sprite::Anchor;
use rutot_core::{
    benchmark, car_label, random_task, solve, CarId, Goal, Layout, Move, Pose, Rng, Sim, State, Yard, YardStats, P2,
};
use std::sync::mpsc::{channel, Receiver};
use std::sync::Mutex;
use std::time::Duration;

const TICK_HZ: f64 = 30.0;
const CAR_LEN: f32 = 44.0;
const PITCH: f32 = 40.0;
const LADDER_DEG: f32 = 30.0;
const N_CARS: usize = 8;
const GOAL_LEN: usize = 5;
const GOAL_SIDING: usize = 0;
const STATS_TASKS: usize = 100;

const CAR_COLORS: [Color; 10] = [
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
    vec![Yard::inglenook(), Yard::inglenook_long_lead(), Yard::inglenook_four()]
}

// ---------------------------------------------------------------- resources

#[derive(Resource)]
struct Session {
    yards: Vec<Yard>,
    yard_idx: usize,
    rng: Rng,
    task_no: u32,
    sim: Sim,
    plan_states: usize,
    plan_time: Duration,
    /// Ticks spent finished before auto-advancing.
    finished_for: u32,
}

#[derive(Resource)]
struct Playback {
    paused: bool,
    speed: u32,
    auto: bool,
}

#[derive(Resource)]
struct Stats {
    by_yard: Vec<Option<YardStats>>,
    inflight: Mutex<Option<(usize, Receiver<YardStats>)>>,
}

#[derive(Resource, Default)]
struct Rebuild(bool);

/// `RUTOT_SHOT_AFTER=<secs>` takes a screenshot then exits; handy for CI/agents.
#[derive(Resource)]
struct AutoShot { at: f32, taken: bool, shots: u32 }

// --------------------------------------------------------------- components

#[derive(Component)]
struct Loco;

#[derive(Component)]
struct Car(CarId);

#[derive(Component, Clone, Copy)]
struct Interp {
    prev: Pose,
    curr: Pose,
}

#[derive(Component)]
struct GoalRow;

#[derive(Component)]
struct Track;

#[derive(Component)]
struct Hud;

// ----------------------------------------------------------------- helpers

fn new_task(yards: &[Yard], idx: usize, rng: &mut Rng) -> (Sim, usize, Duration) {
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

fn v2(p: P2) -> Vec2 {
    Vec2::new(p.x, p.y)
}

fn lerp_angle(a: f32, b: f32, t: f32) -> f32 {
    let mut d = b - a;
    while d > std::f32::consts::PI {
        d -= std::f32::consts::TAU;
    }
    while d < -std::f32::consts::PI {
        d += std::f32::consts::TAU;
    }
    a + d * t
}

fn pose_transform(p: Pose, z: f32) -> Transform {
    Transform::from_xyz(p.pos.x, p.pos.y, z).with_rotation(Quat::from_rotation_z(p.heading))
}

fn car_color(c: CarId) -> Color {
    CAR_COLORS[c as usize % CAR_COLORS.len()]
}

// ------------------------------------------------------------------- main

fn main() {
    let yards = yards();
    let mut rng = Rng::new(0x5eed);
    let (sim, plan_states, plan_time) = new_task(&yards, 0, &mut rng);
    let n = yards.len();

    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "rutot - the yard is a machine".into(),
                resolution: (1280, 800).into(),
                ..default()
            }),
            ..default()
        }))
        .insert_resource(ClearColor(Color::srgb(0.10, 0.11, 0.13)))
        .insert_resource(Time::<Fixed>::from_hz(TICK_HZ))
        .insert_resource(Session {
            yards,
            yard_idx: 0,
            rng,
            task_no: 1,
            sim,
            plan_states,
            plan_time,
            finished_for: 0,
        })
        .insert_resource(Playback { paused: false, speed: 1, auto: true })
        .insert_resource(Stats { by_yard: vec![None; n], inflight: Mutex::new(None) })
        .insert_resource(Rebuild(true))
        .insert_resource(AutoShot {
            at: std::env::var("RUTOT_SHOT_AFTER").ok().and_then(|v| v.parse().ok()).unwrap_or(-1.0),
            taken: false,
            shots: 0,
        })
        .add_systems(Startup, setup)
        .add_systems(FixedUpdate, step_sim)
        .add_systems(
            Update,
            (handle_input, screenshots, rebuild_entities, poll_stats, draw_track, render_interpolated, update_hud).chain(),
        )
        .run();
}

fn setup(mut commands: Commands, session: Res<Session>) {
    let (lo, hi) = session.sim.layout.bounds();
    let cx = (lo.x + hi.x) * 0.5;
    let cy = (lo.y + hi.y) * 0.5 - 30.0;
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection { scale: 0.62, ..OrthographicProjection::default_2d() }),
        Transform::from_xyz(cx, cy, 0.0),
    ));

    commands.spawn((
        Text::new(""),
        TextFont { font_size: FontSize::Px(16.0), ..default() },
        TextColor(Color::srgb(0.9, 0.9, 0.9)),
        Node { position_type: PositionType::Absolute, top: Val::Px(12.0), left: Val::Px(14.0), ..default() },
        Hud,
    ));
}

// ----------------------------------------------------------------- systems

fn handle_input(
    keys: Res<ButtonInput<KeyCode>>,
    mut session: ResMut<Session>,
    mut playback: ResMut<Playback>,
    mut rebuild: ResMut<Rebuild>,
    mut camera: Query<&mut Transform, With<Camera2d>>,
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
        let idx = session.yard_idx;
        let Session { yards, rng, .. } = &mut *session;
        let (sim, n, t) = new_task(yards, idx, rng);
        session.sim = sim;
        session.plan_states = n;
        session.plan_time = t;
        session.task_no += 1;
        session.finished_for = 0;
        rebuild.0 = true;
        if let Ok(mut cam) = camera.single_mut() {
            let (lo, hi) = session.sim.layout.bounds();
            cam.translation.x = (lo.x + hi.x) * 0.5;
            cam.translation.y = (lo.y + hi.y) * 0.5 - 30.0;
        }
    }
}

fn rebuild_entities(
    mut commands: Commands,
    mut rebuild: ResMut<Rebuild>,
    session: Res<Session>,
    old: Query<Entity, Or<(With<Loco>, With<Car>, With<GoalRow>, With<Track>)>>,
) {
    if !rebuild.0 {
        return;
    }
    rebuild.0 = false;
    for e in &old {
        commands.entity(e).despawn();
    }
    let snap = session.sim.snapshot();
    let size = Vec2::new(CAR_LEN * 0.86, CAR_LEN * 0.42);

    spawn_track(&mut commands, &session);

    commands
        .spawn((
            Sprite::from_color(Color::srgb(0.35, 0.12, 0.12), Vec2::new(CAR_LEN * 0.9, CAR_LEN * 0.5)),
            pose_transform(snap.loco, 2.0),
            Loco,
            Interp { prev: snap.loco, curr: snap.loco },
        ))
        .with_children(|p| {
            p.spawn((
                Text2d::new("0008"),
                TextFont { font_size: FontSize::Px(11.0), ..default() },
                TextColor(Color::srgb(0.95, 0.85, 0.6)),
                Transform::from_xyz(0.0, 0.0, 0.1),
            ));
        });

    for (i, pose) in snap.cars.iter().enumerate() {
        let id = i as CarId;
        commands
            .spawn((
                Sprite::from_color(car_color(id), size),
                pose_transform(*pose, 2.0),
                Car(id),
                Interp { prev: *pose, curr: *pose },
            ))
            .with_children(|p| {
                p.spawn((
                    Text2d::new(car_label(id).to_string()),
                    TextFont { font_size: FontSize::Px(15.0), ..default() },
                    TextColor(Color::srgb(0.08, 0.08, 0.10)),
                    Transform::from_xyz(0.0, 0.0, 0.1),
                ));
            });
    }

    // Goal row under the headshunt: what the yard must produce.
    let layout = &session.sim.layout;
    let goal = &session.sim.goal;
    let y = -PITCH * 1.6;
    let x0 = -layout.head_len() + CAR_LEN * 0.5;
    commands.spawn((
        Text2d::new(format!("build on #{}:", goal.siding)),
        TextFont { font_size: FontSize::Px(14.0), ..default() },
        TextColor(Color::srgb(0.7, 0.7, 0.75)),
        Transform::from_xyz(x0 + CAR_LEN * 0.6, y + PITCH * 0.55, 1.0),
        GoalRow,
    ));
    for (k, &c) in goal.order.iter().enumerate() {
        let x = x0 + CAR_LEN * 1.6 + k as f32 * CAR_LEN * 0.95;
        commands
            .spawn((
                Sprite::from_color(car_color(c).with_alpha(0.85), size),
                Transform::from_xyz(x, y, 1.0),
                GoalRow,
            ))
            .with_children(|p| {
                p.spawn((
                    Text2d::new(car_label(c).to_string()),
                    TextFont { font_size: FontSize::Px(15.0), ..default() },
                    TextColor(Color::srgb(0.08, 0.08, 0.10)),
                    Transform::from_xyz(0.0, 0.0, 0.1),
                ));
            });
    }
}

fn step_sim(
    mut session: ResMut<Session>,
    playback: Res<Playback>,
    mut rebuild: ResMut<Rebuild>,
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
            let idx = session.yard_idx;
            let Session { yards, rng, .. } = &mut *session;
            let (sim, n, t) = new_task(yards, idx, rng);
            session.sim = sim;
            session.plan_states = n;
            session.plan_time = t;
            session.task_no += 1;
            session.finished_for = 0;
            rebuild.0 = true;
            return;
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

fn render_interpolated(fixed: Res<Time<Fixed>>, mut q: Query<(&Interp, &mut Transform)>) {
    let a = fixed.overstep_fraction();
    for (it, mut tf) in &mut q {
        let p = it.prev.pos;
        let c = it.curr.pos;
        tf.translation.x = p.x + (c.x - p.x) * a;
        tf.translation.y = p.y + (c.y - p.y) * a;
        tf.rotation = Quat::from_rotation_z(lerp_angle(it.prev.heading, it.curr.heading, a));
    }
}

/// Track is static per task, so it's sprites (under the cars), not gizmos
/// (which always draw on top).
fn spawn_track(commands: &mut Commands, session: &Session) {
    let layout = &session.sim.layout;
    let yard = &session.sim.yard;
    let goal = &session.sim.goal;
    let rail = Color::srgb(0.50, 0.52, 0.58);
    let goal_rail = Color::srgb(0.85, 0.75, 0.35);
    let sleeper = Color::srgb(0.22, 0.23, 0.26);
    let block = Color::srgb(0.8, 0.3, 0.3);
    let gauge = CAR_LEN * 0.30;

    let mut strip = |pl: &rutot_core::Polyline, c: Color| {
        for w in pl.pts.windows(2) {
            let (a, b) = (v2(w[0]), v2(w[1]));
            let mid = (a + b) * 0.5;
            let d = b - a;
            let len = d.length();
            let rot = Quat::from_rotation_z(d.y.atan2(d.x));
            // Two rails.
            for side in [-0.5, 0.5] {
                let off = Vec2::new(-d.y, d.x) / len * gauge * side;
                commands.spawn((
                    Sprite::from_color(c, Vec2::new(len, 1.5)),
                    Transform::from_translation((mid + off).extend(0.2)).with_rotation(rot),
                    Track,
                ));
            }
        }
        // Sleepers along the whole polyline.
        let mut s = 5.0;
        while s < pl.length() {
            let p = v2(pl.point_at(s));
            let h = pl.heading_at(s);
            commands.spawn((
                Sprite::from_color(sleeper, Vec2::new(3.0, gauge * 1.5)),
                Transform::from_translation(p.extend(0.1)).with_rotation(Quat::from_rotation_z(h)),
                Track,
            ));
            s += 10.0;
        }
    };

    strip(&layout.headshunt, rail);
    for (i, sd) in layout.sidings.iter().enumerate() {
        strip(sd, if i == goal.siding { goal_rail } else { rail });
    }

    // Stop blocks and labels.
    let mut marker = |p: Vec2| {
        commands.spawn((
            Sprite::from_color(block, Vec2::new(3.0, gauge * 2.2)),
            Transform::from_translation(p.extend(0.3)),
            Track,
        ));
    };
    marker(v2(layout.headshunt.pts[0]));
    for sd in &layout.sidings {
        marker(v2(*sd.pts.last().unwrap()));
    }
    for (i, sd) in layout.sidings.iter().enumerate() {
        let end = v2(*sd.pts.last().unwrap());
        commands.spawn((
            Text2d::new(format!("#{i} {} ({})", yard.sidings[i].name, yard.sidings[i].capacity)),
            TextFont { font_size: FontSize::Px(12.0), ..default() },
            TextColor(if i == goal.siding { goal_rail } else { Color::srgb(0.6, 0.6, 0.65) }),
            TextLayout::justify(Justify::Left),
            Anchor::CENTER_LEFT,
            Transform::from_translation((end + Vec2::new(10.0, 0.0)).extend(0.3)),
            Track,
        ));
    }
    let hs = v2(layout.headshunt.pts[0]);
    commands.spawn((
        Text2d::new(format!("headshunt (loco + {})", yard.headshunt)),
        TextFont { font_size: FontSize::Px(12.0), ..default() },
        TextColor(Color::srgb(0.6, 0.6, 0.65)),
        Anchor::CENTER_LEFT,
        Transform::from_translation((hs + Vec2::new(0.0, -PITCH * 0.55)).extend(0.3)),
        Track,
    ));
}

/// Ghost the goal prefix on the goal siding so you can see it fill in — only
/// on slots that are currently empty, so nothing is drawn over a car.
fn draw_track(mut gizmos: Gizmos, session: Res<Session>) {
    let layout = &session.sim.layout;
    let goal = &session.sim.goal;
    let goal_rail = Color::srgb(0.85, 0.75, 0.35);
    let cap = session.sim.yard.sidings[goal.siding].capacity;
    let n_now = session.sim.state.sidings[goal.siding].len();
    let first_occupied = cap - n_now;
    for k in 0..goal.order.len() {
        let slot = cap.saturating_sub(goal.order.len().max(n_now)) + k;
        if slot >= first_occupied {
            continue;
        }
        let (p, h) = layout.slot_pose(goal.siding, slot);
        let iso = Isometry2d::new(v2(p), Rot2::radians(h));
        gizmos.rect_2d(iso, Vec2::new(CAR_LEN * 0.86, CAR_LEN * 0.42), goal_rail.with_alpha(0.35));
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
            return;
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
        None => format!("done in {plan_len} moves - {}", if playback.auto { "next task shortly" } else { "[R] for a new task" }),
    };
    let stat = match &stats.by_yard[session.yard_idx] {
        Some(s) => format!(
            "{} random tasks: mean {:.1} moves, max {}, {:.0} ms/solve",
            s.tasks,
            s.mean_moves(),
            s.max_moves,
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
                Some(s) => format!("{mark} {:<28} mean {:>5.1}  max {:>2}", y.name, s.mean_moves(), s.max_moves),
                None => format!("{mark} {:<28} ...", y.name),
            }
        })
        .collect();
    let state = format!("{}", sim.state);
    text.0 = format!(
        "rutot - the yard is a machine\n\
         \n\
         yard: {}      task #{}\n\
         plan: {} moves, optimal  ({} states searched in {:.1} ms)\n\
         {}\n\
         {}\n\
         \n\
         {}\n\
         \n\
         yard throughput (recipe time):\n{}\n\
         \n\
         [space] {}   [1/2/3] speed x{}   [R] new task   [Y] next yard   [A] auto {}   [P] screenshot   30 Hz sim, interpolated render",
        yard.name,
        session.task_no,
        plan_len,
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
    mut auto: ResMut<AutoShot>,
    mut exit: MessageWriter<AppExit>,
) {
    let mut take = keys.just_pressed(KeyCode::KeyP);
    let mut then_exit = false;
    if auto.at >= 0.0 && !auto.taken && time.elapsed_secs() >= auto.at {
        auto.taken = true;
        take = true;
        then_exit = true;
    }
    if auto.taken && auto.at >= 0.0 && time.elapsed_secs() >= auto.at + 1.5 {
        exit.write(AppExit::Success);
    }
    if take {
        auto.shots += 1;
        let path = format!("rutot-{:03}.png", auto.shots);
        commands.spawn(Screenshot::primary_window()).observe(save_to_disk(path));
        let _ = then_exit;
    }
}

fn describe(m: Move, yard: &Yard) -> String {
    let name = |i: usize| yard.sidings[i].name.as_str();
    match m {
        Move::Pull { siding, count } => format!("pull {count} from {} (#{siding})", name(siding)),
        Move::Push { siding, count } => format!("push {count} onto {} (#{siding})", name(siding)),
    }
}

#[allow(dead_code)]
fn _types(_: &State, _: &Goal) {}
