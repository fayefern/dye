use dye_opt::launch::{PowerPreference, PresentMode, WgpuBackend};

pub trait ToWgpu<T> {
    fn to_wgpu(self) -> T;
}

impl ToWgpu<wgpu::Backends> for WgpuBackend {
    fn to_wgpu(self) -> wgpu::Backends {
        match self {
            WgpuBackend::All => wgpu::Backends::all(),
            WgpuBackend::Vulkan => wgpu::Backends::VULKAN,
            WgpuBackend::Metal => wgpu::Backends::METAL,
            WgpuBackend::Dx12 => wgpu::Backends::DX12,
            WgpuBackend::Gl => wgpu::Backends::GL,
            WgpuBackend::Noop => wgpu::Backends::NOOP,
        }
    }
}

impl ToWgpu<wgpu::PowerPreference> for PowerPreference {
    fn to_wgpu(self) -> wgpu::PowerPreference {
        match self {
            PowerPreference::None => wgpu::PowerPreference::None,
            PowerPreference::HighPerformance => wgpu::PowerPreference::HighPerformance,
            PowerPreference::LowPower => wgpu::PowerPreference::LowPower,
        }
    }
}

impl ToWgpu<wgpu::PresentMode> for PresentMode {
    fn to_wgpu(self) -> wgpu::PresentMode {
        match self {
            PresentMode::AutoVsync => wgpu::PresentMode::AutoVsync,
            PresentMode::AutoNoVsync => wgpu::PresentMode::AutoNoVsync,
        }
    }
}
