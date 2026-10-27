//! Three-d's [`InstancedMesh`] with its GPU buffers kept and refilled: its `set_instances` clones
//! the instances, bounds every one and makes new buffers, which was most of a moving frame.

use three_d::renderer::*;

/// Outside three-d's own range, which its [`GeometryId`] reserves.
const REFILLED_MESH_ID: u16 = 0x0100;

/// Drawn the way an [`InstancedMesh`] is, by the same vertex shader, or as [`RefilledMesh::ribbon`].
/// Unlit: no normals or tangents are uploaded.
pub(super) struct RefilledMesh {
    context: Context,
    indices: TriangleBuffer,
    positions: VertexBuffer<Vec3>,
    uvs: Option<VertexBuffer<Vec2>>,
    shape: Shape,
    base_shader: String,
    rows: [InstanceBuffer<Vec4>; 3],
    texture_rows: Option<[InstanceBuffer<Vec3>; 2]>,
    colors: Option<InstanceBuffer<Vec4>>,
    count: u32,
    /// Reused between fills, so a refill allocates nothing.
    staged_rows: [Vec<Vec4>; 3],
    staged_texture_rows: [Vec<Vec3>; 2],
    staged_colors: Vec<Vec4>,
}

#[derive(Clone, Copy)]
enum Shape {
    Mesh,
    Ribbon,
}

impl RefilledMesh {
    pub(super) fn new(context: &Context, instances: &Instances, cpu_mesh: &CpuMesh) -> Self {
        let unlit = CpuMesh {
            normals: None,
            tangents: None,
            ..cpu_mesh.clone()
        };
        let base_shader = Mesh::new(context, &unlit).vertex_shader_source();
        Self::with_shader(context, instances, cpu_mesh, Shape::Mesh, base_shader)
    }

    /// A strip from `(0, 0, 0)` to `(1, 0, 0)` of each instance, turned to face the eye: two
    /// triangles where a tube takes two a side. `dashed` gives it a `u` running along it.
    pub(super) fn ribbon(context: &Context, instances: &Instances, dashed: bool) -> Self {
        let corners = [
            vec2(0.0, -1.0),
            vec2(1.0, -1.0),
            vec2(0.0, 1.0),
            vec2(1.0, 1.0),
        ];
        let strip = CpuMesh {
            positions: Positions::F32(corners.iter().map(|at| at.extend(0.0)).collect()),
            indices: Indices::U8(vec![0, 1, 2, 2, 1, 3]),
            uvs: dashed.then(|| corners.iter().map(|at| vec2(at.x, 0.0)).collect()),
            ..Default::default()
        };
        let defines = if dashed { "#define USE_UVS\n" } else { "" };
        let base_shader = format!("{defines}{}", include_str!("ribbon.vert"));
        Self::with_shader(context, instances, &strip, Shape::Ribbon, base_shader)
    }

    fn with_shader(
        context: &Context,
        instances: &Instances,
        cpu_mesh: &CpuMesh,
        shape: Shape,
        base_shader: String,
    ) -> Self {
        assert!(cpu_mesh.colors.is_none(), "vertex colors are not uploaded");
        let mut mesh = Self {
            context: context.clone(),
            indices: TriangleBuffer::new(context, cpu_mesh),
            positions: VertexBuffer::new_with_data(context, &cpu_mesh.positions.to_f32()),
            // Flipped as three-d's own meshes flip them.
            uvs: cpu_mesh.uvs.as_ref().map(|uvs| {
                let flipped: Vec<Vec2> = uvs.iter().map(|uv| vec2(uv.x, 1.0 - uv.y)).collect();
                VertexBuffer::new_with_data(context, &flipped)
            }),
            shape,
            base_shader,
            rows: std::array::from_fn(|_| InstanceBuffer::new(context)),
            texture_rows: None,
            colors: None,
            count: 0,
            staged_rows: Default::default(),
            staged_texture_rows: Default::default(),
            staged_colors: Vec::new(),
        };
        mesh.set_instances(instances);
        mesh
    }

    pub(super) fn instance_count(&self) -> u32 {
        self.count
    }

    pub(super) fn set_instances(&mut self, instances: &Instances) {
        self.set_transformations(&instances.transformations);
        self.set_texture_transformations(instances.texture_transformations.as_deref());
        self.set_colors(instances.colors.as_deref());
    }

    pub(super) fn set_transformations(&mut self, transformations: &[Mat4]) {
        for (axis, staged) in self.staged_rows.iter_mut().enumerate() {
            staged.clear();
            staged.extend(transformations.iter().map(|placed| placed.row(axis)));
        }
        for (buffer, staged) in self.rows.iter_mut().zip(&self.staged_rows) {
            buffer.fill(staged);
        }
        self.count = transformations.len() as u32;
    }

