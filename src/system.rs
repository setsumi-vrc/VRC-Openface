//! Small platform queries: GPU adapters, process memory and timer resolution.

#[derive(Clone, Debug)]
pub struct GpuAdapter {
    /// DXGI adapter index, which is also the DirectML device id.
    pub index: u32,
    pub name: String,
}

#[cfg(windows)]
pub fn gpu_adapters() -> Vec<GpuAdapter> {
    use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE, IDXGIFactory1};

    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else { return Vec::new() };
    let mut out = Vec::new();
    for index in 0.. {
        let Ok(adapter) = (unsafe { factory.EnumAdapters1(index) }) else { break };
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else { continue };
        if desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32 != 0 {
            continue;
        }
        let len = desc.Description.iter().position(|&c| c == 0).unwrap_or(desc.Description.len());
        out.push(GpuAdapter { index, name: String::from_utf16_lossy(&desc.Description[..len]) });
    }
    out
}

#[cfg(not(windows))]
pub fn gpu_adapters() -> Vec<GpuAdapter> {
    Vec::new()
}

/// Private bytes of this process.
#[cfg(windows)]
pub fn private_memory() -> Option<u64> {
    use windows::Win32::System::ProcessStatus::{
        GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
    };
    use windows::Win32::System::Threading::GetCurrentProcess;

    let mut counters = PROCESS_MEMORY_COUNTERS_EX::default();
    let size = std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32;
    unsafe {
        GetProcessMemoryInfo(GetCurrentProcess(), &mut counters as *mut _ as *mut PROCESS_MEMORY_COUNTERS, size)
            .ok()?;
    }
    Some(counters.PrivateUsage as u64)
}

#[cfg(not(windows))]
pub fn private_memory() -> Option<u64> {
    None
}

/// Raises the system timer resolution to 1 ms for the life of the process so short
/// sleeps in the output loop are accurate (the Windows default is ~15.6 ms).
#[cfg(windows)]
pub fn high_resolution_timer() {
    unsafe {
        windows::Win32::Media::timeBeginPeriod(1);
    }
}

#[cfg(not(windows))]
pub fn high_resolution_timer() {}
