use std::time::Duration;

use bevy::camera::primitives::Aabb;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::render::render_resource::Buffer;
use bevy::render::renderer::{RenderDevice, RenderQueue};
use bevy::render::view::screenshot::{Screenshot, ScreenshotCaptured};
use bevy::time::TimeUpdateStrategy;
use bevy::window::PrimaryWindow;

use benilla_assets::materials::WowModelMaterial;
use benilla_assets::{m2_url, M2Model};
use benilla_world::doodad_anim::{TintAnimMaterials, UvAnimMaterials};
use benilla_world::lighting::LightBlob;
use benilla_world::mat_anim_table::MatAnimTable;
use benilla_world::model_forms::ModelForms;
use benilla_world::model_render::{MaterialCache, ShadeSel};
use benilla_world::schedule::WorldStage;
use benilla_world::terrain_stream::{m2_anim_bound, m2_fade, spawn_model_entities};
use benilla_world::view::WorldCamera;

use crate::perf::PerfHud;
use crate::run_mode::CaptureMode;

use super::{camera_distance, camera_eye, StudioBounds, StudioRequest, StudioView, STUDIO_FOV};

const STUDIO_LAYER: usize = 31;
const STABLE_FRAMES: u32 = 30;
const BUILD_CAP_FRAMES: u32 = 1800;
const SAVE_TIMEOUT_FRAMES: u32 = 180;
const EXIT_GRACE_FRAMES: u32 = 3;
const CAPTURE_FRAME_DT: Duration = Duration::from_nanos(16_666_667);

#[derive(Resource)]
struct StudioLight(Buffer);

#[derive(Component)]
struct StudioSubject;

#[derive(Component)]
struct StabilityShot;

#[derive(Clone, Copy, Debug)]
enum StudioPhase {
    Loading(u32),
    Settling(u32),
    Saving(u32),
    Done(u32),
    Failed(u32),
}

#[derive(Resource)]
struct StudioCtx {
    request: StudioRequest,
    model: Handle<M2Model>,
    bounds: Option<StudioBounds>,
    view: usize,
    phase: StudioPhase,
    prev_frame: Option<Vec<u8>>,
    stable: u32,
    readback_in_flight: bool,
    failure: Option<String>,
}

impl StudioCtx {
    fn fail(&mut self, message: impl Into<String>) {
        let message = message.into();
        error!("m2studio: {message}");
        self.failure = Some(message);
        self.phase = StudioPhase::Failed(0);
        self.readback_in_flight = false;
    }

    fn reset_settle(&mut self) {
        self.prev_frame = None;
        self.stable = 0;
        self.readback_in_flight = false;
        self.phase = StudioPhase::Settling(0);
    }
}

pub(crate) struct M2StudioPlugin;

impl Plugin for M2StudioPlugin {
    fn build(&self, app: &mut App) {
        let request = match StudioRequest::from_env() {
            Ok(request) => request,
            Err(e) => {
                eprintln!("m2studio: {e}");
                std::process::exit(2);
            }
        };
        if let Err(e) = std::fs::create_dir_all(&request.out_dir) {
            eprintln!(
                "m2studio: cannot create output directory {}: {e}",
                request.out_dir.display()
            );
            std::process::exit(2);
        }

        let model = app
            .world()
            .resource::<AssetServer>()
            .load(m2_url(&request.model_path));

        app.insert_resource(CaptureMode)
            .insert_resource(TimeUpdateStrategy::ManualDuration(CAPTURE_FRAME_DT))
            .insert_resource(ClearColor(Color::WHITE))
            .insert_resource(StudioCtx {
                request,
                model,
                bounds: None,
                view: 0,
                phase: StudioPhase::Loading(0),
                prev_frame: None,
                stable: 0,
                readback_in_flight: false,
                failure: None,
            })
            .add_systems(Startup, setup_studio)
            .add_systems(Update, spawn_subject)
            .add_systems(
                Update,
                (isolate_studio_cameras, pin_studio_camera)
                    .chain()
                    .in_set(WorldStage::Present),
            )
            .add_systems(Last, drive_capture);
    }
}

fn setup_studio(
    mut commands: Commands,
    device: Res<RenderDevice>,
    queue: Res<RenderQueue>,
    mut clock: ResMut<Time<Virtual>>,
    mut perf: ResMut<PerfHud>,
) {
    clock.pause();
    perf.visible = false;

    let blob = LightBlob::model(
        [0.42, 0.42, 0.42],
        [0.78, 0.78, 0.78],
        Vec3::new(-0.35, -0.85, -0.40).normalize(),
    );
    let buffer = blob.create(&device, "m2studio-light");
    blob.write(&queue, &buffer);
    commands.insert_resource(StudioLight(buffer));
}

