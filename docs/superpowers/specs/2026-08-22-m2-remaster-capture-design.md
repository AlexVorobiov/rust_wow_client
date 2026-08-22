# M2 Remaster Capture Design

## Goal

Add a developer-only workflow that lets a developer select a static M2 doodad directly in the running Benilla world and generate a Tripo-ready four-view image set by launching Benilla's existing deterministic capture harness in an isolated studio scene.

The live game is only the asset picker and launcher. It must not take the Tripo screenshots from the live world.

## User flow

1. Run Benilla normally and enter the world.
2. Toggle **Remaster Mode** with the dev chord + `R`.
3. Click a visible static M2 doodad such as a tree, bush, rock, lamp, barrel, crate, or fence.
4. Benilla latches the selected `WorldObject` and shows a compact remaster panel with its M2 path.
5. Press **Capture 4 Views**.
6. The running process launches the same Benilla executable as a child process in a new capture mode.
7. The child process boots server-less, loads only that M2 through Benilla's normal M2 rendering path, places it in a white studio scene, auto-frames it, and captures four canonical views.
8. The parent process reports completion and the output directory.

## Scope

### In scope for v1

- Developer-only feature.
- Static world doodads whose `WorldObject.kind == ModelKind::Doodad`.
- Only M2 model paths.
- Existing triangle-accurate world picking.
- Persistent selected asset until another doodad is selected or Remaster Mode is disabled.
- Four PNGs: `front.png`, `right.png`, `back.png`, `left.png`.
- White capture background.
- Neutral, fixed studio lighting.
- Automatic framing from the loaded model's visual bounds.
- Child-process launch from the live game.
- Capture state surfaced in the in-game panel: idle / capturing / complete / failed.
- Output stored under `target/remaster-captures/`, which remains uncommitted.

### Explicit non-goals for v1

- WMO capture.
- Creatures, players, equipment, or GameObjects.
- Tripo API upload.
- GLB replacement or replacement-registry integration.
- Batch capture.
- Editing transforms or WoW world data.
- Capturing the object in its live world lighting/background.
- A Unity-style editor.

## Existing code we reuse

### Asset identity and picking

`benilla_world::interact::WorldObject` already carries the identity needed by this feature:

- `kind: ModelKind`
- `label: String`, documented as the model path for doodads/WMOs/GameObjects
- `id: u32`
- `detail: String`

The existing inspector already calls `pick_at_cursor(...)` and reads `WorldObject`. Picking is triangle-accurate against resident model geometry, so colliderless doodads are selectable. No new source-path component is required.

### Capture harness

The existing `$WOW_CAPTURE` harness already:

- disables ordinary network-driven play for capture runs;
- boots a deterministic/server-less capture path;
- controls the camera;
- waits for the rendered image to stabilize;
- captures the primary window to PNG;
- already has an `fxview` lane that loads an M2 via `AssetServer` / `m2_url` and orbits a camera around it.

The remaster capture must extend this harness rather than introduce a second renderer.

## Architecture

### 1. `remaster` developer feature in `benilla-app`

Create a small developer-only module that owns only live-world selection, UI state, and process launch.

Core state:

```rust
pub(crate) struct RemasterSelection {
    pub(crate) model_path: String,
    pub(crate) placement_id: u32,
}

pub(crate) enum RemasterCaptureStatus {
    Idle,
    Capturing,
    Complete { output_dir: PathBuf },
    Failed { message: String },
}
```

The feature must not change gameplay targeting. It consumes the same local picking foundation as the existing inspector.

### 2. Remaster Mode interaction

Use the existing dev-key plane. The v1 toggle is **dev chord + `R`**, not bare `F7`, so it follows the repository's existing rule that developer affordances stay off the player's normal binding plane.

When Remaster Mode is enabled:

- the panel is visible;
- a world left-click may select a `ModelKind::Doodad` under the cursor;
- non-doodad hits are ignored with a short panel message;
- normal gameplay behaviour outside Remaster Mode is unchanged.

Selection should reuse `pick_at_cursor(...)`/`PickParts` rather than add physics colliders or a second ray-caster.

### 3. Capture launch contract

The parent launches `std::env::current_exe()` so the child is guaranteed to run the same build the developer is currently looking at. This also avoids the existing manual-capture hazard of invoking a stale binary.

The parent passes an environment contract consistent with the current capture harness:

```text
WOW_CAPTURE=m2studio
WOW_M2_MODEL=<internal M2 path>
WOW_M2_OUT_DIR=<absolute output directory>
```

No JSON IPC is added in v1. The contract has only two payload values and the repository already uses environment variables for capture fixtures. Adding a request file would add serialization/versioning without solving a current need.

`WOW_DATA`, when explicitly configured in the parent environment, is inherited by the child normally.

### 4. Output directory

For a selected model path, generate a filesystem-safe deterministic stem from the path plus placement-independent identity. The same M2 path should map to the same asset folder; two placements of the same tree are the same remaster source asset.

Example:

```text
target/remaster-captures/world_generic_passivedoodads_tree_oak01/
    front.png
    right.png
    back.png
    left.png
```

A re-capture replaces the four files in that asset folder. Blizzard-derived screenshots remain under `target/` and are never committed.

### 5. `m2studio` capture mode

Add a dedicated capture fixture instead of overloading `fxview` semantics. It may reuse the same M2 attach/render helpers, but its contract and lifecycle are different:

