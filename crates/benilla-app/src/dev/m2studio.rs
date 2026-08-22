use std::ffi::OsString;
use std::path::PathBuf;

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
