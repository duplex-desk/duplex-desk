use std::sync::mpsc::{self, Receiver, Sender, SyncSender, TryRecvError};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use windows::Win32::Foundation::HMODULE;
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_UNKNOWN;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_MAP_READ,
    D3D11_MAPPED_SUBRESOURCE, D3D11_SDK_VERSION, D3D11_TEXTURE2D_DESC, D3D11_USAGE_STAGING,
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Texture2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT, DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_B8G8R8A8_UNORM_SRGB,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, DXGI_ERROR_ACCESS_LOST, DXGI_ERROR_NOT_FOUND, DXGI_ERROR_WAIT_TIMEOUT,
    DXGI_OUTDUPL_FRAME_INFO, DXGI_OUTPUT_DESC, IDXGIAdapter1, IDXGIFactory1, IDXGIOutput1,
    IDXGIOutputDuplication, IDXGIResource,
};
use windows::core::Interface;

use crate::{
    config::{DisplayInfo, DuplexScapConfig},
    errors::DuplexScapError,
    frame::DuplexScapFrame,
};

pub struct WindowsCapturer {
    running: Option<CaptureThread>,
}

struct CaptureThread {
    stop_tx: Sender<()>,
    join_handle: JoinHandle<()>,
}

#[derive(Clone, Copy, Debug)]
struct OutputTarget {
    display_id: u32,
    adapter_index: u32,
    output_index: u32,
    width: u32,
    height: u32,
}

struct StagingTexture {
    texture: ID3D11Texture2D,
    width: u32,
    height: u32,
    format: DXGI_FORMAT,
}

impl WindowsCapturer {
    pub fn new() -> Self {
        Self { running: None }
    }

    pub fn list_displays() -> Result<Vec<DisplayInfo>, DuplexScapError> {
        let outputs = enumerate_outputs().map_err(DuplexScapError::Internal)?;
        Ok(outputs
            .into_iter()
            .map(|output| DisplayInfo {
                display_id: output.display_id,
                width: output.width,
                height: output.height,
            })
            .collect())
    }

    pub fn start(
        &mut self,
        config: DuplexScapConfig,
    ) -> Result<Receiver<DuplexScapFrame>, DuplexScapError> {
        if self.running.is_some() {
            return Err(DuplexScapError::AlreadyRunning);
        }

        let outputs = enumerate_outputs().map_err(DuplexScapError::Internal)?;
        if outputs.is_empty() {
            return Err(DuplexScapError::Internal(
                "no desktop output found on this machine".to_string(),
            ));
        }

        let target = if config.display_id == 0 {
            outputs[0]
        } else {
            outputs
                .iter()
                .find(|item| item.display_id == config.display_id)
                .copied()
                .ok_or(DuplexScapError::DisplayNotFound(config.display_id))?
        };

        let fps = if config.fps == 0 { 30 } else { config.fps };
        let (frame_tx, frame_rx) = mpsc::sync_channel::<DuplexScapFrame>(1);
        let (stop_tx, stop_rx) = mpsc::channel::<()>();

        let join_handle = thread::spawn(move || {
            if let Err(err) = capture_thread_main(target, fps, frame_tx, stop_rx) {
                tracing::warn!(
                    display_id = target.display_id,
                    error = %err,
                    "windows desktop duplication capture stopped"
                );
            }
        });

        self.running = Some(CaptureThread {
            stop_tx,
            join_handle,
        });

        Ok(frame_rx)
    }

    pub fn stop(&mut self) -> Result<(), DuplexScapError> {
        let Some(capture_thread) = self.running.take() else {
            return Ok(());
        };

        let _ = capture_thread.stop_tx.send(());
        let _ = capture_thread.join_handle.join();
        Ok(())
    }
}

impl Default for WindowsCapturer {
    fn default() -> Self {
        Self::new()
    }
}