- exactly one static M2 subject;
- no terrain, world streaming, player, UI, fog, weather, or sky;
- white clear/background color;
- fixed studio light rig;
- no network session;
- one loaded model reused across all four views;
- four screenshots generated in one child-process invocation.

The fixture must load the subject through the same Benilla M2 asset/material path used by the client. It must not convert the model to another format for capture.

### 6. Studio lighting

The capture is intended as reconstruction reference material, not a beauty shot. Lighting must reveal shape while preserving the source texture colors:

- white background;
- fixed neutral key light;
- weaker neutral fill light from the opposite side;
- low neutral ambient contribution;
- no world day/night/weather state;
- fixed exposure.

All four views use the same light rig.

### 7. Visual bounds and automatic camera fit

A fixed camera distance is invalid because M2 doodads vary from small props to large trees.

After the model is fully attached, compute a world-space AABB for the visible subject by unioning the bounds of its rendered model parts. The capture target is the AABB center, not a hard-coded `root + Vec3::Y` point.

For each view, rotate the camera around the AABB center and derive the distance from:

- the view-space half-width and half-height of the subject bounds;
- the camera vertical FOV;
- the current capture aspect ratio;
- a fixed framing multiplier of **1.12** (12% margin).

Use perspective projection so the reference set matches how the model is normally viewed, but keep the same FOV and framing rule for all four shots.

If usable bounds cannot be produced after the model has loaded, fail the capture explicitly instead of emitting misleading blank/cropped images.

### 8. Canonical views

The four views are horizontal product-reference views around the model's authored orientation:

| File | Azimuth |
| --- | ---: |
| `front.png` | 0° |
| `right.png` | 90° |
| `back.png` | 180° |
| `left.png` | 270° |

Elevation is **0°** in v1. All cameras look at the bounds center. This keeps the set geometrically useful for multi-view reconstruction and avoids inventing an arbitrary elevated viewpoint.

The model itself is not rotated between shots; only the camera moves.

### 9. Four-shot state machine

The existing harness's single-shot stabilization rule remains the shutter gate. `m2studio` adds a small sequence above it:

1. load/attach subject;
2. wait for valid stable visual bounds;
3. place camera at Front;
4. wait for the image to stabilize;
5. capture `front.png`;
6. repeat stabilization + capture for Right, Back, Left;
7. exit success only after all four screenshot callbacks complete.

Do not launch four Benilla processes. One process and one loaded model eliminate redundant loading and keep pose/material state identical across views.

### 10. Parent/child status

The parent keeps a non-blocking child handle and polls it from the Bevy update loop.

- Exit code 0 + all four expected PNGs present => `Complete`.
- Non-zero exit => `Failed` with the exit code.
- Spawn error => `Failed` immediately.
- A second capture button press while a child is active is ignored/disabled.

The live game must never block its render/update loop waiting for the capture child.

## UI

Keep v1 deliberately compact and use the existing developer-overlay/egui visual language.

Panel content:

```text
REMASTER CAPTURE
Mode: ON

Selected M2
World\\Generic\\...\\Oak01.m2

[ Capture 4 Views ]

Status: Capturing…
```

On completion:

```text
Status: Complete
<output path>
[ Re-capture ]
```

Image thumbnails and an `Open Folder` button are useful follow-ups but are not required to prove the v1 pipeline.

## Error handling

The panel must surface these failures without crashing the live client:

- no doodad selected;
- selected `WorldObject.label` is not an M2/MDX model path;
- child process spawn failure;
- missing/invalid `WOW_M2_MODEL` in child;
- model asset load failure;
- no valid visual bounds;
- screenshot write failure;
- child process exits unsuccessfully;
- capture child succeeds but one or more expected PNG files are missing.

The child logs a specific error and exits non-zero for fixture failures.

## Testing strategy

Implementation follows TDD.

### Pure/unit tests

- doodad selection accepts `ModelKind::Doodad` and rejects WMO/Creature/GameObject;
- M2 path validation accepts `.m2` and legacy `.mdx` paths used by the asset layer, case-insensitively;
- output folder sanitization is stable and placement-independent;
- canonical view order and output names are exactly Front/Right/Back/Left;
- camera-fit math contains wide, tall, and tiny AABBs with the 1.12 margin;
- child launch specification uses `current_exe`, `WOW_CAPTURE=m2studio`, model path and output directory;
- capture request parsing rejects missing model/output values;
- parent status only reports success when process exit is successful and all four PNGs exist.

### Existing regression suite

Run the relevant crate tests first, then the workspace suite expected by the repository's normal gate. No existing capture scenario or ordinary gameplay targeting behaviour may change.

### Manual acceptance

1. Enter a world containing a static M2 tree.
2. Enable Remaster Mode.
3. Click the tree crown or trunk.
4. Confirm the panel displays the tree's internal M2 path.
5. Capture four views.
6. Confirm the live client remains responsive while capture runs.
7. Confirm exactly four correctly named PNGs are written.
8. Confirm each image has a white background, the whole model is visible with consistent framing, and the cameras correspond to front/right/back/left.
9. Disable Remaster Mode and confirm ordinary clicks behave as before.

## Compatibility and invariants

- The feature is compiled only with the existing developer feature plane.
- `benilla-world` remains game-agnostic; remaster UI/process orchestration belongs in `benilla-app`.
- No new dependency is required for v1.
- No WoW data or generated screenshot is committed.
- No server protocol or gameplay targeting changes are introduced.
- Original capture scenarios remain valid.
- Original M2 assets remain the rendering source; this feature produces reference images only.
