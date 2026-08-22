#[cfg(test)]
mod tests {
    use super::*;
    use bevy::prelude::*;

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
    fn camera_fit_contains_a_wide_box_with_margin() {
        let b = StudioBounds {
            min: Vec3::new(-10.0, -1.0, -2.0),
            max: Vec3::new(10.0, 1.0, 2.0),
        };
        let d = camera_distance(&b, -Vec3::Z, 45_f32.to_radians(), 16.0 / 9.0).unwrap();
        assert!(d > 6.0);
        assert!(d.is_finite());
    }

    #[test]
    fn bounds_union_tracks_all_extrema() {
        let mut b = StudioBounds::empty();
        b.include(Vec3::new(-2.0, 3.0, 1.0));
        b.include(Vec3::new(5.0, -4.0, 7.0));
        assert_eq!(b.min, Vec3::new(-2.0, -4.0, 1.0));
        assert_eq!(b.max, Vec3::new(5.0, 3.0, 7.0));
    }

    #[test]
    fn each_view_places_camera_opposite_its_forward_vector() {
        let center = Vec3::new(1.0, 2.0, 3.0);
        let eye = camera_eye(center, StudioView::Front.forward(), 10.0);
        assert_eq!(eye, Vec3::new(1.0, 2.0, 13.0));
    }
}
