# M2 Remaster Capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a developer-only Remaster Mode that selects a static world M2 doodad and launches a server-less studio child process that writes deterministic front/right/back/left PNG reference renders.

**Architecture:** Keep all new behavior in `benilla-app`'s existing developer seam. A new `dev/m2studio.rs` owns both the live picker/egui/process orchestration and a dedicated `M2StudioPlugin` for the child run. `dev.rs` only registers the live plugin and routes `WOW_CAPTURE=m2studio` away from the existing single-shot `CapturePlugin`, so the large existing capture harness remains unchanged. The studio subject is rendered through `benilla_world::terrain_stream::spawn_model_entities` and `ModelForms`, using a frozen off-world `LightBlob` and the same WoW model material pipeline as normal doodads.

**Tech Stack:** Rust 2021, Bevy 0.18.1, bevy_egui 0.39, benilla-assets/world production M2 pipeline, Bevy screenshot readback, `image` PNG encoder, `std::process::Command`.

**Spec:** `docs/superpowers/specs/2026-08-22-m2-remaster-capture-design.md`

## Global Constraints

- Developer-only: must compile out under `cargo build -p benilla --no-default-features`.
- Static world doodads only in v1; WMO, Creature and GameObject are rejected.
- M2/MDX model paths only, case-insensitive.
- Do not modify ADT placement, transforms, collision, server protocol, or gameplay data.
- One child process renders all four views.
- Output names are exactly `front.png`, `right.png`, `back.png`, `left.png`.
- Output root is `target/remaster-captures/<stable-model-stem>/` and generated images remain uncommitted.
- White background, fixed neutral off-world lighting, fixed perspective FOV, 1.12 framing margin.
- Model is loaded through `m2_url` and rendered through the production M2 material/mesh assembler.
- Parent never blocks its Bevy update/render loop waiting for the child.
- No new dependency.

---

## File Structure

- Create `crates/benilla-app/src/dev/m2studio.rs`: pure request/path/camera helpers; Remaster live plugin; studio child plugin; tests.
- Modify `crates/benilla-app/src/dev.rs`: declare `m2studio`, add `RemasterPlugin` to `DevToolsPlugin`, route `WOW_CAPTURE=m2studio` to `M2StudioPlugin` in `DevProbesPlugin`.
- Create `.github/workflows/remaster-ci.yml` while developing: branch-only test/check gate with no uploaded artifacts; remove it before completion unless it is intentionally retained as project CI.

### Task 1: Pure remaster contract and camera math

**Files:**
- Create: `crates/benilla-app/src/dev/m2studio.rs`
- Modify: `crates/benilla-app/src/dev.rs` only enough to declare `mod m2studio;` so the test module is compiled.
- Create: `.github/workflows/remaster-ci.yml`

**Interfaces:**
- Produces `RemasterSelection { model_path: String, placement_id: u32 }`.
- Produces `StudioView::{Front,Right,Back,Left}`, `StudioView::ALL`, `StudioView::file_name()` and `StudioView::forward()`.
- Produces `is_m2_path(&str) -> bool`.
- Produces `model_output_stem(&str) -> String` and `capture_output_dir(exe: &Path, model_path: &str) -> PathBuf`.
- Produces `StudioBounds { min: Vec3, max: Vec3 }` and `camera_distance(bounds, forward, vertical_fov, aspect) -> Option<f32>`.
- Produces `StudioRequest::from_env_values(model: Option<OsString>, out: Option<OsString>) -> Result<StudioRequest,String>` so parsing is testable without mutating process environment.

