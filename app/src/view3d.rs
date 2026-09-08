//! "Hewn Cedar" — a wooden tabletop railway diorama.
//!
//! Warm afternoon light through a little window, a cedar baseboard with
//! procedural grain, cork roadbed, walnut sleepers, tin rails, chunky
//! hand-hewn painted cars, a deep-red loco with brass trim, and a tilt-shift
//! depth of field so the whole thing reads as a miniature.

use crate::{car_color, env_f32, lerp_angle, Car, Interp, Loco, Playback, Rebuild, SceneObject, Session, CAR_LEN};
use bevy::anti_alias::smaa::Smaa;
use bevy::asset::embedded_asset;
use bevy::camera::Exposure;
use bevy::core_pipeline::tonemapping::Tonemapping;
use bevy::input::mouse::{AccumulatedMouseMotion, AccumulatedMouseScroll, MouseScrollUnit};
use bevy::light::{CascadeShadowConfigBuilder, GeneratedEnvironmentMapLight, GlobalAmbientLight, NotShadowCaster, NotShadowReceiver, RectLight};
use bevy::pbr::{
    ExtendedMaterial, MaterialExtension, ScreenSpaceAmbientOcclusion, ScreenSpaceAmbientOcclusionQualityLevel,
};
use bevy::post_process::bloom::Bloom;
use bevy::post_process::dof::{DepthOfField, DepthOfFieldMode};
use bevy::prelude::*;
use bevy::asset::RenderAssetUsages;
use bevy::camera::Hdr;
use bevy::render::render_resource::{
    AsBindGroup, Extent3d, TextureDimension, TextureFormat, TextureViewDescriptor, TextureViewDimension,
};
use bevy::shader::ShaderRef;
use bevy::transform::TransformSystems;
use rutot_core::{car_label, CarId, Side, P2};
use std::f32::consts::FRAC_PI_2;

/// Layout units → metres. One car length is one metre.
const W: f32 = 1.0 / CAR_LEN;
const GAUGE: f32 = 0.30;
const RAIL_TOP: f32 = 0.14;
const WHEEL_R: f32 = 0.09;

pub struct View3dPlugin;

impl Plugin for View3dPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/wood.wgsl");
        app.add_plugins(MaterialPlugin::<WoodMaterial>::default())
            .insert_resource(ClearColor(Color::srgb(0.075, 0.070, 0.078)))
            .insert_resource(GlobalAmbientLight { color: Color::srgb(1.0, 0.94, 0.86), brightness: 140.0, ..default() })
            .insert_resource(Orbit {
                centre: Vec3::ZERO,
                fit_width: 10.0,
                yaw: -0.22,
                pitch: 0.66,
                zoom: 1.0,
                target: Vec3::ZERO,
                dist: 12.0,
            })
            .insert_resource(Palette::default())
            .add_systems(Startup, (setup_palette, setup_scene_statics, setup_camera_and_lights).chain())
            .add_systems(
                Update,
                (rebuild_scene, orbit_input, apply_orbit, render_interpolated, spin_wheels, ghosts_pulse, smoke)
                    .chain(),
            )
            .add_systems(PostUpdate, place_labels.after(TransformSystems::Propagate));
    }
}

// --------------------------------------------------------------- materials

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct WoodExt {
    #[uniform(100)]
    light: Vec4,
    #[uniform(100)]
    dark: Vec4,
    #[uniform(100)]
    scale: f32,
    #[uniform(100)]
    ring_freq: f32,
    #[uniform(100)]
    warp: f32,
    #[uniform(100)]
    streak: f32,
    #[uniform(100)]
    plank_w: f32,
    #[uniform(100)]
    _pad: Vec3,
}

impl MaterialExtension for WoodExt {
    fn fragment_shader() -> ShaderRef {
        "embedded://rutot/shaders/wood.wgsl".into()
    }
}

type WoodMaterial = ExtendedMaterial<StandardMaterial, WoodExt>;

#[allow(clippy::too_many_arguments)]
fn wood(base: Color, light: Color, dark: Color, scale: f32, ring_freq: f32, warp: f32, streak: f32, plank_w: f32, rough: f32) -> WoodMaterial {
    let v = |c: Color| {
        let l = c.to_linear();
        Vec4::new(l.red, l.green, l.blue, 1.0)
    };
    ExtendedMaterial {
        base: StandardMaterial { base_color: base, perceptual_roughness: rough, reflectance: 0.35, ..default() },
        extension: WoodExt { light: v(light), dark: v(dark), scale, ring_freq, warp, streak, plank_w, _pad: Vec3::ZERO },
    }
}

