use std::ffi::OsString;
use std::path::{Path, PathBuf};

const FRAMING_MARGIN: f32 = 1.12;

fn is_m2_path(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".m2") || lower.ends_with(".mdx")
}

fn model_output_stem(model_path: &str) -> String {
    let mut out = String::with_capacity(model_path.len());
    let mut separator = false;

    for ch in model_path.chars() {
        if ch.is_ascii_alphanumeric() {
            if separator && !out.is_empty() {
                out.push('_');
            }
            out.push(ch.to_ascii_lowercase());
            separator = false;
        } else {
            separator = true;
        }
    }

    if out.is_empty() {
        "model".to_string()
    } else {
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum StudioView {
    Front,
    Right,
    Back,
    Left,
}

impl StudioView {
    const ALL: [Self; 4] = [Self::Front, Self::Right, Self::Back, Self::Left];

    fn file_name(self) -> &'static str {
        match self {
            Self::Front => "front.png",
            Self::Right => "right.png",
            Self::Back => "back.png",
            Self::Left => "left.png",
        }
    }

    fn forward(self) -> [f32; 3] {
        match self {
            Self::Front => [0.0, 0.0, -1.0],
            Self::Right => [-1.0, 0.0, 0.0],
            Self::Back => [0.0, 0.0, 1.0],
            Self::Left => [1.0, 0.0, 0.0],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct StudioRequest {
    model_path: String,
    out_dir: PathBuf,
}

impl StudioRequest {
    fn from_env_values(model: Option<OsString>, out: Option<OsString>) -> Result<Self, String> {
        let model_path = model
            .ok_or_else(|| "WOW_M2_MODEL is required".to_string())?
            .into_string()
            .map_err(|_| "WOW_M2_MODEL must be valid UTF-8".to_string())?;
        if !is_m2_path(&model_path) {
            return Err(format!("WOW_M2_MODEL is not an M2/MDX path: {model_path}"));
        }
        let out_dir = out
            .ok_or_else(|| "WOW_M2_OUT_DIR is required".to_string())?
            .into();
        Ok(Self {
            model_path,
            out_dir,
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct RemasterSelection {
    model_path: String,
    placement_id: u32,
}

fn selection_from_parts(
    is_doodad: bool,
    model_path: &str,
    placement_id: u32,
) -> Result<RemasterSelection, String> {
    if !is_doodad {
        return Err("Remaster Capture v1 accepts static doodads only".to_string());
    }
    if !is_m2_path(model_path) {
        return Err(format!("not an M2/MDX doodad: {model_path}"));
    }
    Ok(RemasterSelection {
        model_path: model_path.to_string(),
        placement_id,
    })
}

fn capture_output_dir(exe: &Path, model_path: &str) -> PathBuf {
    let target = exe
        .parent()
        .and_then(Path::parent)
        .filter(|dir| dir.file_name().is_some_and(|name| name == "target"))
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("target"));
    target
        .join("remaster-captures")
        .join(model_output_stem(model_path))
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct StudioBounds {
    min: [f32; 3],
    max: [f32; 3],
}

impl StudioBounds {
    fn empty() -> Self {
        Self {
            min: [f32::INFINITY; 3],
            max: [f32::NEG_INFINITY; 3],
        }
    }

    fn include(&mut self, point: [f32; 3]) {
        for axis in 0..3 {
            self.min[axis] = self.min[axis].min(point[axis]);
            self.max[axis] = self.max[axis].max(point[axis]);
        }
    }

    fn is_valid(&self) -> bool {
        (0..3).all(|axis| {
            self.min[axis].is_finite()
                && self.max[axis].is_finite()
                && self.max[axis] > self.min[axis]
        })
    }

    fn center(&self) -> [f32; 3] {
        [
            (self.min[0] + self.max[0]) * 0.5,
            (self.min[1] + self.max[1]) * 0.5,
            (self.min[2] + self.max[2]) * 0.5,
        ]
    }

    fn corners(&self) -> [[f32; 3]; 8] {
        let [x0, y0, z0] = self.min;
        let [x1, y1, z1] = self.max;
        [
            [x0, y0, z0],
            [x0, y0, z1],
            [x0, y1, z0],
            [x0, y1, z1],
            [x1, y0, z0],
            [x1, y0, z1],
            [x1, y1, z0],
            [x1, y1, z1],
        ]
    }
}

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [
        a[1] * b[2] - a[2] * b[1],
        a[2] * b[0] - a[0] * b[2],
        a[0] * b[1] - a[1] * b[0],
    ]
}

fn normalize(v: [f32; 3]) -> Option<[f32; 3]> {
    let len_sq = dot(v, v);
    if !len_sq.is_finite() || len_sq <= f32::EPSILON {
        return None;
    }
    let inv = len_sq.sqrt().recip();
    Some([v[0] * inv, v[1] * inv, v[2] * inv])
}

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn camera_distance(
    bounds: &StudioBounds,
    forward: [f32; 3],
    vertical_fov: f32,
    aspect: f32,
) -> Option<f32> {
    if !bounds.is_valid()
        || !vertical_fov.is_finite()
        || vertical_fov <= 0.0
        || vertical_fov >= std::f32::consts::PI
        || !aspect.is_finite()
        || aspect <= 0.0
    {
        return None;
    }

    let forward = normalize(forward)?;
    let right = normalize(cross(forward, [0.0, 1.0, 0.0]))?;
    let up = normalize(cross(right, forward))?;
    let tan_v = (vertical_fov * 0.5).tan();
    let tan_h = tan_v * aspect;
    let center = bounds.center();
    let mut distance = 0.0f32;

    for corner in bounds.corners() {
        let rel = sub(corner, center);
        let x = dot(rel, right).abs();
        let y = dot(rel, up).abs();
        let z = dot(rel, forward);
        let required = (x / tan_h).max(y / tan_v) - z;
        distance = distance.max(required);
    }

    let distance = distance * FRAMING_MARGIN;
    (distance.is_finite() && distance > 0.0).then_some(distance)
}

fn camera_eye(center: [f32; 3], forward: [f32; 3], distance: f32) -> [f32; 3] {
    [
        center[0] - forward[0] * distance,
        center[1] - forward[1] * distance,
        center[2] - forward[2] * distance,
    ]
}

fn capture_completion(
    exit_success: bool,
    mut exists: impl FnMut(&str) -> bool,
) -> Result<(), String> {
    if !exit_success {
        return Err("capture child exited unsuccessfully".to_string());
    }
    let missing: Vec<_> = StudioView::ALL
        .iter()
        .map(|view| view.file_name())
        .filter(|name| !exists(name))
        .collect();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(format!("capture child omitted: {}", missing.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

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
        assert_eq!(
            names,
            ["front.png", "right.png", "back.png", "left.png"]
        );
        assert_eq!(StudioView::Front.forward(), [0.0, 0.0, -1.0]);
        assert_eq!(StudioView::Right.forward(), [-1.0, 0.0, 0.0]);
        assert_eq!(StudioView::Back.forward(), [0.0, 0.0, 1.0]);
        assert_eq!(StudioView::Left.forward(), [1.0, 0.0, 0.0]);
    }

    #[test]
    fn request_requires_both_model_and_output_dir() {
        assert!(StudioRequest::from_env_values(None, None).is_err());
        assert!(StudioRequest::from_env_values(Some("a.m2".into()), None).is_err());
        assert!(StudioRequest::from_env_values(None, Some("out".into())).is_err());
    }

    #[test]
    fn request_rejects_a_non_m2_model() {
        let result = StudioRequest::from_env_values(
            Some("World/house.wmo".into()),
            Some("target/remaster".into()),
        );
        assert!(result.is_err());
    }

    #[test]
    fn selection_accepts_only_doodad_m2s() {
        assert!(selection_from_parts(true, r"World\Tree.m2", 7).is_ok());
        assert!(selection_from_parts(false, r"World\Tree.m2", 7).is_err());
        assert!(selection_from_parts(true, r"World\House.wmo", 7).is_err());
    }

    #[test]
    fn output_dir_is_anchored_to_the_executables_target_directory() {
        let dir = capture_output_dir(
            Path::new("/repo/target/debug/benilla"),
            r"World\Generic\Tree\Oak01.m2",
        );
        assert_eq!(
            dir,
            Path::new("/repo/target/remaster-captures/world_generic_tree_oak01_m2")
        );
    }

    #[test]
    fn bounds_union_tracks_all_extrema() {
        let mut b = StudioBounds::empty();
        b.include([-2.0, 3.0, 1.0]);
        b.include([5.0, -4.0, 7.0]);
        assert_eq!(b.min, [-2.0, -4.0, 1.0]);
        assert_eq!(b.max, [5.0, 3.0, 7.0]);
        assert_eq!(b.center(), [1.5, -0.5, 4.0]);
    }

    #[test]
    fn camera_fit_contains_wide_and_tall_boxes_with_margin() {
        let wide = StudioBounds {
            min: [-10.0, -1.0, -2.0],
            max: [10.0, 1.0, 2.0],
        };
        let tall = StudioBounds {
            min: [-1.0, -12.0, -2.0],
            max: [1.0, 12.0, 2.0],
        };
        let fov = 45_f32.to_radians();
        let wide_d = camera_distance(&wide, [0.0, 0.0, -1.0], fov, 16.0 / 9.0).unwrap();
        let tall_d = camera_distance(&tall, [0.0, 0.0, -1.0], fov, 16.0 / 9.0).unwrap();
        assert!(wide_d > 13.0, "wide box distance: {wide_d}");
        assert!(tall_d > 32.0, "tall box distance: {tall_d}");
    }

    #[test]
    fn camera_eye_is_opposite_the_forward_vector() {
        assert_eq!(
            camera_eye([1.0, 2.0, 3.0], [0.0, 0.0, -1.0], 10.0),
            [1.0, 2.0, 13.0]
        );
    }

    #[test]
    fn completion_requires_success_and_all_four_images() {
        assert!(capture_completion(false, |_| true).is_err());
        assert!(capture_completion(true, |name| name != "left.png").is_err());
        assert!(capture_completion(true, |_| true).is_ok());
    }
}
