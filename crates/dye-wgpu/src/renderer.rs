use crate::opts::ToWgpu;

const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;

fn wgpu_err<E: std::fmt::Display>(desc: &str) -> impl FnOnce(E) -> dye_render::BackendError + '_ {
    move |e| dye_render::BackendError(format!("{desc}: {e}"))
}

pub struct WgpuRenderer {
    label: String,
    device: wgpu::Device,
    queue: wgpu::Queue,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,

    dirty_surface: bool,
    curr_surface_texture: Option<wgpu::SurfaceTexture>,
    depth_texture: WgpuTexture,
    buffers: dye_render::Slab<wgpu::Buffer>,
    textures: dye_render::Slab<WgpuTexture>,
    samplers: dye_render::Slab<wgpu::Sampler>,
    pipelines: dye_render::Slab<WgpuPipeline>,
}

impl WgpuRenderer {
    pub fn new(desc: &dye_render::BackendDesc) -> Result<Self, dye_render::BackendError> {
        let instance = wgpu::Instance::new(instance_descriptor(desc.opt));
        let surface = instance
            .create_surface(desc.window.clone())
            .map_err(wgpu_err("surface creation"))?;
        let adapter = select_adapter(&instance, &surface, desc.opt)
            .map_err(wgpu_err("no suitable adapter"))?;
        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("device-wgpu"),
            ..Default::default()
        }))
        .map_err(wgpu_err("device creation"))?;

        let policy = desc.opt.error_policy;
        device.on_uncaptured_error(std::sync::Arc::new(move |e| match policy {
            dye_opt::launch::GpuErrorPolicy::Log => tracing::error!("wgpu: {e}"),
            dye_opt::launch::GpuErrorPolicy::Panic => panic!("wgpu: {e}"),
        }));

        let mut config = surface
            .get_default_config(&adapter, desc.width, desc.height)
            .ok_or_else(|| dye_render::BackendError("surface unsupported by adapter".into()))?;

        let caps = surface.get_capabilities(&adapter);
        if let Some(format) = caps.formats.iter().copied().find(|f| f.is_srgb()) {
            config.format = format;
        }
        config.present_mode = desc.opt.present_mode.to_wgpu();
        config.desired_maximum_frame_latency = desc.opt.desired_max_frame_latency.max(1);
        surface.configure(&device, &config);

        let depth_texture = WgpuTexture::create_depth_texture(&device, config.width, config.height);

        let info = adapter.get_info();
        tracing::info!(
            adapter = %info.name,
            backend = ?info.backend,
            device_type = ?info.device_type,
            driver = %info.driver,
            driver_info = %info.driver_info,
            format = ?config.format,
            present_mode = ?config.present_mode,
            frame_latency = config.desired_maximum_frame_latency,
            "gpu initialized"
        );

        Ok(Self {
            label: format!("wgpu-{}", info.backend.to_str()),
            device,
            queue,
            surface,
            config,
            dirty_surface: false,
            curr_surface_texture: None,
            depth_texture,
            buffers: dye_render::Slab::new(),
            textures: dye_render::Slab::new(),
            samplers: dye_render::Slab::new(),
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
                for &(binding, kind) in &p1.bindings {
                    let resource = match (kind, bound.resources.get(&binding)) {
                        (
                            dye_render::BindingKind::Uniform | dye_render::BindingKind::StorageRead,
                            Some(&BoundResource::Buffer(b)),
                        ) => self.buffers.get(b).as_entire_binding(),
                        (dye_render::BindingKind::Texture2d, Some(&BoundResource::Texture(t))) => {
                            wgpu::BindingResource::TextureView(&self.textures.get(t).view)
                        }
                        (dye_render::BindingKind::Sampler, Some(&BoundResource::Sampler(s))) => {
                            wgpu::BindingResource::Sampler(self.samplers.get(s))
                        }
                        _ => return false,
                    };
                    entries.push(wgpu::BindGroupEntry { binding, resource });
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
        if !(0..8192).contains(&width) || !(0..8192).contains(&height) {
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

    fn create_texture(&mut self, desc: &dye_render::TextureDesc) -> dye_render::TextureId {
        let raw = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some(desc.label),
            size: wgpu::Extent3d {
                width: desc.width.max(1),
                height: desc.height.max(1),
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: desc.format.to_wgpu(),
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let view = raw.create_view(&Default::default());

        dye_render::TextureId(self.textures.insert(WgpuTexture { raw, view }))
    }

    fn write_texture(&mut self, texture: dye_render::TextureId, data: &[u8]) {
        let tex = self.textures.get(texture.0);
        let (w, h) = (tex.raw.width(), tex.raw.height());
        let bytes_per_pixel = 4;
        if data.len() as u64 != w as u64 * h as u64 * bytes_per_pixel as u64 {
            tracing::warn!(
                "write_texture: {} bytes for a {w}*{h} texture; ignored",
                data.len()
            );
            return;
        }
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex.raw,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(w * bytes_per_pixel),
                rows_per_image: Some(h),
            },
            wgpu::Extent3d {
                width: w,
                height: h,
                depth_or_array_layers: 1,
            },
        );
    }

    fn destroy_texture(&mut self, texture: dye_render::TextureId) {
        self.textures.remove(texture.0);
    }

    fn create_sampler(&mut self, desc: &dye_render::SamplerDesc) -> dye_render::SamplerId {
        let filter = desc.filter.to_wgpu();
        let address = desc.address.to_wgpu();
        let raw = self.device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some(desc.label),
            address_mode_u: address,
            address_mode_v: address,
            address_mode_w: address,
            mag_filter: filter,
            min_filter: filter,
            ..Default::default()
        });

        dye_render::SamplerId(self.samplers.insert(raw))
    }

    fn destroy_sampler(&mut self, sampler: dye_render::SamplerId) {
        self.samplers.remove(sampler.0);
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
        let entries: Vec<_> = desc.bindings.iter().map(layout_entry).collect();
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

        let bindings = desc.bindings.iter().map(|b| (b.binding, b.kind)).collect();

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
                    bound
                        .resources
                        .insert(binding, BoundResource::Buffer(buffer.0));
                    bound.stale = true;
                }
                dye_render::Command::BindTexture { binding, texture } => {
                    bound
                        .resources
                        .insert(binding, BoundResource::Texture(texture.0));
                    bound.stale = true;
                }
                dye_render::Command::BindSampler { binding, sampler } => {
                    bound
                        .resources
                        .insert(binding, BoundResource::Sampler(sampler.0));
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

fn instance_descriptor(opt: &dye_opt::launch::GpuOpt) -> wgpu::InstanceDescriptor {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();

    if !matches!(opt.backend, dye_opt::launch::WgpuBackend::All) {
        desc.backends = opt.backend.to_wgpu();
    }
    match opt.validation {
        dye_opt::launch::Validation::Auto => {}
        dye_opt::launch::Validation::On => desc.flags |= wgpu::InstanceFlags::debugging(),
        dye_opt::launch::Validation::Off => {
            desc.flags &= !(wgpu::InstanceFlags::VALIDATION | wgpu::InstanceFlags::DEBUG)
        }
    }
    desc
}

fn select_adapter(
    instance: &wgpu::Instance,
    surface: &wgpu::Surface<'_>,
    opt: &dye_opt::launch::GpuOpt,
) -> Result<wgpu::Adapter, dye_render::BackendError> {
    let request_adapter = |force_fallback_adapter: bool| {
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: opt.power_preference.to_wgpu(),
            force_fallback_adapter,
            compatible_surface: Some(surface),
            ..Default::default()
        }))
        .map_err(wgpu_err("no suitable adapter"))
    };

    let all_adapters = || pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));

    let auto_selection = |reason: String| {
        tracing::warn!("{reason}; reverting to automatic adapter selection");
        request_adapter(false)
    };

    match &opt.adapter {
        dye_opt::launch::AdapterSelection::Auto => request_adapter(false),
        dye_opt::launch::AdapterSelection::Name(name) => {
            let needle = name.to_lowercase();
            let found = all_adapters().into_iter().find(|a| {
                a.get_info().name.to_lowercase().contains(&needle)
                    && a.is_surface_supported(surface)
            });
            match found {
                Some(a) => Ok(a),
                None => auto_selection(format!("no surface-compatible adapter matches {name:?}")),
            }
        }
        dye_opt::launch::AdapterSelection::Index(idx) => match all_adapters().into_iter().nth(*idx)
        {
            Some(a) if a.is_surface_supported(surface) => Ok(a),
            Some(a) => auto_selection(format!(
                "adapter {idx} ({}) cannot present to this window",
                a.get_info().name
            )),
            None => auto_selection(format!("adapter index {idx} out of range")),
        },
        dye_opt::launch::AdapterSelection::Software => request_adapter(true),
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
    bindings: Vec<(u32, dye_render::BindingKind)>,
}