- [ ] **Step 1: Write failing tests before production helpers**

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_path_accepts_m2_and_mdx_case_insensitively() {
        assert!(is_m2_path(r"World\Tree.M2"));
        assert!(is_m2_path(r"World\Tree.mdx"));
        assert!(!is_m2_path(r"World\Tree.wmo"));
    }

    #[test]
    fn output_stem_is_stable_and_path_safe() {
        assert_eq!(
            model_output_stem(r"World\Generic\Tree\Oak01.m2"),
            "world_generic_tree_oak01_m2"
        );
    }

    #[test]
    fn canonical_views_have_exact_names_and_order() {
        let names: Vec<_> = StudioView::ALL.iter().map(|v| v.file_name()).collect();
        assert_eq!(names, ["front.png", "right.png", "back.png", "left.png"]);
    }

    #[test]
    fn request_requires_both_model_and_output_dir() {
        assert!(StudioRequest::from_env_values(None, None).is_err());
        assert!(StudioRequest::from_env_values(Some("a.m2".into()), None).is_err());
    }

    #[test]
    fn camera_fit_contains_a_wide_box_with_margin() {
        let b = StudioBounds { min: Vec3::new(-10.0, -1.0, -2.0), max: Vec3::new(10.0, 1.0, 2.0) };
        let d = camera_distance(&b, -Vec3::Z, 45_f32.to_radians(), 16.0 / 9.0).unwrap();
        assert!(d > 6.0);
        assert!(d.is_finite());
    }
}
```

- [ ] **Step 2: Push tests and run the branch CI to verify RED**

Run remotely:

```text
cargo test -p benilla-app m2studio --lib
```

Expected: FAIL because `is_m2_path`, `StudioView`, `StudioRequest`, `StudioBounds`, `camera_distance`, and output helpers do not exist yet.

- [ ] **Step 3: Implement the minimal pure helpers**

```rust
const FRAMING_MARGIN: f32 = 1.12;
const STUDIO_FOV: f32 = 45.0_f32.to_radians();

