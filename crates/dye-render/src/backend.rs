pub struct Slab<T> {
    items: Vec<Option<T>>,
    free: Vec<u32>,
}

impl<T> Default for Slab<T> {
    fn default() -> Self {
        Self {
            items: Vec::new(),
            free: Vec::new(),
        }
    }
}

impl<T> Slab<T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, item: T) -> u32 {
        if let Some(idx) = self.free.pop() {
            self.items[idx as usize] = Some(item);
            idx
        } else {
            self.items.push(Some(item));
            self.items.len() as u32 - 1
        }
    }

    pub fn remove(&mut self, idx: u32) -> Option<T> {
        let item = self.items.get_mut(idx as usize)?.take();
        if item.is_some() {
            self.free.push(idx);
        }
        item
    }

    pub fn get(&self, idx: u32) -> &T {
        self.items[idx as usize]
            .as_ref()
            .expect("stale render handle")
    }

    pub fn get_mut(&mut self, idx: u32) -> &mut T {
        self.items[idx as usize]
            .as_mut()
            .expect("stale render handle")
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BufferId(pub u32);
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PipelineId(pub u32);

pub mod usage {
    pub const VERTEX: u32 = 1;
    pub const INDEX: u32 = 2;
    pub const UNIFORM: u32 = 4;
    pub const STORAGE: u32 = 8;
}

pub struct BackendDesc {
    pub window: std::sync::Arc<dyn WindowSource>,
    pub width: u32,
    pub height: u32,
    pub vsync: bool,
}

pub trait WindowSource:
    raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle + Send + Sync
{
}
impl<T: raw_window_handle::HasWindowHandle + raw_window_handle::HasDisplayHandle + Send + Sync>
    WindowSource for T
{
}

pub type BackendFactory = fn(&BackendDesc) -> Result<Box<dyn Backend>, crate::error::BackendError>;

pub trait Backend: Send {
    fn name(&self) -> &str;

    fn get_size(&self) -> (u32, u32);
    fn set_size(&mut self, width: u32, height: u32);

    fn create_buffer(&mut self, label: &str, usage: u32, size: u64) -> BufferId;
    fn write_buffer(&mut self, buffer: BufferId, offset: u64, data: &[u8]);
    fn create_buffer_with(&mut self, label: &str, usage: u32, data: &[u8]) -> BufferId {
        let buffer = self.create_buffer(label, usage, data.len() as u64);
        self.write_buffer(buffer, 0, data);
        buffer
    }
    fn destroy_buffer(&mut self, buffer: BufferId);

    fn create_pipeline(
        &mut self,
        desc: &PipelineDesc,
    ) -> Result<PipelineId, crate::error::BackendError>;
    fn destroy_pipeline(&mut self, pipeline: PipelineId);

    fn begin_frame(&mut self) -> Option<FrameInfo>;
    fn end_frame(&mut self, list: &CommandList);
    fn wait_idle(&mut self);
}

pub struct PipelineDesc<'a> {
    pub label: &'a str,
    pub shader: &'a str,
    pub vertex: &'a str,
    pub fragment: &'a str,
    pub vertex_layout: Option<VertexLayout<'a>>,
    pub bindings: &'a [BufferBinding],
    pub depth: Option<DepthState>,
    pub cull_back: bool,
    pub topology: Topology,
}

#[derive(Debug, Clone, Copy)]
pub struct VertexLayout<'a> {
    pub stride: u64,
    pub attrs: &'a [VertexAttr],
}

#[derive(Debug, Clone, Copy)]
pub struct VertexAttr {
    pub location: u32,
    pub format: VertexFormat,
    pub offset: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VertexFormat {
    Float32x2,
    Float32x3,
    Float32x4,
}

#[derive(Debug, Clone, Copy)]
pub struct BufferBinding {
    pub binding: u32,
    pub kind: BufferKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferKind {
    Uniform,
    StorageRead,
}

#[derive(Debug, Clone, Copy)]
pub struct DepthState {
    pub write: bool,
    pub compare: Compare,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Compare {
    Always,
    Greater,
    GreaterEqual,
    Less,
    LessEqual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Topology {
    Triangles,
    Lines,
}

#[derive(Debug, Clone, Copy)]
pub struct FrameInfo {
    pub width: u32,
    pub height: u32,
}

#[derive(Default)]
pub struct CommandList {
    pub commands: Vec<Command>,
}

impl CommandList {
    pub fn clear(&mut self) {
        self.commands.clear();
    }

    pub fn push(&mut self, command: Command) {
        self.commands.push(command);
    }

    pub fn begin_pass(&mut self, color: [f32; 4], depth: Option<f32>) {
        self.commands.push(Command::BeginPass { color, depth });
    }

    pub fn end_pass(&mut self) {
        self.commands.push(Command::EndPass);
    }

    pub fn pipeline(&mut self, pipeline: PipelineId) {
        self.commands.push(Command::SetPipeline(pipeline));
    }

    pub fn bind(&mut self, binding: u32, buffer: BufferId) {
        self.commands.push(Command::BindBuffer { binding, buffer });
    }

    pub fn vertex_buffer(&mut self, buffer: BufferId) {
        self.commands.push(Command::VertexBuffer(buffer));
    }

    pub fn index_buffer(&mut self, buffer: BufferId) {
        self.commands.push(Command::IndexBuffer(buffer));
    }

    pub fn draw(&mut self, vertices: u32, instances: u32) {
        self.commands.push(Command::Draw {
            vertices,
            instances,
            first_vertex: 0,
            first_instance: 0,
        });
    }

    pub fn draw_indexed(&mut self, indices: u32, instances: u32) {
        self.commands.push(Command::DrawIndexed {
            indices,
            instances,
            first_index: 0,
            first_instance: 0,
            base_vertex: 0,
        });
    }
}

#[derive(Debug, Clone, Copy)]
pub enum Command {
    BeginPass {
        color: [f32; 4],
        depth: Option<f32>,
    },
    EndPass,
    SetPipeline(PipelineId),
    BindBuffer {
        binding: u32,
        buffer: BufferId,
    },
    VertexBuffer(BufferId),
    IndexBuffer(BufferId),
    Draw {
        vertices: u32,
        instances: u32,
        first_vertex: u32,
        first_instance: u32,
    },
    DrawIndexed {
        indices: u32,
        instances: u32,
        first_index: u32,
        first_instance: u32,
        base_vertex: i32,
    },
}
