use super::*;

impl WgpuRenderer {
    fn scene_path(&self, suffix: &str) -> String {
        match self.active_scene_key {
            Some(scene) => format!("{scene}.{suffix}"),
            None => suffix.to_string(),
        }
    }

    /// If an arrange override is active and `name` matches, apply the
    /// accumulated position and rotation deltas to `model` and return the
    /// modified matrix. The override is expressed as layout-pixel deltas so
    /// it remains layout-relative across window resizes:
    ///
    /// - Translation: `(delta_px, -delta_py, delta_lift)` in world space
    ///   (pixel_x maps 1:1 to world_x; pixel_y maps 1:1 to −world_y).
    /// - Rotation: a `Rz(delta_rz) * Rx(delta_rx)` matrix is left-multiplied
    ///   onto the original 3×3 rotation+scale block, so the delta rotates the
    ///   object in world space on top of whatever convention the placement uses.
    ///
    /// Returns the matrix unchanged if no override is set or the name doesn't
    /// match.
    fn apply_arrange_override(&self, name: &str, model: Mat4) -> Mat4 {
        // Fuse the committed rotation (from the Placement) with any staged
        // arrange-mode rotation delta into a single Euler-angle sum before
        // left-multiplying onto the model. This matters because rotations
        // don't commute: applying `R_delta * R_committed` separately would
        // visually jump at Enter-time (when the delta folds into committed
        // via Euler addition). Summing first keeps preview == commit.
        let committed = self.committed_arrange_rotations.get(name).copied();
        let staged = self
            .debug_arrange_override
            .as_ref()
            .filter(|ov| ov.name == name);
        let (rx, ry, rz) = {
            let [crx, cry, crz] = committed.unwrap_or([0.0, 0.0, 0.0]);
            let (drx, dry, drz) = staged
                .map(|ov| (ov.delta_rx_deg, ov.delta_ry_deg, ov.delta_rz_deg))
                .unwrap_or((0.0, 0.0, 0.0));
            (crx + drx, cry + dry, crz + drz)
        };
        let mut model = if rx != 0.0 || ry != 0.0 || rz != 0.0 {
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
        } else {
            model
        };
        // Translation delta (only applies while a delta is staged for this name).
        if let Some(ov) = staged {
            let dt = glam::Vec3::new(ov.delta_px, -ov.delta_py, ov.delta_lift);
            let t = model.w_axis.truncate() + dt;
            model = Mat4::from_cols(model.x_axis, model.y_axis, model.z_axis, t.extend(1.0));
        }
        model
    }
}
