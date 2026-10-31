//! Models and sounds: media a mod hands the engine as plain data, for its content to name. The
//! simulation never looks inside them; a client draws and plays them.

/// A textured triangle mesh.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelDef {
    /// Namespaced, such as `"base:cow"`.
    pub name: String,
    /// Vertex positions in meters, +Z up, with the feet at the origin and the front facing +Y.
    pub positions: Vec<[f32; 3]>,
    /// Texture coordinates for each vertex, from the texture's top left corner.
    pub uvs: Vec<[f32; 2]>,
    /// Three vertex indices per triangle, counterclockwise seen from outside.
    pub indices: Vec<u32>,
    pub texture: Texture,
}

/// An image, drawn with hard pixel edges.
#[derive(Clone, Debug, PartialEq)]
pub struct Texture {
    pub width: u32,
    pub height: u32,
    /// sRGB with alpha, four bytes per pixel, rows from the top.
    pub rgba: Vec<u8>,
}

impl ModelDef {
    /// Checks the mesh is well formed.
    pub fn validate(&self) -> Result<(), String> {
        if self.uvs.len() != self.positions.len() {
            return Err(format!("{} positions but {} texture coordinates", self.positions.len(), self.uvs.len()));
        }
        if self.indices.is_empty() || !self.indices.len().is_multiple_of(3) {
            return Err(format!("indices must be whole triangles, got {}", self.indices.len()));
        }
        if let Some(i) = self.indices.iter().find(|&&i| i as usize >= self.positions.len()) {
            return Err(format!("index {i} is past the last vertex"));
        }
        if self.positions.iter().flatten().chain(self.uvs.iter().flatten()).any(|v| !v.is_finite()) {
            return Err("vertices must be finite".into());
        }
        let t = &self.texture;
        if t.width == 0 || t.height == 0 || t.rgba.len() != (t.width * t.height * 4) as usize {
            return Err(format!("texture is {}×{} but holds {} bytes", t.width, t.height, t.rgba.len()));
        }
        Ok(())
    }
}

/// A sound, as the bytes of an Ogg Vorbis file.
#[derive(Clone, Debug, PartialEq)]
pub struct SoundDef {
    /// Namespaced, such as `"base:moo"`.
    pub name: String,
    pub ogg: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn triangle() -> ModelDef {
        ModelDef {
            name: "test:tri".into(),
            positions: vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            uvs: vec![[0.0; 2]; 3],
            indices: vec![0, 1, 2],
            texture: Texture { width: 1, height: 1, rgba: vec![255; 4] },
        }
    }

    #[test]
    fn checks_the_mesh() {
        assert_eq!(triangle().validate(), Ok(()));
        assert!(ModelDef { indices: vec![0, 1], ..triangle() }.validate().is_err());
        assert!(ModelDef { indices: vec![0, 1, 3], ..triangle() }.validate().is_err());
        assert!(ModelDef { uvs: vec![[0.0; 2]; 2], ..triangle() }.validate().is_err());
        let texture = Texture { width: 2, height: 1, rgba: vec![255; 4] };
        assert!(ModelDef { texture, ..triangle() }.validate().is_err());
    }
}