fn is_m2_path(path: &str) -> bool {
    let p = path.to_ascii_lowercase();
    p.ends_with(".m2") || p.ends_with(".mdx")
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StudioView { Front, Right, Back, Left }

impl StudioView {
    const ALL: [Self; 4] = [Self::Front, Self::Right, Self::Back, Self::Left];
    fn file_name(self) -> &'static str { /* exact four-name match */ }
    fn forward(self) -> Vec3 { /* camera-to-subject directions: -Z,-X,+Z,+X */ }
}
```

`camera_distance` must evaluate all eight AABB corners in the view basis and enforce both horizontal and vertical perspective constraints, then multiply the final distance by exactly `1.12`.

- [ ] **Step 4: Re-run focused tests and verify GREEN**

```text
cargo test -p benilla-app m2studio --lib
```

Expected: PASS.

- [ ] **Step 5: Commit**

```text
feat: add remaster capture core contract
```

### Task 2: Live Remaster Mode picker and child-process lifecycle

**Files:**
- Modify: `crates/benilla-app/src/dev/m2studio.rs`
- Modify: `crates/benilla-app/src/dev.rs`

**Interfaces:**
- Produces `pub(crate) struct RemasterPlugin`.
- Produces `RemasterState { enabled, selected, status }`.
- Produces `RemasterCaptureStatus::{Idle,Capturing,Complete,Failed}`.
- Uses existing `debug_panel::MouseoverTarget`, `WorldObject`, `ModelKind`, `InspectMode`, and egui context.
- Uses a non-send `RemasterChild` holder for `std::process::Child` so process ownership never constrains the ordinary `Resource` state.

- [ ] **Step 1: Add failing behavior tests**

```rust
#[test]
fn selection_accepts_only_doodad_m2s() {
    assert!(selection_from_object(ModelKind::Doodad, r"World\Tree.m2", 7).is_ok());
    assert!(selection_from_object(ModelKind::Wmo, r"World\House.wmo", 7).is_err());
    assert!(selection_from_object(ModelKind::Doodad, r"World\House.wmo", 7).is_err());
}

#[test]
fn capture_success_requires_exit_success_and_all_four_files() {
    let exists = |name: &str| name != "left.png";
    assert!(capture_completion(true, exists).is_err());
    assert!(capture_completion(false, |_| true).is_err());
    assert!(capture_completion(true, |_| true).is_ok());
}
```

Expected RED: helper behavior is absent.

- [ ] **Step 2: Implement live mode**

`Ctrl+Shift+R` toggles `RemasterState.enabled`. While enabled, set `InspectMode.enabled = true` so existing player-control arbitration gives the click to the developer tool; preserve/restore the previous inspector value when the mode toggles off. On a left press with a `MouseoverTarget`, resolve `WorldObject`, accept only `ModelKind::Doodad` plus M2/MDX path, and latch `RemasterSelection`.

The egui panel must display selected model path, status, and a `Capture 4 Views` / `Re-capture` button. Clicking it creates the output directory, launches:

```rust
Command::new(std::env::current_exe()?)
    .env("WOW_CAPTURE", "m2studio")
    .env("WOW_M2_MODEL", &selection.model_path)
    .env("WOW_M2_OUT_DIR", &output_dir)
    .env_remove("WOW_CAPTURE_OUT")
    .env_remove("WOW_CAPTURE_UI")
    .spawn()
```

Store the child in the non-send holder. An Update system calls `try_wait()` only; it never calls blocking `wait()`. Successful completion additionally requires all four expected files to exist.

- [ ] **Step 3: Register `RemasterPlugin` from `DevToolsPlugin`**

```rust
app.add_plugins(crate::debug_panel::DebugPanelPlugin)
    .add_plugins(m2studio::RemasterPlugin)
    .add_plugins(crate::perf::PerfPlugin)
```

- [ ] **Step 4: Run tests and player-seam build**

```text
cargo test -p benilla-app m2studio --lib
cargo build -p benilla --no-default-features
```

Expected: both PASS; player build contains no remaster module dependency.

- [ ] **Step 5: Commit**

```text
feat: add in-world remaster capture mode
```

### Task 3: Dedicated server-less M2 studio renderer

**Files:**
- Modify: `crates/benilla-app/src/dev/m2studio.rs`
- Modify: `crates/benilla-app/src/dev.rs`

**Interfaces:**
- Produces `pub(crate) fn capture_requested() -> bool` (`WOW_CAPTURE == "m2studio"`).
- Produces `pub(crate) struct M2StudioPlugin`.
- Consumes `StudioRequest`, `StudioView`, `StudioBounds`, camera-fit helpers from Task 1.
- Consumes public engine APIs `m2_url`, `M2Model`, `ModelForms::require_static/slices`, `terrain_stream::{m2_anim_bound,m2_fade,spawn_model_entities}`, `model_render::MaterialCache`, `lighting::LightBlob`, `CaptureMode`.

- [ ] **Step 1: Add failing state/geometry tests**

```rust
#[test]
fn bounds_union_tracks_all_extrema() {
    let mut b = StudioBounds::empty();
    b.include(Vec3::new(-2.0, 3.0, 1.0));
    b.include(Vec3::new(5.0, -4.0, 7.0));
    assert_eq!(b.min, Vec3::new(-2.0, -4.0, 1.0));
    assert_eq!(b.max, Vec3::new(5.0, 3.0, 7.0));
}

#[test]
fn each_view_places_the_camera_on_the_opposite_side_of_its_forward_vector() {
    let center = Vec3::new(1.0, 2.0, 3.0);
    let eye = camera_eye(center, StudioView::Front.forward(), 10.0);
    assert_eq!(eye, Vec3::new(1.0, 2.0, 13.0));
}
```

Expected RED until the helpers/state exist.

- [ ] **Step 2: Build a frozen neutral light buffer**

At studio startup create and write an engine-native off-world light blob:

```rust
let blob = LightBlob::model(
    [0.42, 0.42, 0.42],
    [0.78, 0.78, 0.78],
    Vec3::new(-0.35, -0.85, -0.40).normalize(),
);
let buffer = blob.create(&render_device, "m2studio-light");
blob.write(&render_queue, &buffer);
```

This is the fixed neutral studio light bound directly into the normal `WowModelMaterial`; do not add a Bevy `DirectionalLight` that the WoW shader does not read.

- [ ] **Step 3: Load and spawn the static subject through the production assembler**

Load `asset_server.load(m2_url(&request.model_path))`. Once the `M2Model` is resident, call `ModelForms::require_static(&handle, -1000)` until complete, then call `spawn_model_entities` with identity transform, `is_wmo=false`, `ShadeSel::Lit`, no interior slot, `m2=None`, no merge/static-gx/card-owner, and the frozen light buffer from Step 2.

`m2=None` is intentional v1 behavior: the selected class is a static remaster source, so the studio does not start an animation host or make view-to-view geometry time-dependent.

- [ ] **Step 4: Compute visual bounds after transform propagation**

After the spawned parts have `GlobalTransform`/`Aabb`, union the eight transformed corners of every subject part. Empty/degenerate bounds are a hard capture error. Use the AABB center as the look target.

- [ ] **Step 5: Add four-view image-stability shutter**

For each `StudioView`:

1. set world camera to the computed eye and `look_at(center, Vec3::Y)`;
2. reset stability state;
3. request unpersisted primary-window screenshots until RGB bytes match for the required stable-frame count;
4. request the persisted screenshot only after stability;
5. encode to the exact view filename with `image::ImageFormat::Png`;
6. advance to the next view only after the save observer succeeds.

No four-process fan-out is allowed.

- [ ] **Step 6: Force capture-only presentation state**

Insert `CaptureMode`, fixed manual time step, `ClearColor(Color::WHITE)`, disable the perf HUD, set perspective FOV to `STUDIO_FOV`, and leave networking disabled through the existing `run_mode::scenario_active()` behavior in `lib.rs`.

- [ ] **Step 7: Route `m2studio` in `DevProbesPlugin`**

```rust
if crate::run_mode::scenario_active() {
    if m2studio::capture_requested() {
        app.add_plugins(m2studio::M2StudioPlugin);
    } else {
        app.add_plugins(crate::capture::CapturePlugin);
    }
}
```

This is the only change to capture routing; existing capture scenarios still use the old plugin unchanged.

- [ ] **Step 8: Run focused tests, dev build, and player-seam build**

```text
cargo test -p benilla-app m2studio --lib
cargo check -p benilla-app
cargo build -p benilla --no-default-features
```

Expected: PASS.

- [ ] **Step 9: Commit**

```text
feat: render four-view M2 studio captures
```

### Task 4: Integration verification and cleanup

**Files:**
- Verify all changed files.
- Remove `.github/workflows/remaster-ci.yml` if it was only needed to execute branch-side TDD in this environment.

- [ ] **Step 1: Run repository-level checks available on CI**

```text
cargo fmt --all -- --check
cargo test -p benilla-app m2studio --lib
cargo check -p benilla-app
cargo build -p benilla --no-default-features
```

If the repository gate is feasible in the runner, additionally run `scripts/gates.sh`.

- [ ] **Step 2: Inspect branch diff against `main`**

Verify no generated PNG, WoW data, unrelated refactor, dependency addition, or existing capture modification is present.

- [ ] **Step 3: Manual acceptance on the Windows dev machine**

```powershell
$env:WOW_DATA = "D:\wow_reforged\World Of Warcraft Classic\Data"
cargo run --release -p benilla
```

Enter world → `Ctrl+Shift+R` → click a static tree/rock/prop → `Capture 4 Views`. Confirm live client remains responsive and exactly four PNGs appear in `target\remaster-captures\<model>\`, fully framed on white with consistent front/right/back/left views.

- [ ] **Step 4: Final commit if cleanup changed anything**

```text
chore: finalize remaster capture verification
```

## Self-review

- Spec coverage: selection, doodad/M2 validation, compact dev UI, nonblocking child launch, same executable, environment contract, deterministic asset folder, one-process four-view capture, production M2 rendering, neutral fixed lighting, white background, visual bounds, perspective framing, exact four names/order, error states, player seam, and output non-commit are all mapped to tasks.
- Placeholder scan: no TBD/TODO/"implement later" steps remain.
- Type consistency: `StudioRequest`, `StudioView`, `StudioBounds`, `RemasterSelection`, `RemasterPlugin`, and `M2StudioPlugin` are defined once and consumed under the same names in later tasks.
