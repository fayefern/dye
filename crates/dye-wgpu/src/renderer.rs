const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

fn wgpu_err<E: std::fmt::Display>(desc: &str) -> impl FnOnce(E) -> dye_render::BackendError + '_ {
    move |e| dye_render::BackendError(format!("{desc}: {e}"))
}

pub struct WgpuRenderer {
    label: String,
    device: wgpu::Device,
    adapter: wgpu::Adapter,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,

    dirty_surface: bool,
    curr_surface_texture: Option<wgpu::SurfaceTexture>,
    depth_texture: WgpuTexture,
    buffers: dye_render::Slab<wgpu::Buffer>,
    pipelines: dye_render::Slab<WgpuPipeline>,
}

impl WgpuRenderer {
    pub fn new(desc: &dye_render::BackendDesc) -> Result<Self, dye_render::BackendError> {
        let instance =
            wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance
            .create_surface(desc.window.clone())
            .map_err(wgpu_err("surface creation"))?;
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            ..Default::default()
        }))
        .map_err(wgpu_err("no suitable adapter"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("render-wgpu"),
            ..Default::default()
        }))
        .map_err(wgpu_err("device creation"))?;

        device.on_uncaptured_error(std::sync::Arc::new(|e| eprintln!("wgpu: {e}")));

        let mut config = surface
            .get_default_config(&adapter, desc.width, desc.height)
            .ok_or_else(|| dye_render::BackendError("surface unsupported by adapter".into()))?;

        let caps = surface.get_capabilities(&adapter);
        if let Some(format) = caps.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = format;
        }
        surface.configure(&device, &config);

        let depth_texture = WgpuTexture::create_depth_texture(&device, config.width, config.height);

        let info = adapter.get_info();

        Ok(Self {
            label: format!("wgpu-{}", info.backend.to_str()),
            device,
            adapter,
            queue,
            surface,
            config,
            dirty_surface: false,
            curr_surface_texture: None,
            depth_texture,
            buffers: dye_render::Slab::new(),
            pipelines: dye_render::Slab::new(),
        })
    }

    fn reconfigure(&mut self) {
        self.curr_surface_texture = None;
        self.surface.configure(&self.device, &self.config);
        self.depth_texture =
            WgpuTexture::create_depth_texture(&self.device, self.config.width, self.config.height);
        self.dirty_surface = false;
    }

    fn prepare_draw(&self, bound: &mut WgpuBound, pass: &mut wgpu::RenderPass<'static>) -> bool {
        let Some(pipeline) = bound.pipeline else {
            return false;
        };
        if bound.stale {
            let p1 = self.pipelines.get(pipeline);
            if let Some(layout) = &p1.group {
                let mut entries = Vec::with_capacity(p1.bindings.len());
                for &binding in &p1.bindings {
                    let Some(&buffer) = bound.buffers.get(&binding) else {
                        return false;
                    };
                    entries.push(wgpu::BindGroupEntry {
                        binding,
                        resource: self.buffers.get(buffer).as_entire_binding(),
                    });
                }
                let group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: None,
                    layout,
                    entries: &entries,
                });
                pass.set_bind_group(0, &group, &[]);
            }
            bound.stale = false;
        }
        true
    }
}

impl dye_render::Backend for WgpuRenderer {
    fn name(&self) -> &str {
        &self.label
    }

    fn get_size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    fn set_size(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        if (self.config.width, self.config.height) != (width, height) {
            self.config.width = width;
            self.config.height = height;
            self.dirty_surface = true;
        }
    }