    pub(super) fn set_texture_transformations(&mut self, texture_transformations: Option<&[Mat3]>) {
        let Some(texture_transformations) = texture_transformations else {
            self.texture_rows = None;
            return;
        };
        let [first, second] = &mut self.staged_texture_rows;
        first.clear();
        second.clear();
        for texture in texture_transformations {
            first.push(vec3(texture.x.x, texture.y.x, texture.z.x));
            second.push(vec3(texture.x.y, texture.y.y, texture.z.y));
        }
        let context = &self.context;
        let buffers = self
            .texture_rows
            .get_or_insert_with(|| std::array::from_fn(|_| InstanceBuffer::new(context)));
        for (buffer, staged) in buffers.iter_mut().zip(&self.staged_texture_rows) {
            buffer.fill(staged);
        }
    }

    pub(super) fn set_colors(&mut self, colors: Option<&[Srgba]>) {
        let Some(colors) = colors else {
            self.colors = None;
            return;
        };
        self.staged_colors.clear();
        self.staged_colors
            .extend(colors.iter().map(|color| color.to_linear_srgb()));
        self.colors
            .get_or_insert_with(|| InstanceBuffer::new(&self.context))
            .fill(&self.staged_colors);
    }
}

impl<'a> IntoIterator for &'a RefilledMesh {
    type Item = &'a dyn Geometry;
    type IntoIter = std::iter::Once<&'a dyn Geometry>;

    fn into_iter(self) -> Self::IntoIter {
        std::iter::once(self)
    }
}

impl Geometry for RefilledMesh {
    fn draw(&self, viewer: &dyn Viewer, program: &Program, render_states: RenderStates) {
        program.use_uniform("viewProjection", viewer.projection() * viewer.view());
        match self.shape {
            Shape::Mesh => {
                program.use_uniform("animationTransform", Mat4::identity());
                program.use_uniform("modelMatrix", Mat4::identity());
            }
            Shape::Ribbon => program.use_uniform("eye", viewer.position()),
        }
        for (name, buffer) in ["row1", "row2", "row3"].into_iter().zip(&self.rows) {
            program.use_instance_attribute(name, buffer);
        }
        if program.requires_attribute("tex_transform_row1")
            && let Some([first, second]) = &self.texture_rows
        {
            program.use_instance_attribute("tex_transform_row1", first);
            program.use_instance_attribute("tex_transform_row2", second);
        }
        if program.requires_attribute("instance_color")
            && let Some(colors) = &self.colors
        {
            program.use_instance_attribute("instance_color", colors);
        }
        program.use_vertex_attribute("position", &self.positions);
        if program.requires_attribute("uv_coordinates")
            && let Some(uvs) = &self.uvs
        {
            program.use_vertex_attribute("uv_coordinates", uvs);
        }
        self.indices
            .draw_instanced(program, render_states, viewer, self.count);
    }

    fn vertex_shader_source(&self) -> String {
        format!(
            "#define USE_INSTANCE_TRANSFORMS\n{}{}{}",
            if self.colors.is_some() {
                "#define USE_INSTANCE_COLORS\n"
            } else {
                ""
            },
            if self.texture_rows.is_some() {
                "#define USE_INSTANCE_TEXTURE_TRANSFORMATION\n"
            } else {
                ""
            },
            self.base_shader
        )
    }

    fn id(&self) -> GeometryId {
        let bit = |on: bool, at: u16| u16::from(on) << at;
        GeometryId(
            REFILLED_MESH_ID
                | bit(self.uvs.is_some(), 0)
                | bit(self.colors.is_some(), 1)
                | bit(self.texture_rows.is_some(), 2)
                | match self.shape {
                    Shape::Mesh => 0,
                    Shape::Ribbon => 1 << 3,
                },
        )
    }

    fn render_with_material(
        &self,
        material: &dyn Material,
        viewer: &dyn Viewer,
        lights: &[&dyn Light],
    ) {
        if let Err(error) = render_with_material(&self.context, viewer, self, material, lights) {
            panic!("{error}");
        }
    }

    fn render_with_effect(
        &self,
        effect: &dyn Effect,
        viewer: &dyn Viewer,
        lights: &[&dyn Light],
        color_texture: Option<ColorTexture>,
        depth_texture: Option<DepthTexture>,
    ) {
        if let Err(error) = render_with_effect(
            &self.context,
            viewer,
            self,
            effect,
            lights,
            color_texture,
            depth_texture,
        ) {
            panic!("{error}");
        }
    }

    /// Unbounded, so nothing is culled and a render call keeps its given order; an empty mesh is
    /// still skipped.
    fn aabb(&self) -> AxisAlignedBoundingBox {
        match self.count {
            0 => AxisAlignedBoundingBox::EMPTY,
            _ => AxisAlignedBoundingBox::INFINITE,
        }
    }
}