#[allow(clippy::too_many_arguments)]
fn spawn_subject(
    mut commands: Commands,
    mut ctx: ResMut<StudioCtx>,
    models: Res<Assets<M2Model>>,
    mut forms: ResMut<ModelForms>,
    mut mat_cache: Local<MaterialCache>,
    mut materials: ResMut<Assets<WowModelMaterial>>,
    light: Option<Res<StudioLight>>,
    mut uv_reg: ResMut<UvAnimMaterials>,
    mut tint_reg: ResMut<TintAnimMaterials>,
    mut anim_table: ResMut<MatAnimTable>,
) {
    let StudioPhase::Loading(frame) = ctx.phase else {
        return;
    };
    if frame >= BUILD_CAP_FRAMES {
        let model_path = ctx.request.model_path.clone();
        ctx.fail(format!(
            "model did not become renderable within {BUILD_CAP_FRAMES} frames: {model_path}"
        ));
        return;
    }
    let Some(light) = light else {
        ctx.phase = StudioPhase::Loading(frame + 1);
        return;
    };
    let Some(model) = models.get(&ctx.model) else {
        ctx.phase = StudioPhase::Loading(frame + 1);
        return;
    };
    if !forms.require_static(&ctx.model, -1000) {
        ctx.phase = StudioPhase::Loading(frame + 1);
        return;
    }

    let form_slices = forms.slices(&ctx.model);
    let mut bounds = StudioBounds::empty();
    for (_, aabb) in form_slices.stat {
        let Some(aabb) = aabb else { continue };
        include_aabb(&mut bounds, aabb);
    }
    if !bounds.is_valid() {
        let model_path = ctx.request.model_path.clone();
        ctx.fail(format!("model has no usable rendered bounds: {model_path}"));
        return;
    }

    let (radius, local_center) = m2_fade(&model.bounds, 1.0);
    let spawned = spawn_model_entities(
        &mut commands,
        &mut mat_cache,
        &mut materials,
        &light.0,
        &model.submeshes,
        form_slices,
        Transform::IDENTITY,
        false,
        ShadeSel::Lit,
        None,
        radius,
        local_center,
        m2_anim_bound(&model.bounds),
        None,
        &mut uv_reg,
        &mut tint_reg,
        &mut anim_table,
        None,
        None,
        None,
    );

    if spawned.entities.is_empty() {
        let model_path = ctx.request.model_path.clone();
        ctx.fail(format!("model produced no render entities: {model_path}"));
        return;
    }
    for entity in spawned.entities {
        commands
            .entity(entity)
            .insert((StudioSubject, RenderLayers::layer(STUDIO_LAYER)));
    }

    ctx.bounds = Some(bounds);
    ctx.reset_settle();
    info!(
        "m2studio: subject ready: {} → {}",
        ctx.request.model_path,
        ctx.request.out_dir.display()
    );
}

fn include_aabb(bounds: &mut StudioBounds, aabb: &Aabb) {
    let c = aabb.center;
    let h = aabb.half_extents;
    bounds.include([c.x - h.x, c.y - h.y, c.z - h.z]);
    bounds.include([c.x + h.x, c.y + h.y, c.z + h.z]);
}

fn isolate_studio_cameras(
    mut commands: Commands,
    mut cameras: Query<(Entity, &mut Camera, Has<WorldCamera>)>,
) {
    for (entity, mut camera, world) in &mut cameras {
        if world {
            camera.is_active = true;
            camera.clear_color = bevy::camera::ClearColorConfig::Custom(Color::WHITE);
            commands
                .entity(entity)
                .insert(RenderLayers::layer(STUDIO_LAYER));
        } else {
            camera.is_active = false;
        }
    }
}

fn pin_studio_camera(
    ctx: Res<StudioCtx>,
    windows: Query<&Window, With<PrimaryWindow>>,
    mut camera: Query<(&mut Transform, &mut Projection), With<WorldCamera>>,
) {
    let Some(bounds) = ctx.bounds else {
        return;
    };
    let Some(view) = StudioView::ALL.get(ctx.view).copied() else {
        return;
    };
    let Ok(window) = windows.single() else {
        return;
    };
    let Ok((mut transform, mut projection)) = camera.single_mut() else {
        return;
    };

    let aspect = window.width() / window.height().max(1.0);
    let Some(distance) = camera_distance(&bounds, view.forward(), STUDIO_FOV, aspect) else {
        return;
    };
    let center = bounds.center();
    let eye = camera_eye(center, view.forward(), distance);
    let center = Vec3::from_array(center);
    let eye = Vec3::from_array(eye);
    *transform = Transform::from_translation(eye).looking_at(center, Vec3::Y);
    if let Projection::Perspective(perspective) = &mut *projection {
        perspective.fov = STUDIO_FOV;
    }
}

