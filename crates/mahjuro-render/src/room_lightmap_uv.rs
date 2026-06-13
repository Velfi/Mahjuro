use std::collections::HashMap;

use glam::{Vec2, Vec3};

use crate::tile_glb::{LoadedPrimitive, Vertex3dTex};

#[repr(C)]
#[derive(Clone, Copy, Debug, bytemuck::Pod, bytemuck::Zeroable)]
pub struct RoomEnvLightmapVertex {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub tangent: [f32; 4],
    pub uv_emr: [f32; 2],
    pub color: [f32; 4],
    pub lightmap_uv: [f32; 2],
}

impl RoomEnvLightmapVertex {
    #[inline]
    fn from_source(v: Vertex3dTex, lightmap_uv: [f32; 2]) -> Self {
        Self {
            position: v.position,
            normal: v.normal,
            uv: v.uv,
            tangent: v.tangent,
            uv_emr: v.uv_emr,
            color: v.color,
            lightmap_uv,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RoomLightmapTriangle {
    pub source_indices: [usize; 3],
    pub lightmap_uv: [[f32; 2]; 3],
}

pub struct RoomEnvLightmapGpuMesh {
    pub vertices: Vec<RoomEnvLightmapVertex>,
    pub indices: Vec<u32>,
}

pub fn room_lightmap_triangles(mesh: &LoadedPrimitive) -> Vec<RoomLightmapTriangle> {
    let source_tris = valid_source_triangles(mesh);
    if source_tris.is_empty() {
        return Vec::new();
    }
    let projected = projected_lightmap_triangles(mesh, &source_tris);
    if lightmap_triangles_cover_minimum_chart_texel(&projected) {
        projected
    } else {
        per_triangle_island_lightmap_triangles(&source_tris)
    }
}

fn projected_lightmap_triangles(
    mesh: &LoadedPrimitive,
    source_tris: &[[usize; 3]],
) -> Vec<RoomLightmapTriangle> {
    let tri_buckets = source_tris
        .iter()
        .map(|&ids| ProjectionBucket::from_triangle(mesh, ids))
        .collect::<Vec<_>>();
    let tri_chart_indices = connected_projection_charts(mesh, source_tris, &tri_buckets);
    let chart_count = tri_chart_indices
        .iter()
        .copied()
        .max()
        .map(|i| i + 1)
        .unwrap_or(0);
    if chart_count == 0 {
        return per_triangle_island_lightmap_triangles(source_tris);
    }
    let mut charts = vec![ProjectionChart::empty(); chart_count];
    for ((&ids, &chart_idx), &bucket) in
        source_tris.iter().zip(&tri_chart_indices).zip(&tri_buckets)
    {
        let chart = &mut charts[chart_idx];
        chart.bucket = bucket;
        chart.area += triangle_area(mesh, ids);
        for &id in &ids {
            let p = Vec3::from_array(mesh.vertices[id].position);
            chart.bounds.include(bucket.project(p));
        }
    }
    let active = charts
        .iter()
        .enumerate()
        .filter_map(|(i, chart)| chart.bounds.valid().then_some(i))
        .collect::<Vec<_>>();
    if active.is_empty() {
        return per_triangle_island_lightmap_triangles(source_tris);
    }
    pack_projection_charts(&mut charts, &active);
    source_tris
        .into_iter()
        .zip(tri_chart_indices)
        .map(|(&source_indices, chart_idx)| {
            let chart = charts[chart_idx];
            let lightmap_uv = source_indices.map(|id| {
                let p = Vec3::from_array(mesh.vertices[id].position);
                let uv = chart.bounds.normalize(chart.bucket.project(p));
                chart.rect.map(uv)
            });
            RoomLightmapTriangle {
                source_indices,
                lightmap_uv,
            }
        })
        .collect()
}

pub fn build_room_env_lightmap_gpu_mesh(mesh: &LoadedPrimitive) -> RoomEnvLightmapGpuMesh {
    let tris = room_lightmap_triangles(mesh);
    let mut vertices = Vec::with_capacity(tris.len().saturating_mul(3));
    let mut indices = Vec::with_capacity(tris.len().saturating_mul(3));
    for tri in tris {
        for corner in 0..3 {
            let Some(&v) = mesh.vertices.get(tri.source_indices[corner]) else {
                continue;
            };
            let idx = vertices.len();
            if idx > u32::MAX as usize {
                break;
            }
            vertices.push(RoomEnvLightmapVertex::from_source(
                v,
                tri.lightmap_uv[corner],
            ));
            indices.push(idx as u32);
        }
    }
    RoomEnvLightmapGpuMesh { vertices, indices }
}

fn valid_source_triangles(mesh: &LoadedPrimitive) -> Vec<[usize; 3]> {
    let mut tris = Vec::new();
    if mesh.indices.is_empty() {
        let mut i = 0usize;
        while i + 2 < mesh.vertices.len() {
            let ids = [i, i + 1, i + 2];
            if source_triangle_is_valid(mesh, ids) {
                tris.push(ids);
            }
            i += 3;
        }
        return tris;
    }
    for idx in mesh.indices.chunks_exact(3) {
        let ids = [idx[0] as usize, idx[1] as usize, idx[2] as usize];
        if ids.iter().any(|&i| i >= mesh.vertices.len()) {
            continue;
        }
        if source_triangle_is_valid(mesh, ids) {
            tris.push(ids);
        }
    }
    tris
}

fn source_triangle_is_valid(mesh: &LoadedPrimitive, ids: [usize; 3]) -> bool {
    let p = [
        Vec3::from_array(mesh.vertices[ids[0]].position),
        Vec3::from_array(mesh.vertices[ids[1]].position),
        Vec3::from_array(mesh.vertices[ids[2]].position),
    ];
    if p.iter().any(|p| !p.is_finite()) {
        return false;
    }
    let area2 = (p[1] - p[0]).cross(p[2] - p[0]).length_squared();
    area2.is_finite() && area2 > 1.0e-12
}

fn triangle_area(mesh: &LoadedPrimitive, ids: [usize; 3]) -> f32 {
    let p = [
        Vec3::from_array(mesh.vertices[ids[0]].position),
        Vec3::from_array(mesh.vertices[ids[1]].position),
        Vec3::from_array(mesh.vertices[ids[2]].position),
    ];
    let area = (p[1] - p[0]).cross(p[2] - p[0]).length() * 0.5;
    if area.is_finite() { area.max(0.0) } else { 0.0 }
}

fn per_triangle_island_lightmap_triangles(source_tris: &[[usize; 3]]) -> Vec<RoomLightmapTriangle> {
    let tri_count = source_tris.len();
    let columns = ceil_sqrt_usize(tri_count).max(1);
    let rows = tri_count.div_ceil(columns).max(1);
    source_tris
        .iter()
        .enumerate()
        .map(|(tri_idx, &source_indices)| RoomLightmapTriangle {
            source_indices,
            lightmap_uv: triangle_island_uv(tri_idx, columns, rows),
        })
        .collect()
}

fn triangle_island_uv(tri_idx: usize, columns: usize, rows: usize) -> [[f32; 2]; 3] {
    let rect = chart_cell_uv(tri_idx, columns, rows);
    [
        rect.map(Vec2::new(0.0, 0.0)),
        rect.map(Vec2::new(1.0, 0.0)),
        rect.map(Vec2::new(0.0, 1.0)),
    ]
}

fn lightmap_triangles_cover_minimum_chart_texel(tris: &[RoomLightmapTriangle]) -> bool {
    let side = ceil_sqrt_usize(tris.len().max(1)).saturating_mul(2).max(6) as u32;
    tris.iter()
        .any(|tri| lightmap_triangle_covers_texel(tri.lightmap_uv, side))
}

fn lightmap_triangle_covers_texel(uv: [[f32; 2]; 3], side: u32) -> bool {
    let side = side.max(1);
    let p = [
        lightmap_test_uv_to_pixel(Vec2::from_array(uv[0]), side),
        lightmap_test_uv_to_pixel(Vec2::from_array(uv[1]), side),
        lightmap_test_uv_to_pixel(Vec2::from_array(uv[2]), side),
    ];
    if (p[1] - p[0]).perp_dot(p[2] - p[0]).abs() < 1.0e-5 {
        return false;
    }
    let max_coord = side as i32 - 1;
    let min_x = p
        .iter()
        .map(|v| v.x.floor() as i32)
        .min()
        .unwrap_or(0)
        .clamp(0, max_coord);
    let max_x = p
        .iter()
        .map(|v| v.x.ceil() as i32)
        .max()
        .unwrap_or(0)
        .clamp(0, max_coord);
    let min_y = p
        .iter()
        .map(|v| v.y.floor() as i32)
        .min()
        .unwrap_or(0)
        .clamp(0, max_coord);
    let max_y = p
        .iter()
        .map(|v| v.y.ceil() as i32)
        .max()
        .unwrap_or(0)
        .clamp(0, max_coord);
    for y in min_y..=max_y {
        for x in min_x..=max_x {
            let center = Vec2::new(x as f32 + 0.5, y as f32 + 0.5);
            let Some(bary) = barycentric_2d(center, p[0], p[1], p[2]) else {
                continue;
            };
            if bary.min_element() >= -0.001 {
                return true;
            }
        }
    }
    false
}

fn lightmap_test_uv_to_pixel(uv: Vec2, side: u32) -> Vec2 {
    let side_m1 = side.saturating_sub(1).max(1) as f32;
    Vec2::splat(0.5) + uv.clamp(Vec2::ZERO, Vec2::ONE) * side_m1
}

fn barycentric_2d(p: Vec2, a: Vec2, b: Vec2, c: Vec2) -> Option<Vec3> {
    let v0 = b - a;
    let v1 = c - a;
    let v2 = p - a;
    let d00 = v0.dot(v0);
    let d01 = v0.dot(v1);
    let d11 = v1.dot(v1);
    let d20 = v2.dot(v0);
    let d21 = v2.dot(v1);
    let denom = d00 * d11 - d01 * d01;
    if denom.abs() < 1.0e-8 || !denom.is_finite() {
        return None;
    }
    let v = (d11 * d20 - d01 * d21) / denom;
    let w = (d00 * d21 - d01 * d20) / denom;
    let u = 1.0 - v - w;
    Some(Vec3::new(u, v, w))
}

fn chart_cell_uv(idx: usize, columns: usize, rows: usize) -> ChartUvRect {
    let col = idx % columns;
    let row = idx / columns;
    let cell_w = 1.0 / columns as f32;
    let cell_h = 1.0 / rows as f32;
    let x0 = col as f32 * cell_w;
    let y0 = row as f32 * cell_h;
    let pad_x = cell_w * 0.05;
    let pad_y = cell_h * 0.05;
    ChartUvRect {
        min: Vec2::new(x0 + pad_x, y0 + pad_y),
        max: Vec2::new(x0 + cell_w - pad_x, y0 + cell_h - pad_y),
    }
}

fn pack_projection_charts(charts: &mut [ProjectionChart], active: &[usize]) {
    const ATLAS_SIZE: u32 = 4096;
    const PADDING: u32 = 4;
    const OCCUPANCY: f32 = 0.74;

    let total_projected_area = active
        .iter()
        .map(|&i| charts[i].projected_area().max(0.0))
        .sum::<f32>();
    let active_count = active.len().max(1) as f32;
    let target_area = (ATLAS_SIZE as f32 * ATLAS_SIZE as f32) * OCCUPANCY;
    let base_scale = if total_projected_area > 1.0e-8 {
        (target_area / total_projected_area).sqrt()
    } else {
        ATLAS_SIZE as f32 / active_count.sqrt()
    };
    let mut scale = base_scale;
    for _ in 0..32 {
        let mut requests = active
            .iter()
            .map(|&chart_idx| ChartPackRequest::from_chart(chart_idx, charts[chart_idx], scale))
            .collect::<Vec<_>>();
        if let Some(rects) = pack_chart_requests(&mut requests, ATLAS_SIZE, PADDING) {
            for rect in rects {
                charts[rect.chart_idx].rect = rect.uv_rect(ATLAS_SIZE);
            }
            return;
        }
        scale *= 0.88;
    }

    let columns = ceil_sqrt_usize(active.len()).max(1);
    let rows = active.len().div_ceil(columns).max(1);
    for (rank, &chart_idx) in active.iter().enumerate() {
        charts[chart_idx].rect = chart_cell_uv(rank, columns, rows);
    }
}

#[derive(Clone, Copy, Debug)]
struct ChartPackRequest {
    chart_idx: usize,
    w: u32,
    h: u32,
}

impl ChartPackRequest {
    fn from_chart(chart_idx: usize, chart: ProjectionChart, scale: f32) -> Self {
        let span = chart.bounds.span_abs();
        let fallback = chart.area.max(1.0e-6).sqrt();
        let sx = if span.x > 1.0e-6 { span.x } else { fallback };
        let sy = if span.y > 1.0e-6 { span.y } else { fallback };
        Self {
            chart_idx,
            w: ((sx * scale).ceil() as u32).clamp(8, 4096),
            h: ((sy * scale).ceil() as u32).clamp(8, 4096),
        }
    }

    fn alloc_w(self, padding: u32) -> u32 {
        self.w.saturating_add(padding.saturating_mul(2))
    }

    fn alloc_h(self, padding: u32) -> u32 {
        self.h.saturating_add(padding.saturating_mul(2))
    }
}

#[derive(Clone, Copy, Debug)]
struct PackedChartRect {
    chart_idx: usize,
    x: u32,
    y: u32,
    w: u32,
    h: u32,
}

impl PackedChartRect {
    fn uv_rect(self, atlas_size: u32) -> ChartUvRect {
        let atlas_size = atlas_size.max(1) as f32;
        ChartUvRect {
            min: Vec2::new(
                (self.x as f32 + 0.5) / atlas_size,
                (self.y as f32 + 0.5) / atlas_size,
            ),
            max: Vec2::new(
                (self.x as f32 + self.w.saturating_sub(1).max(1) as f32 + 0.5) / atlas_size,
                (self.y as f32 + self.h.saturating_sub(1).max(1) as f32 + 0.5) / atlas_size,
            ),
        }
    }
}

fn pack_chart_requests(
    requests: &mut [ChartPackRequest],
    atlas_size: u32,
    padding: u32,
) -> Option<Vec<PackedChartRect>> {
    requests.sort_by(|a, b| {
        b.alloc_h(padding)
            .cmp(&a.alloc_h(padding))
            .then_with(|| b.alloc_w(padding).cmp(&a.alloc_w(padding)))
    });
    let mut out = Vec::with_capacity(requests.len());
    let mut x = 0u32;
    let mut y = 0u32;
    let mut row_h = 0u32;
    for request in requests.iter().copied() {
        let alloc_w = request.alloc_w(padding);
        let alloc_h = request.alloc_h(padding);
        if alloc_w > atlas_size || alloc_h > atlas_size {
            return None;
        }
        if x > 0 && x.saturating_add(alloc_w) > atlas_size {
            x = 0;
            y = y.checked_add(row_h)?;
            row_h = 0;
        }
        if y.saturating_add(alloc_h) > atlas_size {
            return None;
        }
        out.push(PackedChartRect {
            chart_idx: request.chart_idx,
            x: x + padding,
            y: y + padding,
            w: request.w,
            h: request.h,
        });
        x = x.checked_add(alloc_w)?;
        row_h = row_h.max(alloc_h);
    }
    Some(out)
}

fn ceil_sqrt_usize(n: usize) -> usize {
    if n <= 1 {
        return n;
    }
    let mut x = (n as f64).sqrt().ceil() as usize;
    while x.saturating_mul(x) < n {
        x += 1;
    }
    x
}

#[derive(Clone, Copy, Debug)]
struct ProjectionBucket {
    axis: usize,
    positive: bool,
}

impl ProjectionBucket {
    fn from_triangle(mesh: &LoadedPrimitive, ids: [usize; 3]) -> Self {
        let p = [
            Vec3::from_array(mesh.vertices[ids[0]].position),
            Vec3::from_array(mesh.vertices[ids[1]].position),
            Vec3::from_array(mesh.vertices[ids[2]].position),
        ];
        let n = (p[1] - p[0]).cross(p[2] - p[0]);
        let abs = n.abs();
        let axis = if abs.x >= abs.y && abs.x >= abs.z {
            0
        } else if abs.y >= abs.z {
            1
        } else {
            2
        };
        let positive = n[axis] >= 0.0;
        Self { axis, positive }
    }

    fn index(self) -> usize {
        self.axis * 2 + usize::from(self.positive)
    }

    fn project(self, p: Vec3) -> Vec2 {
        match self.axis {
            0 => Vec2::new(p.y, p.z),
            1 => Vec2::new(p.x, p.z),
            _ => Vec2::new(p.x, p.y),
        }
    }
}

fn connected_projection_charts(
    mesh: &LoadedPrimitive,
    source_tris: &[[usize; 3]],
    tri_buckets: &[ProjectionBucket],
) -> Vec<usize> {
    let mut dsu = DisjointSet::new(source_tris.len());
    let mut edges = HashMap::<(usize, PositionKey, PositionKey), usize>::new();
    for (tri_idx, (&ids, &bucket)) in source_tris.iter().zip(tri_buckets).enumerate() {
        for (a, b) in [(ids[0], ids[1]), (ids[1], ids[2]), (ids[2], ids[0])] {
            let ka = position_key(mesh, a);
            let kb = position_key(mesh, b);
            let (lo, hi) = if ka <= kb { (ka, kb) } else { (kb, ka) };
            let edge_key = (bucket.index(), lo, hi);
            if let Some(&other_tri) = edges.get(&edge_key) {
                dsu.union(tri_idx, other_tri);
            } else {
                edges.insert(edge_key, tri_idx);
            }
        }
    }

    let mut roots_to_chart = HashMap::<usize, usize>::new();
    let mut out = Vec::with_capacity(source_tris.len());
    for tri_idx in 0..source_tris.len() {
        let root = dsu.find(tri_idx);
        let next = roots_to_chart.len();
        let chart_idx = *roots_to_chart.entry(root).or_insert(next);
        out.push(chart_idx);
    }
    out
}

type PositionKey = [i64; 3];

fn position_key(mesh: &LoadedPrimitive, id: usize) -> PositionKey {
    let p = mesh.vertices[id].position;
    [
        quantize_position_axis(p[0]),
        quantize_position_axis(p[1]),
        quantize_position_axis(p[2]),
    ]
}

fn quantize_position_axis(v: f32) -> i64 {
    (v as f64 * 100_000.0).round() as i64
}

#[derive(Clone, Debug)]
struct DisjointSet {
    parent: Vec<usize>,
    rank: Vec<u8>,
}

impl DisjointSet {
    fn new(len: usize) -> Self {
        Self {
            parent: (0..len).collect(),
            rank: vec![0; len],
        }
    }

    fn find(&mut self, x: usize) -> usize {
        let parent = self.parent[x];
        if parent == x {
            x
        } else {
            let root = self.find(parent);
            self.parent[x] = root;
            root
        }
    }

    fn union(&mut self, a: usize, b: usize) {
        let mut root_a = self.find(a);
        let mut root_b = self.find(b);
        if root_a == root_b {
            return;
        }
        if self.rank[root_a] < self.rank[root_b] {
            std::mem::swap(&mut root_a, &mut root_b);
        }
        self.parent[root_b] = root_a;
        if self.rank[root_a] == self.rank[root_b] {
            self.rank[root_a] = self.rank[root_a].saturating_add(1);
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ProjectionChart {
    bucket: ProjectionBucket,
    bounds: ProjectionBounds,
    rect: ChartUvRect,
    area: f32,
}

impl ProjectionChart {
    fn empty() -> Self {
        Self {
            bucket: ProjectionBucket {
                axis: 2,
                positive: true,
            },
            bounds: ProjectionBounds::empty(),
            rect: ChartUvRect::empty(),
            area: 0.0,
        }
    }

    fn projected_area(self) -> f32 {
        let span = self.bounds.span_abs();
        let bounds_area = span.x * span.y;
        if bounds_area.is_finite() && bounds_area > 1.0e-8 {
            bounds_area
        } else {
            self.area.max(1.0e-8)
        }
    }
}

#[derive(Clone, Copy, Debug)]
struct ProjectionBounds {
    min: Vec2,
    max: Vec2,
}

impl ProjectionBounds {
    fn empty() -> Self {
        Self {
            min: Vec2::splat(f32::INFINITY),
            max: Vec2::splat(f32::NEG_INFINITY),
        }
    }

    fn include(&mut self, p: Vec2) {
        if p.is_finite() {
            self.min = self.min.min(p);
            self.max = self.max.max(p);
        }
    }

    fn valid(self) -> bool {
        self.min.is_finite() && self.max.is_finite()
    }

    fn normalize(self, p: Vec2) -> Vec2 {
        let span = self.max - self.min;
        Vec2::new(
            normalize_projected_axis(p.x, self.min.x, span.x),
            normalize_projected_axis(p.y, self.min.y, span.y),
        )
    }

    fn span_abs(self) -> Vec2 {
        (self.max - self.min).abs()
    }
}

fn normalize_projected_axis(v: f32, min: f32, span: f32) -> f32 {
    if span.abs() <= 1.0e-6 || !span.is_finite() {
        0.5
    } else {
        ((v - min) / span).clamp(0.0, 1.0)
    }
}

#[derive(Clone, Copy, Debug)]
struct ChartUvRect {
    min: Vec2,
    max: Vec2,
}

impl ChartUvRect {
    fn empty() -> Self {
        Self {
            min: Vec2::ZERO,
            max: Vec2::ZERO,
        }
    }

    fn map(self, uv: Vec2) -> [f32; 2] {
        (self.min + uv.clamp(Vec2::ZERO, Vec2::ONE) * (self.max - self.min)).to_array()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tile_glb::LoadedPrimitive;

    fn vertex(x: f32, y: f32, z: f32) -> Vertex3dTex {
        Vertex3dTex::new([x, y, z], [0.0, 0.0, 1.0], [0.0, 0.0], [1.0, 0.0, 0.0, 1.0])
    }

    fn primitive_with_two_triangles() -> LoadedPrimitive {
        LoadedPrimitive {
            vertices: vec![
                vertex(0.0, 0.0, 0.0),
                vertex(1.0, 0.0, 0.0),
                vertex(0.0, 1.0, 0.0),
                vertex(1.0, 0.0, 0.0),
                vertex(1.0, 1.0, 0.0),
                vertex(0.0, 1.0, 0.0),
            ],
            indices: vec![0, 1, 2, 3, 4, 5],
            albedo_rgba: None,
            albedo_mip_chain: None,
            normal_rgba: None,
            normal_mip_chain: None,
            metallic_roughness_rgba: None,
            metallic_roughness_mip_chain: None,
            emissive_rgba: None,
            emissive_mip_chain: None,
            metallic_factor: 0.0,
            roughness_factor: 1.0,
            emissive_factor: [0.0; 3],
            alpha_mode: crate::tile_glb::GltfAlphaMode::Opaque,
            alpha_cutoff: 0.5,
            double_sided: false,
            sampler: crate::gltf_helpers::GltfSamplerCpu {
                wrap_s: gltf::texture::WrappingMode::Repeat,
                wrap_t: gltf::texture::WrappingMode::Repeat,
                mag_filter: None,
                min_filter: None,
            },
        }
    }

    #[test]
    fn lightmap_uvs_are_projected_across_coplanar_triangles() {
        let tris = room_lightmap_triangles(&primitive_with_two_triangles());
        assert_eq!(tris.len(), 2);
        assert_eq!(tris[0].lightmap_uv[1], tris[1].lightmap_uv[0]);
        assert_eq!(tris[0].lightmap_uv[2], tris[1].lightmap_uv[2]);
        assert_ne!(tris[0].lightmap_uv, tris[1].lightmap_uv);
        for tri in &tris {
            for uv in tri.lightmap_uv {
                assert!((0.0..=1.0).contains(&uv[0]));
                assert!((0.0..=1.0).contains(&uv[1]));
            }
        }
    }

    #[test]
    fn room_gpu_mesh_deindexes_lightmap_vertices() {
        let mesh = build_room_env_lightmap_gpu_mesh(&primitive_with_two_triangles());
        assert_eq!(mesh.vertices.len(), 6);
        assert_eq!(mesh.indices, vec![0, 1, 2, 3, 4, 5]);
        assert_ne!(mesh.vertices[0].lightmap_uv, mesh.vertices[3].lightmap_uv);
    }
}