/// Shared material handles.
#[derive(Resource, Default)]
struct Palette {
    cork: Handle<StandardMaterial>,
    sleeper: Handle<StandardMaterial>,
    rail: Handle<StandardMaterial>,
    rail_goal: Handle<StandardMaterial>,
    rail_loop: Handle<StandardMaterial>,
    block: Handle<StandardMaterial>,
    chassis: Handle<StandardMaterial>,
    wheel: Handle<StandardMaterial>,
    loco_paint: Handle<StandardMaterial>,
    brass: Handle<StandardMaterial>,
    lamp: Handle<StandardMaterial>,
    smoke: Handle<StandardMaterial>,
    ghost: Handle<StandardMaterial>,
    cars: Vec<Handle<StandardMaterial>>,
    // Meshes reused across the scene.
    unit_cube: Handle<Mesh>,
    wheel_mesh: Handle<Mesh>,
    sphere: Handle<Mesh>,
}

fn setup_palette(mut pal: ResMut<Palette>, mut mats: ResMut<Assets<StandardMaterial>>, mut meshes: ResMut<Assets<Mesh>>) {
    let matte = |c: Color, r: f32| StandardMaterial { base_color: c, perceptual_roughness: r, reflectance: 0.3, ..default() };
    let metal = |c: Color, r: f32| StandardMaterial { base_color: c, metallic: 1.0, perceptual_roughness: r, ..default() };
    pal.cork = mats.add(matte(Color::srgb(0.60, 0.46, 0.30), 0.95));
    pal.sleeper = mats.add(matte(Color::srgb(0.30, 0.19, 0.12), 0.85));
    pal.rail = mats.add(metal(Color::srgb(0.80, 0.80, 0.82), 0.30));
    pal.rail_goal = mats.add(metal(Color::srgb(0.95, 0.78, 0.40), 0.28));
    pal.rail_loop = mats.add(metal(Color::srgb(0.62, 0.74, 0.80), 0.30));
    pal.block = mats.add(matte(Color::srgb(0.72, 0.20, 0.16), 0.6));
    pal.chassis = mats.add(matte(Color::srgb(0.16, 0.13, 0.11), 0.8));
    pal.wheel = mats.add(StandardMaterial {
        base_color: Color::srgb(0.20, 0.20, 0.22),
        metallic: 0.7,
        perceptual_roughness: 0.45,
        ..default()
    });
    pal.loco_paint = mats.add(StandardMaterial {
        base_color: Color::srgb(0.46, 0.09, 0.08),
        perceptual_roughness: 0.38,
        reflectance: 0.5,
        ..default()
    });
    pal.brass = mats.add(metal(Color::srgb(0.95, 0.75, 0.35), 0.22));
    pal.lamp = mats.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.9, 0.7),
        emissive: LinearRgba::new(6.0, 4.2, 2.0, 1.0),
        ..default()
    });
    pal.smoke = mats.add(StandardMaterial {
        base_color: Color::srgba(0.88, 0.87, 0.90, 0.13),
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 1.0,
        ..default()
    });
    pal.ghost = mats.add(StandardMaterial {
        base_color: Color::srgba(0.95, 0.78, 0.40, 0.18),
        emissive: LinearRgba::new(0.6, 0.45, 0.15, 1.0),
        alpha_mode: AlphaMode::Blend,
        perceptual_roughness: 0.5,
        ..default()
    });
    pal.cars = (0..crate::CAR_COLORS.len() as u8)
        .map(|i| mats.add(StandardMaterial { base_color: car_color(i), perceptual_roughness: 0.55, reflectance: 0.4, ..default() }))
        .collect();
    pal.unit_cube = meshes.add(Cuboid::new(1.0, 1.0, 1.0));
    pal.wheel_mesh = meshes.add(Cylinder::new(WHEEL_R, 0.05));
    pal.sphere = meshes.add(Sphere::new(1.0).mesh().ico(3).unwrap());
}

// ---------------------------------------------------------------- geometry

fn to_world(p: P2, y: f32) -> Vec3 {
    Vec3::new(p.x * W, y, -p.y * W)
}

/// A box spanning `a`→`b` (world XZ) of the given width/height, its bottom at `y0`.
fn segment_box(a: Vec3, b: Vec3, width: f32, height: f32, y0: f32, overlap: f32) -> Transform {
    let d = b - a;
    let len = d.length() + overlap;
    let yaw = (-d.z).atan2(d.x);
    Transform {
        translation: (a + b) * 0.5 + Vec3::Y * (y0 + height * 0.5),
        rotation: Quat::from_rotation_y(yaw),
        scale: Vec3::new(len, height, width),
    }
}

/// Collapse coincident collinear segments (sidings share the ladder
/// diagonals), keeping the longest from each start point and direction.
fn unique_segments(polylines: &[(&rutot_core::Polyline, usize)]) -> Vec<(Vec3, Vec3, usize)> {
    let mut out: Vec<(Vec3, Vec3, usize)> = Vec::new();
    for (pl, tag) in polylines {
        for w in pl.pts.windows(2) {
            let (a, b) = (to_world(w[0], 0.0), to_world(w[1], 0.0));
            let dir = (b - a).normalize();
            let mut replaced = false;
            for seg in out.iter_mut() {
                let sdir = (seg.1 - seg.0).normalize();
                let same_start = seg.0.distance(a) < 1e-3;
                let same_dir = sdir.dot(dir) > 0.9999;
                if same_start && same_dir {
                    if b.distance(a) > seg.1.distance(seg.0) {
                        *seg = (a, b, *tag);
                    }
                    replaced = true;
                    break;
                }
            }
            if !replaced {
                out.push((a, b, *tag));
            }
        }
    }
    out
}

