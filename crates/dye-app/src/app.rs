use std::{
    fs,
    path::{Path, PathBuf},
    sync::Arc,
};

use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Icon, Window},
};

use crate::error::{AppError, AppOptError};

pub fn start(opt: AppOpt) -> Result<(), AppError> {
    let event_loop = EventLoop::builder()
        .build()
        .map_err(|e| AppError::External(e.to_string()))?;
    start_on(event_loop, opt)
}

pub fn start_on(event_loop: EventLoop<()>, opt: AppOpt) -> Result<(), AppError> {
    let mut app = App::init(opt)?;
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop
        .run_app(&mut app)
        .map_err(|e| AppError::External(e.to_string()))?;
    app.exit
}

struct App {
    opt: AppOpt,
    running: Option<Running>,
    exit: Result<(), AppError>,
}

impl App {
    fn init(opt: AppOpt) -> Result<Self, AppError> {
        fs::create_dir_all(&opt.data_dir).map_err(|e| AppError::External(e.to_string()))?;
        fs::create_dir_all(&opt.cache_dir).map_err(|e| AppError::External(e.to_string()))?;

        Ok(Self {
            opt,
            running: None,
            exit: Ok(()),
        })
    }

    fn run(&mut self, event_loop: &ActiveEventLoop) -> Result<Running, AppError> {
        let icon = Icon::from_rgba(vec![0u8; 4096], 32, 32).ok();
        let window_attributes = Window::default_attributes()
            .with_title("Dye")
            .with_window_icon(icon)
            .with_inner_size(PhysicalSize::new(self.opt.size.0, self.opt.size.1));
        let window = event_loop
            .create_window(window_attributes)
            .map_err(|e| AppError::External(e.to_string()))?;

        let running = Running {
            window: Arc::new(window),

            quit: false,
        };
        Ok(running)
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        match self.run(event_loop) {
            Ok(r) => self.running = Some(r),
            Err(e) => {
                self.exit = Err(e);
                event_loop.exit();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: winit::window::WindowId,
        event: winit::event::WindowEvent,
    ) {
        let Some(running) = &mut self.running else {
            return;
        };
        if event == WindowEvent::CloseRequested {
            running.quit = true;
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let Some(running) = &mut self.running else {
            return;
        };
        if running.quit {
            event_loop.exit();
        }
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        println!("we made it!");
    }
}

pub struct AppOpt {
    data_dir: PathBuf,
    cache_dir: PathBuf,

    size: (u32, u32),
}

impl AppOpt {
    pub fn builder() -> AppOptBuilder {
        AppOptBuilder::new()
    }

    pub fn get_data_dir(&self) -> &Path {
        &self.data_dir
    }

    pub fn get_cache_dir(&self) -> &Path {
        &self.cache_dir
    }

    pub fn set_data_dir(&mut self, data_dir: &Path) -> bool {
        if data_dir.as_os_str().is_empty() || (data_dir.exists() && !data_dir.is_dir()) {
            return false;
        }
        self.data_dir = data_dir.to_path_buf();
        true
    }

    pub fn set_cache_dir(&mut self, cache_dir: &Path) -> bool {
        if cache_dir.as_os_str().is_empty() || (cache_dir.exists() && !cache_dir.is_dir()) {
            return false;
        }
        self.cache_dir = cache_dir.to_path_buf();
        true
    }
}

#[derive(Default)]
pub struct AppOptBuilder {
    data_dir: Option<PathBuf>,
    cache_dir: Option<PathBuf>,

    size: (u32, u32),
}

impl AppOptBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn build(self) -> Result<AppOpt, AppOptError> {
        let data_dir = self
            .data_dir
            .ok_or(AppOptError::Missing("data_dir".to_string()))?;
        if data_dir.as_os_str().is_empty() || (data_dir.exists() && !data_dir.is_dir()) {
            return Err(AppOptError::Invalid("data_dir".to_string()));
        }
        let cache_dir = self
            .cache_dir
            .ok_or(AppOptError::Missing("cache_dir".to_string()))?;
        if cache_dir.as_os_str().is_empty() || (cache_dir.exists() && !cache_dir.is_dir()) {
            return Err(AppOptError::Invalid("cache_dir".to_string()));
        }

        if self.size.0 == 0 || self.size.1 == 0 {
            return Err(AppOptError::NullSize);
        }

        Ok(AppOpt {
            data_dir,
            cache_dir,

            size: self.size,
        })
    }

    pub fn with_data_dir(mut self, data_dir: &Path) -> Self {
        self.data_dir = Some(data_dir.to_path_buf());
        self
    }

    pub fn with_cache_dir(mut self, cache_dir: &Path) -> Self {
        self.cache_dir = Some(cache_dir.to_path_buf());
        self
    }

    pub const fn with_window_inner_size(mut self, width: u32, height: u32) -> Self {
        self.size = (width, height);
        self
    }
}

struct Running {
    window: Arc<Window>,

    quit: bool,
}
