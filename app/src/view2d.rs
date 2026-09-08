//! Schematic 2D view: sprites for track and cars, gizmos for goal ghosts.

use crate::{car_color, lerp_angle, Car, Interp, Loco, Rebuild, SceneObject, Session, CAR_LEN, PITCH};
use bevy::prelude::*;
use bevy::sprite::Anchor;
use rutot_core::{car_label, CarId, Layout, Pose, Side, P2};

pub struct View2dPlugin;

impl Plugin for View2dPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(ClearColor(Color::srgb(0.10, 0.11, 0.13)))
            .add_systems(Startup, setup_camera)
            .add_systems(Update, (rebuild_scene, draw_ghosts, render_interpolated).chain());
    }
}

fn v2(p: P2) -> Vec2 {
    Vec2::new(p.x, p.y)
}

/// Cars are symmetric; keep their lettering upright by folding headings
/// into (-90°, 90°].
fn upright(h: f32) -> f32 {
    use std::f32::consts::{FRAC_PI_2, PI};
    let mut h = h;
    while h > FRAC_PI_2 {
        h -= PI;
    }
    while h <= -FRAC_PI_2 {
        h += PI;
    }
    h
}

fn pose_transform(p: Pose, z: f32) -> Transform {
    Transform::from_xyz(p.pos.x, p.pos.y, z).with_rotation(Quat::from_rotation_z(upright(p.heading)))
}

fn fit_scale(layout: &Layout, window_w: f32) -> f32 {
    let (lo, hi) = layout.bounds();
    ((hi.x - lo.x + 260.0) / window_w).max(0.55)
}