// ------------------------------------------------------------------ scene

#[derive(Component)]
struct Wheel {
    angle: f32,
}

#[derive(Component)]
struct Rolling {
    last: Vec3,
}

#[derive(Component)]
struct Ghost {
    slot: usize,
}

#[derive(Component)]
struct Smoke {
    age: f32,
    vel: Vec3,
}

#[derive(Component)]
struct Chimney;

/// UI text pinned to a world-space point.
#[derive(Component)]
struct Label {
    target: Entity,
    offset: Vec3,
}

/// Static furniture: the table under the board.
#[derive(Component)]
struct Furniture;

fn setup_scene_statics(mut commands: Commands, mut woods: ResMut<Assets<WoodMaterial>>, pal: Res<Palette>) {
    // The table the diorama sits on: dark walnut, coarse grain.
    let table = woods.add(wood(
        Color::srgb(0.9, 0.85, 0.8),
        Color::srgb(0.30, 0.19, 0.12),
        Color::srgb(0.14, 0.08, 0.05),
        0.35,
        9.0,
        2.5,
        0.35,
        2.2,
        0.55,
    ));
    commands.spawn((
        Mesh3d(pal.unit_cube.clone()),
        MeshMaterial3d(table),
        Transform { translation: Vec3::new(0.0, -0.75, 0.0), scale: Vec3::new(90.0, 1.0, 60.0), ..default() },
        Furniture,
    ));
}