#[derive(Clone, Copy)]
enum BoundResource {
    Buffer(u32),
    Texture(u32),
    Sampler(u32),
}

#[derive(Default)]
struct WgpuBound {
    pipeline: Option<u32>,
    resources: std::collections::BTreeMap<u32, BoundResource>,
    stale: bool,
}

fn layout_entry(b: &dye_render::Binding) -> wgpu::BindGroupLayoutEntry {
    let (visibility, ty) = match b.kind {
        dye_render::BindingKind::Uniform => (
            wgpu::ShaderStages::VERTEX_FRAGMENT,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        ),
        dye_render::BindingKind::StorageRead => (
            wgpu::ShaderStages::VERTEX_FRAGMENT,
            wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Storage { read_only: true },
                has_dynamic_offset: false,
                min_binding_size: None,
            },
        ),
        dye_render::BindingKind::Texture2d => (
            wgpu::ShaderStages::FRAGMENT,
            wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
        ),
        dye_render::BindingKind::Sampler => (
            wgpu::ShaderStages::FRAGMENT,
            wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        ),
    };

    wgpu::BindGroupLayoutEntry {
        binding: b.binding,
        visibility,
        ty,
        count: None,
    }
}

pub fn describe_adapters(backend: dye_opt::launch::WgpuBackend) -> Vec<String> {
    let mut desc = wgpu::InstanceDescriptor::new_without_display_handle_from_env();
    if !matches!(backend, dye_opt::launch::WgpuBackend::All) {
        desc.backends = backend.to_wgpu();
    }
    let instance = wgpu::Instance::new(desc);
    let adapters = pollster::block_on(instance.enumerate_adapters(wgpu::Backends::all()));

    adapters
        .iter()
        .enumerate()
        .map(|(i, a)| {
            let info = a.get_info();
            format!(
                "[{i}] {} ({:?}, {:?})",
                info.name, info.backend, info.device_type
            )
        })
        .collect()
}
