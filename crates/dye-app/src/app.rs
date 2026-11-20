use std::{sync::Arc, time::Instant};

use dye_assets::{reload::Reloader, store::Assets};
use dye_opts::launch::{GpuOpt, LaunchOpt};
use dye_render::{Backend, BackendDesc, BackendError};
use tracing::{info, warn};
use winit::{
    application::ApplicationHandler,
    dpi::PhysicalSize,
    event::WindowEvent,
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    window::{Icon, Window},
};

use crate::{error::AppError, scene::Scene};

pub type BackendFactory = fn(&BackendDesc) -> Result<Box<dyn Backend>, BackendError>;

pub type Renderers = &'static [(&'static str, BackendFactory)];

pub fn start(
    launch_opt: LaunchOpt,
    renderers: Renderers,
    assets: Assets,
    reloader: Option<Reloader>,
) -> Result<(), AppError> {
    let event_loop = EventLoop::builder()
        .build()
        .map_err(|e| AppError::External(e.to_string()))?;
    start_on(event_loop, launch_opt, renderers, assets, reloader)
}

pub fn start_on(
    event_loop: EventLoop<()>,
    launch_opt: LaunchOpt,
    renderers: Renderers,
    assets: Assets,
    reloader: Option<Reloader>,
) -> Result<(), AppError> {
    let mut app = App {
        launch_opt,
        renderers,
        assets: Some(assets),
        reloader: reloader,
        running: None,
        exit: Ok(()),
    };
    event_loop.set_control_flow(ControlFlow::Poll);
    event_loop
        .run_app(&mut app)
        .map_err(|e| AppError::External(e.to_string()))?;
    app.exit
}

struct App {
    launch_opt: LaunchOpt,
    renderers: Renderers,
    assets: Option<Assets>,
    reloader: Option<Reloader>,
    running: Option<Running>,
    exit: Result<(), AppError>,
}

impl App {
    fn run(&mut self, event_loop: &ActiveEventLoop) -> Result<Running, AppError> {
        let icon = Icon::from_rgba(vec![0u8; 4096], 32, 32).ok();
        let window_attributes = Window::default_attributes()
            .with_title("Dye")
            .with_window_icon(icon)
            .with_inner_size(PhysicalSize::new(
                self.launch_opt.window.size.width,
                self.launch_opt.window.size.height,
            ));
        let window = Arc::new(
            event_loop
                .create_window(window_attributes)
                .map_err(|e| AppError::External(e.to_string()))?,
        );

        let (width, height) = {
            let size = window.inner_size();
            (size.width, size.height)
        };
        let desc = BackendDesc {
            window: window.clone(),
            width,
            height,
            opt: &self.launch_opt.gpu,
        };
        let (_, factory) = self
            .renderers
            .iter()
            .find(|(name, _)| *name == "wgpu")
            .ok_or_else(|| AppError::Internal("renderer not found".to_string()))?;
        let mut backend = factory(&desc).map_err(|e| AppError::External(e.to_string()))?;

        let mut assets = self
            .assets
            .take()
            .ok_or_else(|| AppError::Internal("assets already consumed".to_string()))?;
        let reloader = self.reloader.take();
        let scene = Scene::new(&mut backend, &mut assets)?;

        let running = Running {
            start: Instant::now(),

            backend,
            window,
            assets,
            reloader,
            scene,

            quit: false,
        };

        Ok(running)
    }
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.running.is_some() {
            return;
        }
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
        match event {
            WindowEvent::CloseRequested => running.quit = true,
            WindowEvent::Resized(size) => {
                running.backend.set_size(size.width, size.height);
            }
            WindowEvent::RedrawRequested => {
                if let Some(reloader) = &mut running.reloader
                    && let Some(reload) = reloader.poll(&mut running.assets)
                {
                    for d in &reload.diags {
                        warn!("{d}");
                    }
                    if reload.changed > 0 {
                        info!(changed = reload.changed, "pack files changed, reloading");
                    }
                }

                running.scene.frame(
                    &mut running.backend,
                    &mut running.assets,
                    running.start.elapsed().as_secs_f32(),
                );
                running.window.request_redraw();
            }
            _ => {}
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

struct Running {
    start: Instant,

    backend: Box<dyn Backend>,
    window: Arc<Window>,
    assets: Assets,
    reloader: Option<Reloader>,
    scene: Scene,

    quit: bool,
}
