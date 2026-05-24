use super::*;

/// Walk the cmd list and build one [`crate::render::flame_volume::FlameEmitter`] per Candle, in cmd
/// submission order so the matching `DrawCmd::Flame` batch loop downstream
/// consumes them in lockstep.
///
/// Each emitter sets:
/// - `wick_world` from `pixel_to_world` at the candle's wick tip,
/// - `scale` from the candle's physical scale (matches the previous 2D
///   flame's visual width),
/// - `wind` sampled from `frame.wind_gusts` with a soft falloff per gust,
/// - `brightness` and `phase` from the matching `DrawCmd::Flame` (the
///   scene pushes one `Flame` per candle; missing entries fall back to
///   `(1.0, 0.0)`).
pub(super) fn build_flame_emitters(
    frame: &UiFrame,
    w: f32,
    h: f32,
) -> Vec<crate::render::flame_volume::FlameEmitter> {
    if !frame.procedural_flame_emitters.is_empty() {
        return frame.procedural_flame_emitters.to_vec();
    }
    let mut out: Vec<crate::render::flame_volume::FlameEmitter> = Vec::new();
    // Candles in submission order.
    let candles: Vec<(&crate::render::draw_cmd::Object3d, f32, f32)> = frame
        .cmds
        .iter()
        .flat_map(|cmd| {
            let objs: Box<dyn Iterator<Item = &crate::render::draw_cmd::Object3d>> = match cmd {
                DrawCmd::Object3d(o) => Box::new(std::iter::once(o)),
                DrawCmd::Object3dBatch(v) => Box::new(v.iter()),
                _ => Box::new(std::iter::empty()),
            };
            objs.filter_map(|o| {
                if let crate::render::draw_cmd::Object3dKind::Candle {
                    scale,
                    height_scale,
                } = o.kind
                {
                    Some((o, scale, height_scale))
                } else {
                    None
                }
            })
            .collect::<Vec<_>>()
        })
        .collect();
    // Per-flame brightness + phase, pulled from the cmd stream in the
    // same order candles appear.
    let mut flame_cmd_iter = frame.cmds.iter().filter_map(|cmd| match cmd {
        DrawCmd::Flame(inst) => Some(*inst),
        _ => None,
    });
    for (o, p_scale, p_height) in candles.into_iter() {
        let p_pos = o.pos;
        let tip_world = pixel_to_world(
            w,
            h,
            p_pos[0],
            p_pos[1],
            crate::render::candle_mesh::WICK_TIP_Y * p_scale * p_height,
        );
        let scene_inst = flame_cmd_iter.next();
        let (brightness, phase) = scene_inst
            .map(|inst| (inst.color[2], inst.color[3]))
            .unwrap_or((1.0, 0.0));

        out.push(crate::render::flame_volume::FlameEmitter {
            wick_world: tip_world,
            scale: crate::render::flame_volume::flame_emitter_scale(p_scale, p_height),
            wind: glam::Vec2::ZERO,
            brightness,
            phase,
            flicker_amp: crate::render::flame_volume::FLAME_FLICKER_AMP,
        });
    }
    out.extend(frame.procedural_flame_emitters.iter().copied());
    out
}