    fn create_buffer(
        &mut self,
        label: &str,
        usage: u32,
        size: u64,
    ) -> dye_render::backend::BufferId {
        let mut flags = wgpu::BufferUsages::COPY_DST;
        for (ours, theirs) in [
            (dye_render::usage::VERTEX, wgpu::BufferUsages::VERTEX),
            (dye_render::usage::INDEX, wgpu::BufferUsages::INDEX),
            (dye_render::usage::UNIFORM, wgpu::BufferUsages::UNIFORM),
            (dye_render::usage::STORAGE, wgpu::BufferUsages::STORAGE),
        ] {
            if usage & ours != 0 {
                flags |= theirs
            }
        }
        let size = size.max(16).next_multiple_of(4);
        let raw = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size,
            usage: flags,
            mapped_at_creation: false,
        });

        dye_render::BufferId(self.buffers.insert(raw))
    }

    fn write_buffer(&mut self, buffer: dye_render::BufferId, offset: u64, data: &[u8]) {
        if data.is_empty() {
            return;
        }
        if offset.is_multiple_of(4) && data.len().is_multiple_of(4) {
            self.queue
                .write_buffer(self.buffers.get(buffer.0), offset, data);
        }
    }

    fn destroy_buffer(&mut self, buffer: dye_render::BufferId) {
        self.buffers.remove(buffer.0);
    }

    fn create_pipeline(
        &mut self,
        desc: &dye_render::backend::PipelineDesc,
    ) -> Result<dye_render::backend::PipelineId, dye_render::BackendError> {
        let module = self
            .device
            .create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(desc.label),
                source: wgpu::ShaderSource::Wgsl(std::borrow::Cow::Borrowed(desc.shader)),
            });
        let entries: Vec<_> = desc
            .bindings
            .iter()
            .map(|b| wgpu::BindGroupLayoutEntry {
                binding: b.binding,
                visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: match b.kind {
                        dye_render::BufferKind::Uniform => wgpu::BufferBindingType::Uniform,
                        dye_render::BufferKind::StorageRead => {
                            wgpu::BufferBindingType::Storage { read_only: true }
                        }
                    },
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            })
            .collect();
        let group = (!entries.is_empty()).then(|| {
            self.device
                .create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some(desc.label),
                    entries: &entries,
                })
        });
        let layouts = [group.as_ref()];
        let layout = self
            .device
            .create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some(desc.label),
                bind_group_layouts: &layouts[..group.is_some() as usize],
                immediate_size: 0,
            });

        let attrs: Vec<wgpu::VertexAttribute> = desc
            .vertex_layout
            .iter()
            .flat_map(|l| l.attrs.iter())
            .map(|a| wgpu::VertexAttribute {
                format: match a.format {
                    dye_render::VertexFormat::Float32x2 => wgpu::VertexFormat::Float32x2,
                    dye_render::VertexFormat::Float32x3 => wgpu::VertexFormat::Float32x3,
                    dye_render::VertexFormat::Float32x4 => wgpu::VertexFormat::Float32x4,
                },
                offset: a.offset,
                shader_location: a.location,
            })
            .collect();
        let vertex_buffers: Vec<Option<wgpu::VertexBufferLayout>> = desc
            .vertex_layout
            .iter()
            .map(|l| {
                Some(wgpu::VertexBufferLayout {
                    array_stride: l.stride,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &attrs,
                })
            })
            .collect();

        let topology = |t: dye_render::Topology| match t {
            dye_render::Topology::Triangles => wgpu::PrimitiveTopology::TriangleList,
            dye_render::Topology::Lines => wgpu::PrimitiveTopology::LineList,
        };
        let compare = |c: dye_render::Compare| match c {
            dye_render::Compare::Always => wgpu::CompareFunction::Always,
            dye_render::Compare::Greater => wgpu::CompareFunction::Greater,
            dye_render::Compare::GreaterEqual => wgpu::CompareFunction::GreaterEqual,
            dye_render::Compare::Less => wgpu::CompareFunction::Less,
            dye_render::Compare::LessEqual => wgpu::CompareFunction::LessEqual,
        };

        let raw = self
            .device
            .create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(desc.label),
                layout: Some(&layout),
                vertex: wgpu::VertexState {
                    module: &module,
                    entry_point: Some(desc.vertex),
                    compilation_options: Default::default(),
                    buffers: &vertex_buffers,
                },
                primitive: wgpu::PrimitiveState {
                    topology: topology(desc.topology),
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: desc.cull_back.then_some(wgpu::Face::Back),
                    ..Default::default()
                },
                depth_stencil: desc.depth.map(|d| wgpu::DepthStencilState {
                    format: DEPTH_FORMAT,
                    depth_write_enabled: Some(d.write),
                    depth_compare: Some(compare(d.compare)),
                    stencil: Default::default(),
                    bias: Default::default(),
                }),
                multisample: Default::default(),
                fragment: Some(wgpu::FragmentState {
                    module: &module,
                    entry_point: Some(desc.fragment),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: self.config.format,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                multiview_mask: None,
                cache: None,
            });

        let bindings = desc.bindings.iter().map(|b| b.binding).collect();

        Ok(dye_render::PipelineId(self.pipelines.insert(
            WgpuPipeline {
                raw,
                group,
                bindings,
                label: desc.label.to_string(),
            },
        )))
    }

    fn destroy_pipeline(&mut self, pipeline: dye_render::PipelineId) {
        self.pipelines.remove(pipeline.0);
    }

    fn begin_frame(&mut self) -> Option<dye_render::backend::FrameInfo> {
        if self.dirty_surface {
            self.reconfigure();
        }
        let texture = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(t) => t,
            wgpu::CurrentSurfaceTexture::Suboptimal(t) => {
                self.dirty_surface = true;
                t
            }
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.dirty_surface = true;
                return None;
            }
            _ => return None,
        };
        self.curr_surface_texture = Some(texture);

        Some(dye_render::FrameInfo {
            width: self.config.width,
            height: self.config.height,
        })
    }

    fn end_frame(&mut self, list: &dye_render::backend::CommandList) {
        let Some(frame) = self.curr_surface_texture.take() else {
            return;
        };
        let view = frame.texture.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let mut pass: Option<wgpu::RenderPass<'static>> = None;
        let mut bound = WgpuBound::default();

        for command in &list.commands {
            match *command {
                dye_render::Command::BeginPass { color, depth } => {
                    drop(pass.take());
                    pass = Some(
                        encoder
                            .begin_render_pass(&wgpu::RenderPassDescriptor {
                                label: None,
                                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                                    view: &view,
                                    depth_slice: None,
                                    resolve_target: None,
                                    ops: wgpu::Operations {
                                        load: wgpu::LoadOp::Clear(wgpu::Color {
                                            r: color[0] as f64,
                                            g: color[1] as f64,
                                            b: color[2] as f64,
                                            a: color[3] as f64,
                                        }),
                                        store: wgpu::StoreOp::Store,
                                    },
                                })],
                                depth_stencil_attachment: depth.map(|clear| {
                                    wgpu::RenderPassDepthStencilAttachment {
                                        view: &self.depth_texture.view,
                                        depth_ops: Some(wgpu::Operations {
                                            load: wgpu::LoadOp::Clear(clear),
                                            store: wgpu::StoreOp::Store,
                                        }),
                                        stencil_ops: None,
                                    }
                                }),
                                timestamp_writes: None,
                                occlusion_query_set: None,
                                multiview_mask: None,
                            })
                            .forget_lifetime(),
                    );
                    bound = WgpuBound::default();
                }
                dye_render::Command::EndPass => drop(pass.take()),
                dye_render::Command::SetPipeline(pipeline) => {
                    let Some(pass) = pass.as_mut() else {
                        continue;
                    };
                    pass.set_pipeline(&self.pipelines.get(pipeline.0).raw);
                    bound.pipeline = Some(pipeline.0);
                    bound.stale = true;
                }
                dye_render::Command::BindBuffer { binding, buffer } => {
                    bound.buffers.insert(binding, buffer.0);
                    bound.stale = true;
                }
                dye_render::Command::VertexBuffer(buffer) => {
                    if let Some(pass) = pass.as_mut() {
                        pass.set_vertex_buffer(0, self.buffers.get(buffer.0).slice(..));
                    }
                }
                dye_render::Command::IndexBuffer(buffer) => {
                    if let Some(pass) = pass.as_mut() {
                        pass.set_index_buffer(
                            self.buffers.get(buffer.0).slice(..),
                            wgpu::IndexFormat::Uint32,
                        );
                    }
                }
                dye_render::Command::Draw {
                    vertices,
                    instances,
                    first_vertex,
                    first_instance,
                } => {
                    let Some(pass) = pass.as_mut() else {
                        continue;
                    };
                    if self.prepare_draw(&mut bound, pass) {
                        pass.draw(
                            first_vertex..first_vertex + vertices,
                            first_instance..first_instance + instances,
                        );
                    }
                }
                dye_render::Command::DrawIndexed {
                    indices,
                    instances,
                    first_index,
                    first_instance,
                    base_vertex,
                } => {
                    let Some(pass) = pass.as_mut() else {
                        continue;
                    };
                    if self.prepare_draw(&mut bound, pass) {
                        pass.draw_indexed(
                            first_index..first_index + indices,
                            base_vertex,
                            first_instance..first_instance + instances,
                        );
                    }
                }
            }
        }
        drop(pass);

        self.queue.submit([encoder.finish()]);
        self.queue.present(frame);
    }

    fn wait_idle(&mut self) {
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
    }
}

struct WgpuTexture {
    raw: wgpu::Texture,
    view: wgpu::TextureView,
}

impl WgpuTexture {
    fn create_depth_texture(device: &wgpu::Device, width: u32, height: u32) -> Self {
        let raw = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("depth"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: DEPTH_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });

        let view = raw.create_view(&Default::default());

        Self { raw, view }
    }
}

struct WgpuPipeline {
    label: String,
    raw: wgpu::RenderPipeline,
    group: Option<wgpu::BindGroupLayout>,
    bindings: Vec<u32>,
}

#[derive(Default)]
struct WgpuBound {
    pipeline: Option<u32>,
    buffers: std::collections::BTreeMap<u32, u32>,
    stale: bool,
}