fn enumerate_outputs() -> Result<Vec<OutputTarget>, String> {
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.map_err(|e| format!("CreateDXGIFactory1 failed: {e}"))?;

    let mut outputs = Vec::new();
    let mut display_id = 1u32;
    let mut adapter_index = 0u32;

    loop {
        let adapter = match unsafe { factory.EnumAdapters1(adapter_index) } {
            Ok(adapter) => adapter,
            Err(err) if err.code() == DXGI_ERROR_NOT_FOUND => break,
            Err(err) => {
                return Err(format!(
                    "EnumAdapters1 failed for adapter #{adapter_index}: {err}"
                ));
            }
        };

        let mut output_index = 0u32;
        loop {
            let output = match unsafe { adapter.EnumOutputs(output_index) } {
                Ok(output) => output,
                Err(err) if err.code() == DXGI_ERROR_NOT_FOUND => break,
                Err(err) => {
                    return Err(format!(
                        "EnumOutputs failed for adapter #{adapter_index}, output #{output_index}: {err}"
                    ));
                }
            };

            let mut desc = DXGI_OUTPUT_DESC::default();
            unsafe { output.GetDesc(&mut desc) }.map_err(|e| {
                format!("IDXGIOutput::GetDesc failed for output #{output_index}: {e}")
            })?;

            let width = desc
                .DesktopCoordinates
                .right
                .saturating_sub(desc.DesktopCoordinates.left) as u32;
            let height = desc
                .DesktopCoordinates
                .bottom
                .saturating_sub(desc.DesktopCoordinates.top) as u32;

            if width > 0 && height > 0 {
                outputs.push(OutputTarget {
                    display_id,
                    adapter_index,
                    output_index,
                    width,
                    height,
                });
                display_id = display_id.saturating_add(1);
            }

            output_index = output_index.saturating_add(1);
        }

        adapter_index = adapter_index.saturating_add(1);
    }

    Ok(outputs)
}

fn capture_thread_main(
    target: OutputTarget,
    fps: u64,
    frame_tx: SyncSender<DuplexScapFrame>,
    stop_rx: Receiver<()>,
) -> Result<(), String> {
    let (device, context, duplication) = create_duplication_session(target)?;
    let mut staging = None::<StagingTexture>;

    let timeout_ms = timeout_from_fps(fps);
    let min_frame_interval = Duration::from_micros(1_000_000u64 / fps.max(1));
    let mut last_sent = Instant::now()
        .checked_sub(min_frame_interval)
        .unwrap_or_else(Instant::now);

    loop {
        match stop_rx.try_recv() {
            Ok(()) => return Ok(()),
            Err(TryRecvError::Disconnected) => return Ok(()),
            Err(TryRecvError::Empty) => {}
        }

        let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
        let mut desktop_resource: Option<IDXGIResource> = None;

        let acquire_result = unsafe {
            duplication.AcquireNextFrame(timeout_ms, &mut frame_info, &mut desktop_resource)
        };
        match acquire_result {
            Ok(()) => {}
            Err(err) if err.code() == DXGI_ERROR_WAIT_TIMEOUT => continue,
            Err(err) if err.code() == DXGI_ERROR_ACCESS_LOST => {
                return Err("desktop duplication access lost; restart capture".to_string());
            }
            Err(err) => return Err(format!("AcquireNextFrame failed: {err}")),
        }

        let frame_result = copy_frame_to_cpu(&device, &context, desktop_resource, &mut staging);
        let _ = unsafe { duplication.ReleaseFrame() };
        let frame = frame_result?;

        let elapsed = last_sent.elapsed();
        if elapsed < min_frame_interval {
            thread::sleep(min_frame_interval - elapsed);
        }
        last_sent = Instant::now();

        match frame_tx.try_send(frame) {
            Ok(()) => {}
            Err(mpsc::TrySendError::Full(_)) => {}
            Err(mpsc::TrySendError::Disconnected(_)) => return Ok(()),
        }
    }
}

fn create_duplication_session(
    target: OutputTarget,
) -> Result<(ID3D11Device, ID3D11DeviceContext, IDXGIOutputDuplication), String> {
    let factory: IDXGIFactory1 =
        unsafe { CreateDXGIFactory1() }.map_err(|e| format!("CreateDXGIFactory1 failed: {e}"))?;
    let adapter = unsafe { factory.EnumAdapters1(target.adapter_index) }
        .map_err(|e| format!("EnumAdapters1 failed: {e}"))?;
    let output = unsafe { adapter.EnumOutputs(target.output_index) }
        .map_err(|e| format!("EnumOutputs failed: {e}"))?;
    let output1: IDXGIOutput1 = output
        .cast()
        .map_err(|e| format!("cast IDXGIOutput -> IDXGIOutput1 failed: {e}"))?;

    let (device, context) = create_d3d11_device(&adapter)?;
    let duplication = unsafe { output1.DuplicateOutput(&device) }
        .map_err(|e| format!("DuplicateOutput failed: {e}"))?;

    Ok((device, context, duplication))
}

