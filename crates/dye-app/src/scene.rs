use bytemuck::{Pod, Zeroable};
use dye_render::{
    Backend, BackendError, BufferId, BufferKind, CommandList, Compare, PipelineDesc, PipelineId,
    Topology, VertexFormat,
    backend::{BufferBinding, DepthState, VertexAttr, VertexLayout},
    usage,
};
use glam::{
    Mat4, Vec3,
    camera::rh::{proj::directx::perspective_infinite_reverse, view::look_at_mat4},
};

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 3],
    color: [f32; 3],
}

const fn v(pos: [f32; 3], color: [f32; 3]) -> Vertex {
    Vertex { pos, color }
}

const VERTICES: [Vertex; 8] = [
    v([-1.0, -1.0, 1.0], [1.0, 0.0, 0.0]),
    v([1.0, -1.0, 1.0], [0.0, 1.0, 0.0]),
    v([1.0, 1.0, 1.0], [0.0, 0.0, 1.0]),
    v([-1.0, 1.0, 1.0], [1.0, 1.0, 0.0]),
    v([-1.0, -1.0, -1.0], [1.0, 0.0, 1.0]),
    v([1.0, -1.0, -1.0], [0.0, 1.0, 1.0]),
    v([1.0, 1.0, -1.0], [1.0, 1.0, 1.0]),
    v([-1.0, 1.0, -1.0], [0.2, 0.2, 0.2]),
];

const INDICES: [u32; 36] = [
    0, 1, 2, 0, 2, 3, 5, 4, 7, 5, 7, 6, 1, 5, 6, 1, 6, 2, 4, 0, 3, 4, 3, 7, 3, 2, 6, 3, 6, 7, 4, 5,
    1, 4, 1, 0,
];

const SHADER: &str = r#"
    struct Uniforms { mvp: mat4x4<f32> };
    @group(0) @binding(0) var<uniform> u: Uniforms;

    struct VsIn {
        @location(0) pos: vec3<f32>,
        @location(1) color: vec3<f32>,
    };
    struct VsOut {
    @builtin(position) clip: vec4<f32>,
    @location(0) color: vec3<f32>,
    };

    @vertex
    fn vs_main(v: VsIn) -> VsOut {
        var out: VsOut;
        out.clip = u.mvp * vec4<f32>(v.pos, 1.0);
        out.color = v.color;
        return out;
    }

    @fragment
    fn fs_main(v: VsOut) -> @location(0) vec4<f32> {
        return vec4<f32>(v.color, 1.0);
    }
"#;

pub struct Scene {
    commands: CommandList,
    pipeline: PipelineId,
    vertices: BufferId,
    indices: BufferId,
    uniform: BufferId,
}

impl Scene {
    pub fn new(backend: &mut Box<dyn Backend>) -> Result<Scene, BackendError> {
        let vertices =
            backend.create_buffer_with("vertices", usage::VERTEX, bytemuck::cast_slice(&VERTICES));
        let indices =
            backend.create_buffer_with("indices", usage::INDEX, bytemuck::cast_slice(&INDICES));
        let uniform = backend.create_buffer("mvp", usage::UNIFORM, 64);

        let pipeline = backend.create_pipeline(&PipelineDesc {
            label: "cube",
            shader: SHADER,
            vertex: "vs_main",
            fragment: "fs_main",
            vertex_layout: Some(VertexLayout {
                stride: size_of::<Vertex>() as u64,
                attrs: &[
                    VertexAttr {
                        location: 0,
                        format: VertexFormat::Float32x3,
                        offset: 0,
                    },
                    VertexAttr {
                        location: 1,
                        format: VertexFormat::Float32x3,
                        offset: 12,
                    },
                ],
            }),
            bindings: &[BufferBinding {
                binding: 0,
                kind: BufferKind::Uniform,
            }],
            depth: Some(DepthState {
                write: true,
                compare: Compare::Greater,
            }),
            cull_back: true,
            topology: Topology::Triangles,
        })?;

        Ok(Self {
            commands: CommandList::default(),
            pipeline,
            vertices,
            indices,
            uniform,
        })
    }

    pub fn frame(&mut self, backend: &mut Box<dyn Backend>, seconds: f32) {
        let Some(info) = backend.begin_frame() else {
            return;
        };

        let cam_proj = perspective_infinite_reverse(
            45.0f32.to_radians(),
            info.width as f32 / info.height as f32,
            0.1,
        );
        let cam_view = look_at_mat4(Vec3::new(0.0, 1.5, 5.0), Vec3::ZERO, Vec3::Y);
        let cam_model = Mat4::from_rotation_y(seconds) * Mat4::from_rotation_x(seconds * 0.5);
        let mvp = cam_proj * cam_view * cam_model;

        backend.write_buffer(self.uniform, 0, bytemuck::cast_slice(&mvp.to_cols_array()));

        self.commands.clear();
        self.commands.begin_pass([0.05, 0.05, 0.08, 1.0], Some(0.0));
        self.commands.pipeline(self.pipeline);
        self.commands.bind(0, self.uniform);
        self.commands.vertex_buffer(self.vertices);
        self.commands.index_buffer(self.indices);
        self.commands.draw_indexed(INDICES.len() as u32, 1);
        self.commands.end_pass();

        backend.end_frame(&self.commands);
    }
}
