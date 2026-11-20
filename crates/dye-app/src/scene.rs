use bytemuck::{Pod, Zeroable};
use dye_assets::{
    handle::Handle,
    kinds::Image,
    store::{Assets, ScopeId},
};
use dye_render::{
    AddressMode, Backend, BindingKind, BufferId, CommandList, Compare, FilterMode, PipelineDesc,
    PipelineId, SamplerDesc, SamplerId, TextureDesc, TextureFormat, TextureId, Topology,
    VertexFormat,
    atlas::{Atlas, AtlasOpts, UvRect},
    backend::{Binding, DepthState, VertexAttr, VertexLayout},
    usage,
};
use glam::{
    Mat4, Vec3,
    camera::rh::{proj::directx::perspective_infinite_reverse, view::look_at_mat4},
};
use tracing::{error, warn};

use crate::error::AppError;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Vertex {
    pos: [f32; 3],
    uv: [f32; 2],
}

struct Face {
    id: &'static str,
    normal: Vec3,
    right: Vec3,
    up: Vec3,
}

const FACES: [Face; 6] = [
    Face {
        id: "core:debug/front",
        normal: Vec3::Z,
        right: Vec3::X,
        up: Vec3::Y,
    },
    Face {
        id: "core:debug/back",
        normal: Vec3::NEG_Z,
        right: Vec3::NEG_X,
        up: Vec3::Y,
    },
    Face {
        id: "core:debug/right",
        normal: Vec3::X,
        right: Vec3::NEG_Z,
        up: Vec3::Y,
    },
    Face {
        id: "core:debug/left",
        normal: Vec3::NEG_X,
        right: Vec3::Z,
        up: Vec3::Y,
    },
    Face {
        id: "core:debug/top",
        normal: Vec3::Y,
        right: Vec3::X,
        up: Vec3::NEG_Z,
    },
    Face {
        id: "core:debug/bottom",
        normal: Vec3::NEG_Y,
        right: Vec3::X,
        up: Vec3::Z,
    },
];

const INDICES: [u32; 36] = {
    let mut out = [0u32; 36];
    let mut q = 0;

    while q < 6 {
        let (b, i) = (q as u32 * 4, q * 6);
        out[i] = b;
        out[i + 1] = b + 1;
        out[i + 2] = b + 2;
        out[i + 3] = b;
        out[i + 4] = b + 2;
        out[i + 5] = b + 3;
        q += 1;
    }

    out
};

const SHADER: &str = r#"
    struct Uniforms { mvp: mat4x4<f32> };
    @group(0) @binding(0) var<uniform> u: Uniforms;
    @group(0) @binding(1) var atlas: texture_2d<f32>;
    @group(0) @binding(2) var atlas_sampler: sampler;

    struct VsIn {
        @location(0) pos: vec3<f32>,
        @location(1) uv: vec2<f32>,
    };
    struct VsOut {
        @builtin(position) clip: vec4<f32>,
        @location(0) uv: vec2<f32>,
    };

    @vertex
    fn vs_main(v: VsIn) -> VsOut {
        var out: VsOut;
        out.clip = u.mvp * vec4<f32>(v.pos, 1.0);
        out.uv = v.uv;
        return out;
    }

    @fragment
    fn fs_main(v: VsOut) -> @location(0) vec4<f32> {
        return textureSample(atlas, atlas_sampler, v.uv);
    }
"#;

fn mesh(atlas: &Atlas, handles: &[Handle<Image>; 6]) -> [Vertex; 24] {
    let mut out = [Vertex::zeroed(); 24];

    for (i, (face, &h)) in FACES.iter().zip(handles).enumerate() {
        let UvRect { min, max } = atlas.uv(h);
        let corners = [
            (-face.right - face.up, [min[0], max[1]]),
            (face.right - face.up, [max[0], max[1]]),
            (face.right + face.up, [max[0], min[1]]),
            (-face.right + face.up, [min[0], min[1]]),
        ];

        for (j, (offset, uv)) in corners.into_iter().enumerate() {
            out[i * 4 + j] = Vertex {
                pos: (face.normal + offset).to_array(),
                uv,
            }
        }
    }

    out
}