fn create_d3d11_device(
    adapter: &IDXGIAdapter1,
) -> Result<(ID3D11Device, ID3D11DeviceContext), String> {
    let mut device = None::<ID3D11Device>;
    let mut context = None::<ID3D11DeviceContext>;

    unsafe {
        D3D11CreateDevice(
            adapter,
            D3D_DRIVER_TYPE_UNKNOWN,
            HMODULE::default(),
            D3D11_CREATE_DEVICE_BGRA_SUPPORT,
            None,
            D3D11_SDK_VERSION,
            Some(&mut device),
            None,
            Some(&mut context),
        )
        .map_err(|e| format!("D3D11CreateDevice failed: {e}"))?;
    }

    let device = device.ok_or_else(|| "D3D11CreateDevice returned null device".to_string())?;
    let context = context.ok_or_else(|| "D3D11CreateDevice returned null context".to_string())?;
    Ok((device, context))
}

fn copy_frame_to_cpu(
    device: &ID3D11Device,
    context: &ID3D11DeviceContext,
    desktop_resource: Option<IDXGIResource>,
    staging: &mut Option<StagingTexture>,
) -> Result<DuplexScapFrame, String> {
    let desktop_resource =
        desktop_resource.ok_or_else(|| "AcquireNextFrame returned empty resource".to_string())?;
    let desktop_texture: ID3D11Texture2D = desktop_resource
        .cast()
        .map_err(|e| format!("cast IDXGIResource -> ID3D11Texture2D failed: {e}"))?;

    let mut source_desc = D3D11_TEXTURE2D_DESC::default();
    unsafe { desktop_texture.GetDesc(&mut source_desc) };
    if !is_supported_format(source_desc.Format) {
        return Err(format!(
            "unsupported desktop texture format: {:?}",
            source_desc.Format
        ));
    }

    let recreate_staging = match staging {
        Some(existing) => {
            existing.width != source_desc.Width
                || existing.height != source_desc.Height
                || existing.format != source_desc.Format
        }
        None => true,
    };

    if recreate_staging {
        *staging = Some(StagingTexture {
            texture: create_staging_texture(device, &source_desc)?,
            width: source_desc.Width,
            height: source_desc.Height,
            format: source_desc.Format,
        });
    }

    let staging_texture = &staging
        .as_ref()
        .expect("staging texture should exist after creation")
        .texture;

    unsafe {
        context.CopyResource(staging_texture, &desktop_texture);
    }

    let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
    unsafe {
        context
            .Map(staging_texture, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .map_err(|e| format!("ID3D11DeviceContext::Map failed: {e}"))?;
    }

    if mapped.pData.is_null() {
        unsafe {
            context.Unmap(staging_texture, 0);
        }
        return Err("mapped desktop frame has null data pointer".to_string());
    }

    let row_pitch = mapped.RowPitch as usize;
    let height = source_desc.Height as usize;
    let byte_count = row_pitch
        .checked_mul(height)
        .ok_or_else(|| "desktop frame byte size overflow".to_string())?;
    let data =
        unsafe { std::slice::from_raw_parts(mapped.pData as *const u8, byte_count).to_vec() };

    unsafe {
        context.Unmap(staging_texture, 0);
    }

    Ok(DuplexScapFrame {
        data,
        width: source_desc.Width,
        height: source_desc.Height,
        stride: mapped.RowPitch,
        timestamp_us: unix_timestamp_us(),
    })
}

fn create_staging_texture(
    device: &ID3D11Device,
    source_desc: &D3D11_TEXTURE2D_DESC,
) -> Result<ID3D11Texture2D, String> {
    let desc = D3D11_TEXTURE2D_DESC {
        Width: source_desc.Width,
        Height: source_desc.Height,
        MipLevels: 1,
        ArraySize: 1,
        Format: source_desc.Format,
        SampleDesc: source_desc.SampleDesc,
        Usage: D3D11_USAGE_STAGING,
        BindFlags: 0,
        CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
        MiscFlags: 0,
    };

    let mut texture = None::<ID3D11Texture2D>;
    unsafe {
        device
            .CreateTexture2D(&desc, None, Some(&mut texture))
            .map_err(|e| format!("ID3D11Device::CreateTexture2D failed: {e}"))?;
    }

    texture.ok_or_else(|| "CreateTexture2D returned null staging texture".to_string())
}

fn is_supported_format(format: DXGI_FORMAT) -> bool {
    format == DXGI_FORMAT_B8G8R8A8_UNORM || format == DXGI_FORMAT_B8G8R8A8_UNORM_SRGB
}

fn timeout_from_fps(fps: u64) -> u32 {
    let fps = fps.max(1);
    let frame_ms = (1000 / fps).max(1);
    u32::try_from(frame_ms.min(500)).unwrap_or(500)
}

fn unix_timestamp_us() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| {
            let micros = d.as_micros();
            if micros > u64::MAX as u128 {
                u64::MAX
            } else {
                micros as u64
            }
        })
        .unwrap_or(0)
}
