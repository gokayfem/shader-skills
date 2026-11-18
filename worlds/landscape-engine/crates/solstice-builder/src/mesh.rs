//! Input mesh type.

use glam::{Vec2, Vec3};

/// A vertex of an input mesh.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vertex {
    pub position: Vec3,
    pub normal: Vec3,
    pub uv: Vec2,
}

/// An indexed triangle mesh.
#[derive(Clone, Debug, Default)]
pub struct Mesh {
    pub vertices: Vec<Vertex>,
    pub indices: Vec<u32>,
}

impl Mesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// How finely this mesh's points will be kept once it is built, in the mesh's own units: a
    /// 65,535th of its longest side, the same along all three.
    ///
    /// Every point of a built mesh lies on that grid (see `build_virtual_mesh`). For most
    /// things it is far finer than matters: a ten metre statue keeps its points to 0.15 mm.
    /// But it is one grid for the whole mesh, so a mesh that is very long keeps its points
    /// coarsely however thin it is. A road a kilometre long, built as one mesh, keeps them to
    /// 15 mm, up and down as well as along; paint laid 12 mm above it as another such mesh
    /// then comes and goes under it. Something long with fine detail is better built as
    /// several meshes, each of a part of it.
    pub fn grid_step(&self) -> f32 {
        let (mut min, mut max) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
        for vertex in &self.vertices {
            min = min.min(vertex.position);
            max = max.max(vertex.position);
        }
        if self.vertices.is_empty() {
            return 0.0;
        }
        (max - min).max_element() / 65535.0
    }

    /// Replaces every normal with the area-weighted average of its adjacent faces.
    pub fn compute_smooth_normals(&mut self) {
        for vertex in &mut self.vertices {
            vertex.normal = Vec3::ZERO;
        }
        for triangle in self.indices.chunks_exact(3) {
            let [a, b, c] = [triangle[0], triangle[1], triangle[2]].map(|i| i as usize);
            let face = (self.vertices[b].position - self.vertices[a].position)
                .cross(self.vertices[c].position - self.vertices[a].position);
            self.vertices[a].normal += face;
            self.vertices[b].normal += face;
            self.vertices[c].normal += face;
        }
        for vertex in &mut self.vertices {
            vertex.normal = vertex.normal.normalize_or(Vec3::Y);
        }
    }

    /// Appends another mesh.
    pub fn append(&mut self, other: &Mesh) {
        let base = self.vertices.len() as u32;
        self.vertices.extend_from_slice(&other.vertices);
        self.indices.extend(other.indices.iter().map(|i| i + base));
    }
}