fn watch_frame(shot: On<ScreenshotCaptured>, mut ctx: ResMut<StudioCtx>) {
    ctx.readback_in_flight = false;
    let Some(bytes) = shot.image.data.as_ref() else {
        return;
    };
    ctx.stable = if ctx.prev_frame.as_deref() == Some(bytes.as_slice()) {
        ctx.stable + 1
    } else {
        0
    };
    ctx.prev_frame = Some(bytes.clone());
}

fn save_view(shot: On<ScreenshotCaptured>, mut ctx: ResMut<StudioCtx>) {
    let Some(view) = StudioView::ALL.get(ctx.view).copied() else {
        ctx.fail("capture view index escaped the four-view set");
        return;
    };
    let path = ctx.request.out_dir.join(view.file_name());
    let result = shot
        .image
        .clone()
        .try_into_dynamic()
        .map_err(|e| format!("failed to convert {}: {e}", view.file_name()))
        .and_then(|image| {
            image
                .to_rgb8()
                .save_with_format(&path, image::ImageFormat::Png)
                .map_err(|e| format!("failed to save {}: {e}", path.display()))
        });
    if let Err(message) = result {
        ctx.fail(message);
        return;
    }

    info!("m2studio: wrote {}", path.display());
    ctx.view += 1;
    if ctx.view == StudioView::ALL.len() {
        ctx.phase = StudioPhase::Done(0);
        ctx.readback_in_flight = false;
    } else {
        ctx.reset_settle();
    }
}

fn drive_capture(
    mut commands: Commands,
    mut ctx: ResMut<StudioCtx>,
    mut exit: MessageWriter<AppExit>,
) {
    ctx.phase = match ctx.phase {
        StudioPhase::Loading(frame) => StudioPhase::Loading(frame),
        StudioPhase::Settling(frame) => {
            if frame >= BUILD_CAP_FRAMES {
                let view_name = StudioView::ALL[ctx.view].file_name();
                ctx.fail(format!(
                    "view {view_name} never stabilized in {BUILD_CAP_FRAMES} frames"
                ));
                return;
            }
            if ctx.stable >= STABLE_FRAMES {
                commands
                    .spawn(Screenshot::primary_window())
                    .observe(save_view);
                StudioPhase::Saving(0)
            } else {
                if !ctx.readback_in_flight {
                    ctx.readback_in_flight = true;
                    commands
                        .spawn((Screenshot::primary_window(), StabilityShot))
                        .observe(watch_frame);
                }
                StudioPhase::Settling(frame + 1)
            }
        }
        StudioPhase::Saving(frame) => {
            if frame >= SAVE_TIMEOUT_FRAMES {
                let view_name = StudioView::ALL
                    .get(ctx.view)
                    .map(|v| v.file_name())
                    .unwrap_or("unknown view");
                ctx.fail(format!("timed out saving {view_name}"));
                return;
            }
            StudioPhase::Saving(frame + 1)
        }
        StudioPhase::Done(frame) => {
            if frame >= EXIT_GRACE_FRAMES {
                let missing: Vec<_> = StudioView::ALL
                    .iter()
                    .map(|view| view.file_name())
                    .filter(|name| !ctx.request.out_dir.join(name).is_file())
                    .collect();
                if missing.is_empty() {
                    info!(
                        "m2studio: four-view capture complete: {}",
                        ctx.request.out_dir.display()
                    );
                    exit.write(AppExit::Success);
                } else {
                    error!("m2studio: capture finished with missing files: {missing:?}");
                    exit.write(AppExit::error());
                }
                StudioPhase::Done(frame)
            } else {
                StudioPhase::Done(frame + 1)
            }
        }
        StudioPhase::Failed(frame) => {
            if frame >= EXIT_GRACE_FRAMES {
                if let Some(message) = &ctx.failure {
                    error!("m2studio: failed: {message}");
                }
                exit.write(AppExit::error());
                StudioPhase::Failed(frame)
            } else {
                StudioPhase::Failed(frame + 1)
            }
        }
    };
}