fn setup_camera(mut commands: Commands, session: Res<Session>) {
    let (lo, hi) = session.sim.layout.bounds();
    commands.spawn((
        Camera2d,
        Projection::Orthographic(OrthographicProjection {
            scale: fit_scale(&session.sim.layout, 1280.0),
            ..OrthographicProjection::default_2d()
        }),
        Transform::from_xyz((lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5 - 30.0, 0.0),
    ));
}

fn rebuild_scene(
    mut commands: Commands,
    mut rebuild: ResMut<Rebuild>,
    session: Res<Session>,
    old: Query<Entity, With<SceneObject>>,
    mut camera: Query<(&mut Transform, &mut Projection), With<Camera2d>>,
    window: Query<&Window>,
) {
    if !rebuild.0 {
        return;
    }
    rebuild.0 = false;
    for e in &old {
        commands.entity(e).despawn();
    }
    if let Ok((mut cam, mut proj)) = camera.single_mut() {
        let (lo, hi) = session.sim.layout.bounds();
        cam.translation.x = (lo.x + hi.x) * 0.5;
        cam.translation.y = (lo.y + hi.y) * 0.5 - 30.0;
        if let Projection::Orthographic(o) = &mut *proj {
            o.scale = fit_scale(&session.sim.layout, window.single().map(|w| w.width()).unwrap_or(1280.0));
        }
    }

    let snap = session.sim.snapshot();
    let size = Vec2::new(CAR_LEN * 0.86, CAR_LEN * 0.42);
    spawn_track(&mut commands, &session);

    commands
        .spawn((
            Sprite::from_color(Color::srgb(0.35, 0.12, 0.12), Vec2::new(CAR_LEN * 0.9, CAR_LEN * 0.5)),
            pose_transform(snap.loco, 2.0),
            Loco,
            SceneObject,
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
                SceneObject,
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

    // Goal row under the main: what the yard must produce.
    let layout = &session.sim.layout;
    let goal = &session.sim.goal;
    let y = -PITCH * 1.6;
    let x0 = -layout.main_len() + CAR_LEN * 0.5;
    commands.spawn((
        Text2d::new(format!("build on #{}:", goal.siding)),
        TextFont { font_size: FontSize::Px(14.0), ..default() },
        TextColor(Color::srgb(0.7, 0.7, 0.75)),
        Transform::from_xyz(x0 + CAR_LEN * 0.6, y + PITCH * 0.55, 1.0),
        SceneObject,
    ));
    for (k, &c) in goal.order.iter().enumerate() {
        let x = x0 + CAR_LEN * 1.6 + k as f32 * CAR_LEN * 0.95;
        commands
            .spawn((Sprite::from_color(car_color(c).with_alpha(0.85), size), Transform::from_xyz(x, y, 1.0), SceneObject))
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

fn render_interpolated(fixed: Res<Time<Fixed>>, mut q: Query<(&Interp, &mut Transform)>) {
    let a = fixed.overstep_fraction();
    for (it, mut tf) in &mut q {
        let p = it.prev.pos;
        let c = it.curr.pos;
        tf.translation.x = p.x + (c.x - p.x) * a;
        tf.translation.y = p.y + (c.y - p.y) * a;
        tf.rotation = Quat::from_rotation_z(lerp_angle(upright(it.prev.heading), upright(it.curr.heading), a));
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
            for side in [-0.5, 0.5] {
                let off = Vec2::new(-d.y, d.x) / len * gauge * side;
                commands.spawn((
                    Sprite::from_color(c, Vec2::new(len, 1.5)),
                    Transform::from_translation((mid + off).extend(0.2)).with_rotation(rot),
                    SceneObject,
                ));
            }
        }
        let mut s = 5.0;
        while s < pl.length() {
            let p = v2(pl.point_at(s));
            let h = pl.heading_at(s);
            commands.spawn((
                Sprite::from_color(sleeper, Vec2::new(3.0, gauge * 1.5)),
                Transform::from_translation(p.extend(0.1)).with_rotation(Quat::from_rotation_z(h)),
                SceneObject,
            ));
            s += 10.0;
        }
    };

    strip(&layout.main, rail);
    if let Some(lp) = &layout.loop_track {
        strip(lp, Color::srgb(0.42, 0.55, 0.62));
    }
    for (i, sd) in layout.sidings.iter().enumerate() {
        strip(sd, if i == goal.siding { goal_rail } else { rail });
    }

    let mut marker = |p: Vec2| {
        commands.spawn((
            Sprite::from_color(block, Vec2::new(3.0, gauge * 2.2)),
            Transform::from_translation(p.extend(0.3)),
            SceneObject,
        ));
    };
    if !yard.has_side(Side::Left) {
        marker(v2(layout.throat(Side::Left)));
    }
    if !yard.has_side(Side::Right) {
        marker(v2(layout.throat(Side::Right)));
    }
    for sd in &layout.sidings {
        marker(v2(*sd.pts.last().unwrap()));
    }
    let dim = Color::srgb(0.6, 0.6, 0.65);
    for (i, sd) in layout.sidings.iter().enumerate() {
        let end = v2(*sd.pts.last().unwrap());
        let (anchor, dx) = match yard.sidings[i].side {
            Side::Right => (Anchor::CENTER_LEFT, 10.0),
            Side::Left => (Anchor::CENTER_RIGHT, -10.0),
        };
        commands.spawn((
            Text2d::new(format!("#{i} {} ({})", yard.sidings[i].name, yard.sidings[i].capacity)),
            TextFont { font_size: FontSize::Px(12.0), ..default() },
            TextColor(if i == goal.siding { goal_rail } else { dim }),
            anchor,
            Transform::from_translation((end + Vec2::new(dx, 0.0)).extend(0.3)),
            SceneObject,
        ));
    }
    let hs = v2(layout.throat(Side::Left));
    commands.spawn((
        Text2d::new(format!(
            "main (loco + {}){}",
            yard.headshunt,
            if yard.runaround { "   run-round loop above" } else { "" }
        )),
        TextFont { font_size: FontSize::Px(12.0), ..default() },
        TextColor(dim),
        Anchor::CENTER_LEFT,
        Transform::from_translation((hs + Vec2::new(0.0, -PITCH * 0.55)).extend(0.3)),
        SceneObject,
    ));
}

/// Ghost the goal prefix on the goal siding so you can see it fill in — only
/// on slots that are currently empty, so nothing is drawn over a car.
fn draw_ghosts(mut gizmos: Gizmos, session: Res<Session>) {
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