fn setup_camera_and_lights(mut commands: Commands, mut images: ResMut<Assets<Image>>) {
    // Camera: HDR, SSAO (needs MSAA off), SMAA, bloom for the brass and the
    // headlamp, tilt-shift depth of field, a touch of fog for depth.
    let f_stops = env_f32("RUTOT_FSTOP", 0.6);
    commands.spawn((
        Camera3d::default(),
        Hdr,
        Msaa::Off,
        Tonemapping::TonyMcMapface,
        Exposure { ev100: 9.4 },
        Bloom { intensity: 0.10, ..Bloom::NATURAL },
        ScreenSpaceAmbientOcclusion {
            quality_level: ScreenSpaceAmbientOcclusionQualityLevel::High,
            constant_object_thickness: 0.3,
        },
        Smaa::default(),
        DepthOfField {
            mode: DepthOfFieldMode::Bokeh,
            focal_distance: 12.0,
            aperture_f_stops: f_stops,
            max_circle_of_confusion_diameter: 48.0,
            max_depth: 200.0,
            ..default()
        },
        DistanceFog {
            color: Color::srgb(0.075, 0.070, 0.078),
            falloff: FogFalloff::Linear { start: 28.0, end: 75.0 },
            ..default()
        },
        GeneratedEnvironmentMapLight {
            environment_map: images.add(room_cubemap()),
            intensity: 450.0,
            ..default()
        },
        Transform::from_xyz(0.0, 8.0, 12.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // The sun through the window: warm, low, casts the long shadows.
    commands.spawn((
        DirectionalLight {
            color: Color::srgb(1.0, 0.90, 0.76),
            illuminance: 5200.0,
            shadow_maps_enabled: true,
            ..default()
        },
        CascadeShadowConfigBuilder { num_cascades: 3, first_cascade_far_bound: 9.0, maximum_distance: 60.0, ..default() }
            .build(),
        Transform::from_xyz(-7.0, 8.5, 6.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));

    // The window itself: a soft area light so the rails and brass get a
    // broad highlight, not a pinpoint.
    commands.spawn((
        RectLight {
            color: Color::srgb(1.0, 0.94, 0.84),
            intensity: 260_000.0,
            range: 45.0,
            width: 3.2,
            height: 2.2,
        },
        Transform::from_xyz(-7.5, 6.5, 5.0).looking_at(Vec3::new(0.0, 0.3, 0.0), Vec3::Y),
    ));

    // Cool bounce from the other side of the room.
    commands.spawn((
        DirectionalLight {
            color: Color::srgb(0.66, 0.76, 1.0),
            illuminance: 700.0,
            shadow_maps_enabled: false,
            ..default()
        },
        Transform::from_xyz(8.0, 5.0, -7.0).looking_at(Vec3::ZERO, Vec3::Y),
    ));
}

/// A tiny gradient cubemap standing in for the room: warm wood below, cool
/// ceiling above, a bright warm patch on the window side.
fn room_cubemap() -> Image {
    const N: usize = 32;
    let mut data = vec![0u8; N * N * 6 * 4];
    let floor = Vec3::new(0.22, 0.15, 0.09);
    let ceil = Vec3::new(0.50, 0.58, 0.72);
    let window = Vec3::new(1.0, 0.92, 0.78);
    // Face order: +X, -X, +Y, -Y, +Z, -Z.
    for face in 0..6 {
        for y in 0..N {
            for x in 0..N {
                let u = (x as f32 + 0.5) / N as f32;
                let v = (y as f32 + 0.5) / N as f32; // 0 = top row
                let mut c = match face {
                    2 => ceil,
                    3 => floor,
                    _ => floor.lerp(ceil, 1.0 - v),
                };
                if face == 1 {
                    // -X is the window wall.
                    let dx = (u - 0.5).abs() / 0.22;
                    let dy = (v - 0.38).abs() / 0.2;
                    let pane = (1.0 - (dx * dx + dy * dy)).clamp(0.0, 1.0);
                    c = c.lerp(window, pane * 0.9);
                }
                let i = ((face * N + y) * N + x) * 4;
                let srgb = |f: f32| (f.clamp(0.0, 1.0).powf(1.0 / 2.2) * 255.0) as u8;
                data[i] = srgb(c.x);
                data[i + 1] = srgb(c.y);
                data[i + 2] = srgb(c.z);
                data[i + 3] = 255;
            }
        }
    }
    let mut img = Image::new(
        Extent3d { width: N as u32, height: (N * 6) as u32, depth_or_array_layers: 1 },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::RENDER_WORLD,
    );
    img.reinterpret_stacked_2d_as_array(6).expect("6 square faces");
    img.texture_view_descriptor = Some(TextureViewDescriptor { dimension: Some(TextureViewDimension::Cube), ..default() });
    img
}

#[allow(clippy::too_many_arguments)]
fn rebuild_scene(
    mut commands: Commands,
    mut rebuild: ResMut<Rebuild>,
    session: Res<Session>,
    pal: Res<Palette>,
    mut woods: ResMut<Assets<WoodMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut orbit: ResMut<Orbit>,
    old: Query<Entity, With<SceneObject>>,
) {
    if !rebuild.0 {
        return;
    }
    rebuild.0 = false;
    for e in &old {
        commands.entity(e).despawn();
    }

    let layout = &session.sim.layout;
    let yard = &session.sim.yard;
    let goal = &session.sim.goal;

    // Frame the yard.
    let (lo, hi) = layout.bounds();
    let centre = to_world(P2::new((lo.x + hi.x) * 0.5, (lo.y + hi.y) * 0.5), 0.0);
    let width = (hi.x - lo.x) * W;
    // Framing is finished per-frame in `apply_orbit` against the live
    // window aspect (the first frame still reports the requested size).
    orbit.centre = centre + Vec3::new(0.0, 0.15, 0.0);
    orbit.fit_width = width;
    orbit.zoom = 1.0;

    // Baseboard: pale cedar, fine grain along X.
    let board = woods.add(wood(
        Color::srgb(1.0, 0.96, 0.9),
        Color::srgb(0.86, 0.68, 0.46),
        Color::srgb(0.58, 0.40, 0.24),
        1.0,
        30.0,
        0.9,
        0.28,
        1.35,
        0.62,
    ));
    let margin = 2.6;
    commands.spawn((
        Mesh3d(pal.unit_cube.clone()),
        MeshMaterial3d(board),
        Transform {
            translation: centre - Vec3::Y * 0.125,
            scale: Vec3::new(width + margin * 2.0, 0.25, (hi.y - lo.y) * W + margin * 2.0),
            ..default()
        },
        SceneObject,
    ));

    // Track.
    let mut pls: Vec<(&rutot_core::Polyline, usize)> = vec![(&layout.main, 0)];
    if let Some(lp) = &layout.loop_track {
        pls.push((lp, 1));
    }
    for (i, sd) in layout.sidings.iter().enumerate() {
        pls.push((sd, if i == goal.siding { 2 } else { 0 }));
    }
    let segs = unique_segments(&pls);
    for (a, b, tag) in &segs {
        let rail = match tag {
            1 => pal.rail_loop.clone(),
            2 => pal.rail_goal.clone(),
            _ => pal.rail.clone(),
        };
        // Cork roadbed.
        commands.spawn((
            Mesh3d(pal.unit_cube.clone()),
            MeshMaterial3d(pal.cork.clone()),
            segment_box(*a, *b, 0.68, 0.05, 0.0, 0.3),
            SceneObject,
        ));
        // Rails.
        let d = (*b - *a).normalize();
        let n = Vec3::new(-d.z, 0.0, d.x);
        for side in [-0.5, 0.5] {
            let off = n * GAUGE * side;
            commands.spawn((
                Mesh3d(pal.unit_cube.clone()),
                MeshMaterial3d(rail.clone()),
                segment_box(*a + off, *b + off, 0.035, 0.05, RAIL_TOP - 0.05, 0.04),
                SceneObject,
            ));
        }
        // Sleepers.
        let len = a.distance(*b);
        let mut s = 0.12;
        while s < len {
            let p = *a + d * s;
            commands.spawn((
                Mesh3d(pal.unit_cube.clone()),
                MeshMaterial3d(pal.sleeper.clone()),
                Transform {
                    translation: p + Vec3::Y * (0.05 + 0.02),
                    rotation: Quat::from_rotation_y((-d.z).atan2(d.x)),
                    scale: Vec3::new(0.09, 0.04, 0.50),
                },
                SceneObject,
            ));
            s += 0.24;
        }
    }

    // Stop blocks and end labels.
    let mut block = |p: P2, h: f32| {
        commands.spawn((
            Mesh3d(pal.unit_cube.clone()),
            MeshMaterial3d(pal.block.clone()),
            Transform {
                translation: to_world(p, 0.05 + 0.11),
                rotation: Quat::from_rotation_y(h),
                scale: Vec3::new(0.10, 0.22, 0.46),
            },
            SceneObject,
        ));
    };
    if !yard.has_side(Side::Left) {
        block(layout.throat(Side::Left), 0.0);
    }
    if !yard.has_side(Side::Right) {
        block(layout.throat(Side::Right), 0.0);
    }
    for sd in &layout.sidings {
        block(*sd.pts.last().unwrap(), sd.heading_at(sd.length()));
    }
    let dim = Color::srgb(0.78, 0.74, 0.68);
    let gold = Color::srgb(0.95, 0.80, 0.45);
    for (i, sd) in layout.sidings.iter().enumerate() {
        let end = to_world(*sd.pts.last().unwrap(), 0.0);
        // Label floats just past the stop block, on the aisle side.
        let dx = match yard.sidings[i].side {
            Side::Right => 0.55,
            Side::Left => -0.55,
        };
        let anchor = commands
            .spawn((Transform::from_translation(end + Vec3::new(dx, 0.0, 0.55)), Visibility::Hidden, SceneObject))
            .id();
        spawn_label(
            &mut commands,
            anchor,
            Vec3::ZERO,
            format!("#{i} {} ({})", yard.sidings[i].name, yard.sidings[i].capacity),
            12.0,
            if i == goal.siding { gold } else { dim },
            110.0,
        );
    }

    // Cars.
    let snap = session.sim.snapshot();
    for (i, pose) in snap.cars.iter().enumerate() {
        let id = i as CarId;
        let jitter = hash01(id as u32 * 7 + 3) - 0.5;
        let root = commands
            .spawn((
                Transform::from_translation(to_world(pose.pos, 0.0)).with_rotation(Quat::from_rotation_y(pose.heading)),
                Visibility::default(),
                Car(id),
                SceneObject,
                Interp { prev: *pose, curr: *pose },
                Rolling { last: to_world(pose.pos, 0.0) },
                Hewn { yaw: jitter * 0.02 },
            ))
            .with_children(|p| {
                let sx = 1.0 + (hash01(id as u32 * 13 + 1) - 0.5) * 0.05;
                let sy = 1.0 + (hash01(id as u32 * 17 + 5) - 0.5) * 0.08;
                // Chassis.
                p.spawn((
                    Mesh3d(pal.unit_cube.clone()),
                    MeshMaterial3d(pal.chassis.clone()),
                    Transform { translation: Vec3::new(0.0, RAIL_TOP + WHEEL_R + 0.03, 0.0), scale: Vec3::new(0.80, 0.06, 0.40), ..default() },
                ));
                // Body: a chunky hewn block.
                p.spawn((
                    Mesh3d(pal.unit_cube.clone()),
                    MeshMaterial3d(pal.cars[id as usize % pal.cars.len()].clone()),
                    Transform {
                        translation: Vec3::new(0.0, RAIL_TOP + WHEEL_R + 0.06 + 0.18 * sy, 0.0),
                        rotation: Quat::from_rotation_z(jitter * 0.012),
                        scale: Vec3::new(0.86 * sx, 0.36 * sy, 0.44),
                    },
                ));
                spawn_wheels(p, &pal, 0.28, WHEEL_R);
            })
            .id();
        spawn_label(&mut commands, root, Vec3::new(0.0, 0.95, 0.0), car_label(id).to_string(), 17.0, Color::srgb(0.98, 0.96, 0.92), 30.0);
    }

    // Loco.
    let lp = snap.loco;
    let loco = commands
        .spawn((
            Transform::from_translation(to_world(lp.pos, 0.0)).with_rotation(Quat::from_rotation_y(lp.heading)),
            Visibility::default(),
            Loco,
            SceneObject,
            Interp { prev: lp, curr: lp },
            Rolling { last: to_world(lp.pos, 0.0) },
            Hewn { yaw: 0.0 },
        ))
        .with_children(|p| {
            let base = RAIL_TOP + WHEEL_R + 0.03;
            p.spawn((
                Mesh3d(pal.unit_cube.clone()),
                MeshMaterial3d(pal.chassis.clone()),
                Transform { translation: Vec3::new(0.0, base, 0.0), scale: Vec3::new(0.92, 0.06, 0.42), ..default() },
            ));
            // Boiler along X.
            let boiler = meshes.add(Cylinder::new(0.17, 0.56));
            p.spawn((
                Mesh3d(boiler),
                MeshMaterial3d(pal.loco_paint.clone()),
                Transform {
                    translation: Vec3::new(0.14, base + 0.03 + 0.17, 0.0),
                    rotation: Quat::from_rotation_z(FRAC_PI_2),
                    ..default()
                },
            ));
            // Cab.
            p.spawn((
                Mesh3d(pal.unit_cube.clone()),
                MeshMaterial3d(pal.loco_paint.clone()),
                Transform { translation: Vec3::new(-0.30, base + 0.03 + 0.22, 0.0), scale: Vec3::new(0.28, 0.44, 0.44), ..default() },
            ));
            // Cab roof, brass.
            p.spawn((
                Mesh3d(pal.unit_cube.clone()),
                MeshMaterial3d(pal.brass.clone()),
                Transform { translation: Vec3::new(-0.30, base + 0.03 + 0.455, 0.0), scale: Vec3::new(0.32, 0.03, 0.48), ..default() },
            ));
            // Chimney + brass cap.
            let chimney = meshes.add(Cylinder::new(0.05, 0.22));
            p.spawn((
                Mesh3d(chimney),
                MeshMaterial3d(pal.chassis.clone()),
                Transform::from_xyz(0.32, base + 0.03 + 0.34 + 0.08, 0.0),
                Chimney,
            ));
            let cap = meshes.add(Cylinder::new(0.065, 0.04));
            p.spawn((Mesh3d(cap), MeshMaterial3d(pal.brass.clone()), Transform::from_xyz(0.32, base + 0.03 + 0.34 + 0.20, 0.0)));
            // Brass dome.
            p.spawn((
                Mesh3d(pal.sphere.clone()),
                MeshMaterial3d(pal.brass.clone()),
                Transform { translation: Vec3::new(0.05, base + 0.03 + 0.34, 0.0), scale: Vec3::splat(0.075), ..default() },
            ));
            // Headlamp: emissive bead + a real light on the rails ahead.
            p.spawn((
                Mesh3d(pal.sphere.clone()),
                MeshMaterial3d(pal.lamp.clone()),
                Transform { translation: Vec3::new(0.43, base + 0.03 + 0.17, 0.0), scale: Vec3::splat(0.045), ..default() },
            ));
            p.spawn((
                PointLight {
                    color: Color::srgb(1.0, 0.86, 0.6),
                    intensity: 3_000.0,
                    range: 3.0,
                    radius: 0.03,
                    shadow_maps_enabled: false,
                    ..default()
                },
                Transform::from_xyz(0.55, base + 0.03 + 0.17, 0.0),
            ));
            // Buffers.
            for x in [0.47, -0.47] {
                for z in [-0.12, 0.12] {
                    p.spawn((
                        Mesh3d(pal.unit_cube.clone()),
                        MeshMaterial3d(pal.brass.clone()),
                        Transform { translation: Vec3::new(x, base + 0.02, z), scale: Vec3::new(0.05, 0.05, 0.05), ..default() },
                    ));
                }
            }
            spawn_wheels(p, &pal, 0.30, WHEEL_R);
        })
        .id();
    spawn_label(&mut commands, loco, Vec3::new(0.0, 1.1, 0.0), "0008".into(), 13.0, Color::srgb(0.98, 0.86, 0.6), 44.0);

    // Goal ghosts on the goal siding.
    let cap = yard.sidings[goal.siding].capacity;
    for k in 0..goal.order.len() {
        let slot = cap - goal.order.len() + k;
        let (p, h) = layout.slot_pose(goal.siding, slot);
        commands.spawn((
            Mesh3d(pal.unit_cube.clone()),
            MeshMaterial3d(pal.ghost.clone()),
            Transform {
                translation: to_world(p, RAIL_TOP + WHEEL_R + 0.06 + 0.18),
                rotation: Quat::from_rotation_y(h),
                scale: Vec3::new(0.86, 0.36, 0.44),
            },
            Ghost { slot },
            NotShadowCaster,
            NotShadowReceiver,
            SceneObject,
        ));
    }

    // Goal row: UI along the bottom.
    commands
        .spawn((
            Node {
                position_type: PositionType::Absolute,
                left: Val::Px(0.0),
                right: Val::Px(0.0),
                bottom: Val::Px(26.0),
                justify_content: JustifyContent::Center,
                ..default()
            },
            SceneObject,
        ))
        .with_children(|p| {
            p.spawn((
                Node {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(7.0),
                    padding: UiRect::axes(Val::Px(14.0), Val::Px(8.0)),
                    border_radius: BorderRadius::all(Val::Px(8.0)),
                    ..default()
                },
                BackgroundColor(Color::srgba(0.05, 0.04, 0.05, 0.55)),
            ))
            .with_children(|row| {
                row.spawn((
                    Text::new(format!("build on #{} {}:", goal.siding, yard.sidings[goal.siding].name)),
                    TextFont { font_size: FontSize::Px(14.0), ..default() },
                    TextColor(dim),
                    Node { margin: UiRect::right(Val::Px(6.0)), ..default() },
                ));
                for &c in &goal.order {
                    row.spawn((
                        Node {
                            width: Val::Px(38.0),
                            height: Val::Px(24.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            border_radius: BorderRadius::all(Val::Px(4.0)),
                            ..default()
                        },
                        BackgroundColor(car_color(c)),
                    ))
                    .with_children(|sq| {
                        sq.spawn((
                            Text::new(car_label(c).to_string()),
                            TextFont { font_size: FontSize::Px(15.0), ..default() },
                            TextColor(Color::srgb(0.08, 0.07, 0.08)),
                        ));
                    });
                }
            });
        });
}

/// Hand-made: each car sits very slightly askew.
#[derive(Component)]
struct Hewn {
    yaw: f32,
}

fn hash01(n: u32) -> f32 {
    let mut x = n.wrapping_mul(0x9E37_79B9) ^ 0x85EB_CA6B;
    x ^= x >> 15;
    x = x.wrapping_mul(0x2C1B_3C6D);
    x ^= x >> 12;
    (x & 0xFFFF) as f32 / 65535.0
}

fn spawn_wheels(p: &mut ChildSpawnerCommands, pal: &Palette, half_span: f32, r: f32) {
    for x in [-half_span, half_span] {
        for z in [-(GAUGE * 0.5 + 0.02), GAUGE * 0.5 + 0.02] {
            p.spawn((
                Mesh3d(pal.wheel_mesh.clone()),
                MeshMaterial3d(pal.wheel.clone()),
                Transform { translation: Vec3::new(x, RAIL_TOP + r, z), rotation: Quat::from_rotation_x(FRAC_PI_2), ..default() },
                Wheel { angle: 0.0 },
            ));
        }
    }
}

fn spawn_label(commands: &mut Commands, target: Entity, offset: Vec3, text: String, size: f32, color: Color, width: f32) {
    commands.spawn((
        Node {
            position_type: PositionType::Absolute,
            width: Val::Px(width),
            justify_content: JustifyContent::Center,
            ..default()
        },
        Label { target, offset },
        SceneObject,
        children![(
            Text::new(text),
            TextFont { font_size: FontSize::Px(size), ..default() },
            TextColor(color),
            TextShadow { offset: Vec2::splat(1.0), color: Color::BLACK.with_alpha(0.75) },
        )],
    ));
}

// ----------------------------------------------------------------- camera

#[derive(Resource)]
struct Orbit {
    /// Yard centre; the camera aims a little past it so the yard sits low.
    centre: Vec3,
    /// Yard width in metres, fitted to the window's horizontal FOV.
    fit_width: f32,
    yaw: f32,
    pitch: f32,
    /// Wheel zoom, multiplies the fitted distance.
    zoom: f32,
    /// Derived each frame.
    target: Vec3,
    dist: f32,
}

fn orbit_input(
    buttons: Res<ButtonInput<MouseButton>>,
    motion: Res<AccumulatedMouseMotion>,
    scroll: Res<AccumulatedMouseScroll>,
    mut orbit: ResMut<Orbit>,
) {
    if buttons.pressed(MouseButton::Left) || buttons.pressed(MouseButton::Right) {
        orbit.yaw -= motion.delta.x * 0.005;
        orbit.pitch = (orbit.pitch + motion.delta.y * 0.005).clamp(0.12, 1.45);
    }
    let dy = match scroll.unit {
        MouseScrollUnit::Line => scroll.delta.y,
        MouseScrollUnit::Pixel => scroll.delta.y / 40.0,
    };
    if dy != 0.0 {
        orbit.zoom = (orbit.zoom * (1.0 - dy * 0.1)).clamp(0.25, 3.0);
    }
}

fn apply_orbit(
    mut orbit: ResMut<Orbit>,
    window: Query<&Window>,
    mut cam: Query<(&mut Transform, &mut DepthOfField), With<Camera3d>>,
) {
    let Ok((mut tf, mut dof)) = cam.single_mut() else { return };
    // Fit the yard's width to the horizontal field of view.
    let aspect = window.single().map(|w| w.width() / w.height()).unwrap_or(1.6);
    let hfov = 2.0 * ((45.0_f32.to_radians() * 0.5).tan() * aspect).atan();
    let fit = (orbit.fit_width * 0.5 + 0.9) / (hfov * 0.5).tan() * 1.04;
    orbit.dist = (fit * orbit.zoom).clamp(3.0, 60.0);
    let away = Vec3::new(orbit.yaw.sin(), 0.0, orbit.yaw.cos());
    orbit.target = orbit.centre - away * (orbit.dist * 0.12);
    let dir = Vec3::new(orbit.pitch.cos() * orbit.yaw.sin(), orbit.pitch.sin(), orbit.pitch.cos() * orbit.yaw.cos());
    tf.translation = orbit.target + dir * orbit.dist;
    tf.look_at(orbit.target, Vec3::Y);
    dof.focal_distance = orbit.dist;
}

// ---------------------------------------------------------------- per-frame

fn render_interpolated(fixed: Res<Time<Fixed>>, mut q: Query<(&Interp, &Hewn, &mut Transform)>) {
    let a = fixed.overstep_fraction();
    for (it, hewn, mut tf) in &mut q {
        let p = it.prev.pos;
        let c = it.curr.pos;
        let pos = P2::new(p.x + (c.x - p.x) * a, p.y + (c.y - p.y) * a);
        tf.translation = to_world(pos, 0.0);
        tf.rotation = Quat::from_rotation_y(lerp_angle(it.prev.heading, it.curr.heading, a) + hewn.yaw);
    }
}

fn spin_wheels(
    mut roots: Query<(&Transform, &mut Rolling, &Children), With<Interp>>,
    mut wheels: Query<(&mut Transform, &mut Wheel), Without<Interp>>,
) {
    for (tf, mut roll, children) in &mut roots {
        let delta = tf.translation - roll.last;
        roll.last = tf.translation;
        let forward = tf.rotation * Vec3::X;
        let signed = delta.dot(forward);
        if signed.abs() < 1e-6 {
            continue;
        }
        for child in children.iter() {
            if let Ok((mut wtf, mut wheel)) = wheels.get_mut(child) {
                wheel.angle -= signed / WHEEL_R;
                wtf.rotation = Quat::from_rotation_z(wheel.angle) * Quat::from_rotation_x(FRAC_PI_2);
            }
        }
    }
}

fn ghosts_pulse(
    time: Res<Time>,
    session: Res<Session>,
    pal: Res<Palette>,
    mut mats: ResMut<Assets<StandardMaterial>>,
    mut ghosts: Query<(&Ghost, &mut Visibility)>,
) {
    let goal = &session.sim.goal;
    let cap = session.sim.yard.sidings[goal.siding].capacity;
    let n_now = session.sim.state.sidings[goal.siding].len();
    let first_occupied = cap - n_now;
    for (g, mut vis) in &mut ghosts {
        *vis = if g.slot < first_occupied { Visibility::Inherited } else { Visibility::Hidden };
    }
    if let Some(mut m) = mats.get_mut(&pal.ghost) {
        let t = 0.5 + 0.5 * (time.elapsed_secs() * 2.2).sin();
        m.base_color = Color::srgba(0.95, 0.78, 0.40, 0.10 + 0.14 * t);
        m.emissive = LinearRgba::new(0.35 + 0.5 * t, 0.26 + 0.38 * t, 0.08 + 0.12 * t, 1.0);
    }
}

#[allow(clippy::too_many_arguments)]
fn smoke(
    mut commands: Commands,
    time: Res<Time>,
    playback: Res<Playback>,
    pal: Res<Palette>,
    loco: Query<(&Transform, &Rolling), With<Loco>>,
    chimney: Query<&GlobalTransform, With<Chimney>>,
    mut puffs: Query<(Entity, &mut Transform, &mut Smoke), Without<Loco>>,
    mut acc: Local<f32>,
) {
    let dt = time.delta_secs();
    for (e, mut tf, mut s) in &mut puffs {
        s.age += dt;
        if s.age > 1.7 {
            commands.entity(e).despawn();
            continue;
        }
        let k = s.age / 1.7;
        tf.translation += s.vel * dt;
        s.vel.y *= 0.985;
        s.vel.x += 0.25 * dt; // a little draught across the room
        tf.scale = Vec3::splat(0.06 + 0.30 * k);
    }
    if playback.paused {
        return;
    }
    let Ok((tf, roll)) = loco.single() else { return };
    let moving = tf.translation.distance(roll.last) > 1e-4 || *acc < 0.0;
    *acc += dt;
    let interval = if moving { 0.11 } else { 0.55 };
    if *acc < interval {
        return;
    }
    *acc = 0.0;
    let Ok(ch) = chimney.single() else { return };
    let seed = time.elapsed_secs() * 37.0;
    commands.spawn((
        Mesh3d(pal.sphere.clone()),
        MeshMaterial3d(pal.smoke.clone()),
        Transform { translation: ch.translation() + Vec3::Y * 0.14, scale: Vec3::splat(0.06), ..default() },
        Smoke { age: 0.0, vel: Vec3::new((seed.sin()) * 0.12, 0.7 + (seed * 1.7).cos() * 0.15, (seed * 0.7).cos() * 0.12) },
        NotShadowCaster,
        NotShadowReceiver,
        SceneObject,
    ));
}

fn place_labels(
    cam: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    targets: Query<&GlobalTransform>,
    mut labels: Query<(&Label, &mut Node, &mut Visibility)>,
) {
    let Ok((camera, cam_tf)) = cam.single() else { return };
    for (label, mut node, mut vis) in &mut labels {
        let Ok(t) = targets.get(label.target) else {
            *vis = Visibility::Hidden;
            continue;
        };
        let world = t.translation() + label.offset;
        match camera.world_to_viewport(cam_tf, world) {
            Ok(p) => {
                let w = match node.width {
                    Val::Px(w) => w,
                    _ => 0.0,
                };
                node.left = Val::Px(p.x - w * 0.5);
                node.top = Val::Px(p.y - 10.0);
                *vis = Visibility::Inherited;
            }
            Err(_) => *vis = Visibility::Hidden,
        }
    }
}