fn upload(backend: &mut Box<dyn Backend>, atlas: &Atlas) -> TextureId {
    let texture = backend.create_texture_with(
        &TextureDesc {
            label: "atlas",
            width: atlas.width,
            height: atlas.height,
            format: TextureFormat::Rgba8UnormSrgb,
        },
        &atlas.pixels,
    );

    texture
}

pub struct Scene {
    commands: CommandList,
    pipeline: PipelineId,
    vertices: BufferId,
    indices: BufferId,
    uniform: BufferId,
    sampler: SamplerId,
    texture: TextureId,
    atlas: Atlas,
    handles: [Handle<Image>; 6],
    scope: ScopeId,
}

impl Scene {
    pub fn new(backend: &mut Box<dyn Backend>, assets: &mut Assets) -> Result<Scene, AppError> {
        let handles = FACES.map(|f| assets.handle::<Image>(f.id));
        let scope = assets.scope("scene");
        for h in handles {
            assets.require(scope, h);
        }
        assets.wait(scope);
        let atlas = Atlas::init(AtlasOpts::default(), assets, &handles)
            .map_err(|e| AppError::Internal(e.to_string()))?;

        let texture = upload(backend, &atlas);
        let sampler = backend.create_sampler(&SamplerDesc {
            label: "nearest",
            filter: FilterMode::Nearest,
            address: AddressMode::ClampToEdge,
        });
        let vertices = backend.create_buffer_with(
            "vertices",
            usage::VERTEX,
            bytemuck::cast_slice(&mesh(&atlas, &handles)),
        );
        let indices =
            backend.create_buffer_with("indices", usage::INDEX, bytemuck::cast_slice(&INDICES));
        let uniform = backend.create_buffer("mvp", usage::UNIFORM, 64);

        let pipeline = backend
            .create_pipeline(&PipelineDesc {
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
                            format: VertexFormat::Float32x2,
                            offset: 12,
                        },
                    ],
                }),
                bindings: &[
                    Binding {
                        binding: 0,
                        kind: BindingKind::Uniform,
                    },
                    Binding {
                        binding: 1,
                        kind: BindingKind::Texture2d,
                    },
                    Binding {
                        binding: 2,
                        kind: BindingKind::Sampler,
                    },
                ],
                depth: Some(DepthState {
                    write: true,
                    compare: Compare::Greater,
                }),
                cull_back: true,
                topology: Topology::Triangles,
            })
            .map_err(|e| AppError::External(e.to_string()))?;

        Ok(Self {
            commands: CommandList::default(),
            pipeline,
            vertices,
            indices,
            uniform,
            sampler,
            texture,
            atlas,
            handles,
            scope,
        })
    }

    fn refresh(&mut self, backend: &mut Box<dyn Backend>, assets: &mut Assets) {
        if !self.atlas.is_stale(assets) || !assets.progress(self.scope).is_done() {
            return;
        }

        match Atlas::init(AtlasOpts::default(), assets, &self.handles) {
            Ok(atlas) => {
                backend.destroy_texture(self.texture);
                self.texture = upload(backend, &atlas);
                let vertices = mesh(&atlas, &self.handles);
                backend.write_buffer(self.vertices, 0, bytemuck::cast_slice(&vertices));
                self.atlas = atlas;
            }
            Err(e) => error!("atlas rebuild failed: {e}"),
        }
    }

    pub fn frame(&mut self, backend: &mut Box<dyn Backend>, assets: &mut Assets, seconds: f32) {
        assets.commit();
        for d in assets.take_diagnostics() {
            warn!("{d}");
        }
        self.refresh(backend, assets);

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
        self.commands.bind_buffer(0, self.uniform);
        self.commands.bind_texture(1, self.texture);
        self.commands.bind_sampler(2, self.sampler);
        self.commands.vertex_buffer(self.vertices);
        self.commands.index_buffer(self.indices);
        self.commands.draw_indexed(INDICES.len() as u32, 1);
        self.commands.end_pass();

        backend.end_frame(&self.commands);
    }
}
