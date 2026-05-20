use super::*;

impl WgpuRenderer {
    pub(crate) fn scene_path(&self, suffix: &str) -> String {
        match self.active_scene_key {
            Some(scene) => format!("{scene}.{suffix}"),
            None => suffix.to_string(),
        }
    }

    /// Apply committed placement rotation degrees to `model` when `name` matches.
    pub(crate) fn apply_placement_rotation(&self, name: &str, model: Mat4) -> Mat4 {
        let Some([rx, ry, rz]) = self.placement_rotations.get(name).copied() else {
            return model;
        };
        if rx == 0.0 && ry == 0.0 && rz == 0.0 {
            return model;
        }
        let r = Mat4::from_rotation_z(rz.to_radians())
            * Mat4::from_rotation_y(ry.to_radians())
            * Mat4::from_rotation_x(rx.to_radians());
        let t = model.w_axis.truncate();
        let nx = r.transform_vector3(model.x_axis.truncate());
        let ny = r.transform_vector3(model.y_axis.truncate());
        let nz = r.transform_vector3(model.z_axis.truncate());
        Mat4::from_cols(
            nx.extend(0.0),
            ny.extend(0.0),
            nz.extend(0.0),
            t.extend(1.0),
        )
    }
}
