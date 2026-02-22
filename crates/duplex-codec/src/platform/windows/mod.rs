use std::cmp::{max, min};
use std::ffi::c_void;
use std::mem::ManuallyDrop;
use std::ptr;
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver, SyncSender, TrySendError};

use duplex_scap::frame::DuplexScapFrame;
use windows::Win32::Foundation::{HMODULE, RPC_E_CHANGED_MODE};
use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
use windows::Win32::Graphics::Direct3D11::{
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_SDK_VERSION, D3D11CreateDevice, ID3D11Device,
    ID3D11DeviceContext,
};
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFDXGIDeviceManager, IMFMediaType, IMFSample, IMFTransform, MF_E_NOTACCEPTING,
    MF_E_TRANSFORM_NEED_MORE_INPUT, MF_E_TRANSFORM_STREAM_CHANGE, MF_LOW_LATENCY,
    MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE,
    MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_SA_D3D11_AWARE,
    MF_TRANSFORM_ASYNC_UNLOCK, MF_VERSION, MFCreateAlignedMemoryBuffer, MFCreateDXGIDeviceManager,
    MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample, MFMediaType_Video, MFSTARTUP_FULL,
    MFSampleExtension_CleanPoint, MFShutdown, MFStartup, MFT_CATEGORY_VIDEO_DECODER,
    MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER,
    MFT_MESSAGE_COMMAND_FLUSH, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_MESSAGE_SET_D3D_MANAGER, MFT_OUTPUT_DATA_BUFFER,
    MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES, MFT_OUTPUT_STREAM_PROVIDES_SAMPLES,
    MFT_REGISTER_TYPE_INFO, MFTEnumEx, MFVideoFormat_ARGB32, MFVideoFormat_H264,
    MFVideoFormat_NV12, MFVideoInterlace_Progressive,
};
use windows::Win32::System::Com::{
    COINIT_MULTITHREADED, CoInitializeEx, CoTaskMemFree, CoUninitialize,
};
use windows::core::{Error as WinError, GUID, IUnknown, Interface};

use crate::EncodedPacket;

pub struct PlatformVideoEncoder {
    state: Mutex<EncoderState>,
    _mf: MfThreadContext,
}

struct EncoderState {
    transform: IMFTransform,
    input_stream_id: u32,
    output_stream_id: u32,
    packet_tx: SyncSender<EncodedPacket>,
    seq_header_annexb: Option<Vec<u8>>,
    nal_len_size: usize,
    base_ts_us: Option<u64>,
    frame_duration_100ns: i64,
    _dxgi: Option<MfDxgiDeviceContext>,
}

pub struct PlatformVideoDecoder {
    state: Mutex<DecoderState>,
    _mf: MfThreadContext,
}

struct DecoderState {
    transform: IMFTransform,
    input_stream_id: u32,
    output_stream_id: u32,
    frame_tx: SyncSender<DuplexScapFrame>,
    out_fmt: DecoderOutputFormat,
    width: u32,
    height: u32,
    _dxgi: Option<MfDxgiDeviceContext>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DecoderOutputFormat {
    Argb32,
    Nv12,
}

struct MfThreadContext {
    com_inited: bool,
    mf_started: bool,
}

struct MfDxgiDeviceContext {
    _device: ID3D11Device,
    _context: ID3D11DeviceContext,
    manager: IMFDXGIDeviceManager,
}

impl MfThreadContext {
    fn init() -> Result<Self, String> {
        let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        let com_inited = if hr.is_ok() {
            true
        } else if hr == RPC_E_CHANGED_MODE {
            false
        } else {
            return Err(format!("CoInitializeEx failed: 0x{:08X}", hr.0 as u32));
        };
        unsafe { MFStartup(MF_VERSION, MFSTARTUP_FULL) }
            .map_err(|e| fmt_err("MFStartup failed", &e))?;
        Ok(Self {
            com_inited,
            mf_started: true,
        })
    }
}

impl Drop for MfThreadContext {
    fn drop(&mut self) {
        if self.mf_started {
            let _ = unsafe { MFShutdown() };
        }
        if self.com_inited {
            unsafe { CoUninitialize() };
        }
    }
}

impl MfDxgiDeviceContext {
    fn new() -> Result<Self, String> {
        let mut device = None::<ID3D11Device>;
        let mut context = None::<ID3D11DeviceContext>;
        unsafe {
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .map_err(|e| fmt_err("D3D11CreateDevice failed", &e))?;
        }
        let device = device.ok_or_else(|| "D3D11CreateDevice returned null device".to_string())?;
        let context =
            context.ok_or_else(|| "D3D11CreateDevice returned null context".to_string())?;

        let mut reset_token = 0u32;
        let mut manager = None::<IMFDXGIDeviceManager>;
        unsafe { MFCreateDXGIDeviceManager(&mut reset_token, &mut manager) }
            .map_err(|e| fmt_err("MFCreateDXGIDeviceManager failed", &e))?;
        let manager =
            manager.ok_or_else(|| "MFCreateDXGIDeviceManager returned null manager".to_string())?;
        unsafe { manager.ResetDevice(&device, reset_token) }
            .map_err(|e| fmt_err("IMFDXGIDeviceManager::ResetDevice failed", &e))?;

        Ok(Self {
            _device: device,
            _context: context,
            manager,
        })
    }
}

impl PlatformVideoEncoder {
    pub fn new(
        width: u32,
        height: u32,
        fps: u64,
        bitrate_kbps: u32,
    ) -> Result<(Self, Receiver<EncodedPacket>), String> {
        if width == 0 || height == 0 || fps == 0 {
            return Err("invalid encoder config".to_string());
        }

        let mf = MfThreadContext::init()?;
        let (transform, input_stream_id, output_stream_id, seq_header_annexb, nal_len_size, dxgi) =
            init_encoder_transform(width, height, fps, bitrate_kbps)?;
        let (tx, rx) = mpsc::sync_channel::<EncodedPacket>(1);

        Ok((
            Self {
                state: Mutex::new(EncoderState {
                    transform,
                    input_stream_id,
                    output_stream_id,
                    packet_tx: tx,
                    seq_header_annexb,
                    nal_len_size,
                    base_ts_us: None,
                    frame_duration_100ns: max(1, (10_000_000u64 / fps) as i64),
                    _dxgi: dxgi,
                }),
                _mf: mf,
            },
            rx,
        ))
    }

    pub fn encode(&self, frame: &DuplexScapFrame) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "encoder mutex poisoned".to_string())?;
        let nv12 = bgra_to_nv12(frame)?;
        let ts_us = normalized_ts_us(&mut state.base_ts_us, frame.timestamp_us);
        let sample = create_sample(&nv12, ts_us, state.frame_duration_100ns)?;

        match unsafe {
            state
                .transform
                .ProcessInput(state.input_stream_id, &sample, 0)
        } {
            Ok(()) => {}
            Err(err) if err.code() == MF_E_NOTACCEPTING => {
                drain_encoder(&mut state)?;
                unsafe {
                    state
                        .transform
                        .ProcessInput(state.input_stream_id, &sample, 0)
                }
                .map_err(|e| fmt_err("encoder ProcessInput failed", &e))?;
            }
            Err(err) => return Err(fmt_err("encoder ProcessInput failed", &err)),
        }

        drain_encoder(&mut state)
    }
}

impl PlatformVideoDecoder {
    pub fn from_keyframe(
        packet: &EncodedPacket,
    ) -> Result<(Self, Receiver<DuplexScapFrame>), String> {
        if !packet.is_keyframe {
            return Err("decoder init requires keyframe".to_string());
        }

        let mf = MfThreadContext::init()?;
        let transform =
            create_transform(MFT_CATEGORY_VIDEO_DECODER, None, Some(MFVideoFormat_H264))?;
        unlock_transform_async_mode(&transform);
        let dxgi = set_transform_d3d_manager(&transform)?;
        let (input_stream_id, output_stream_id) = resolve_stream_ids(&transform)?;
        configure_decoder_types(&transform, input_stream_id)?;
        start_streaming(&transform)?;
        let (fmt, w, h) = read_decoder_output_format(&transform, output_stream_id)?;
        let (tx, rx) = mpsc::sync_channel::<DuplexScapFrame>(1);

        Ok((
            Self {
                state: Mutex::new(DecoderState {
                    transform,
                    input_stream_id,
                    output_stream_id,
                    frame_tx: tx,
                    out_fmt: fmt,
                    width: w,
                    height: h,
                    _dxgi: dxgi,
                }),
                _mf: mf,
            },
            rx,
        ))
    }

    pub fn decode(&self, packet: &EncodedPacket) -> Result<(), String> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| "decoder mutex poisoned".to_string())?;
        let sample = create_sample(&packet.data, packet.timestamp_us, 0)?;
        if packet.is_keyframe {
            let _ = unsafe { sample.SetUINT32(&MFSampleExtension_CleanPoint, 1) };
        }

        match unsafe {
            state
                .transform
                .ProcessInput(state.input_stream_id, &sample, 0)
        } {
            Ok(()) => {}
            Err(err) if err.code() == MF_E_NOTACCEPTING => {
                drain_decoder(&mut state)?;
                unsafe {
                    state
                        .transform
                        .ProcessInput(state.input_stream_id, &sample, 0)
                }
                .map_err(|e| fmt_err("decoder ProcessInput failed", &e))?;
            }
            Err(err) => return Err(fmt_err("decoder ProcessInput failed", &err)),
        }

        drain_decoder(&mut state)
    }
}

fn create_transform(
    category: GUID,
    out_sub: Option<GUID>,
    in_sub: Option<GUID>,
) -> Result<IMFTransform, String> {
    create_transforms(category, out_sub, in_sub)?
        .into_iter()
        .next()
        .ok_or_else(|| "no usable MF transform found".to_string())
}

fn create_transforms(
    category: GUID,
    out_sub: Option<GUID>,
    in_sub: Option<GUID>,
) -> Result<Vec<IMFTransform>, String> {
    let mut in_info = MFT_REGISTER_TYPE_INFO::default();
    let mut out_info = MFT_REGISTER_TYPE_INFO::default();
    let in_ptr = in_sub.map(|s| {
        in_info.guidMajorType = MFMediaType_Video;
        in_info.guidSubtype = s;
        &in_info as *const _
    });
    let out_ptr = out_sub.map(|s| {
        out_info.guidMajorType = MFMediaType_Video;
        out_info.guidSubtype = s;
        &out_info as *const _
    });

    let mut transforms = Vec::<IMFTransform>::new();
    for flags in [
        MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_SORTANDFILTER,
        MFT_ENUM_FLAG_SORTANDFILTER,
    ] {
        let mut acts: *mut Option<IMFActivate> = ptr::null_mut();
        let mut count = 0u32;
        if unsafe { MFTEnumEx(category, flags, in_ptr, out_ptr, &mut acts, &mut count) }.is_ok()
            && !acts.is_null()
            && count > 0
        {
            let entries: Vec<IMFActivate> = unsafe {
                let arr = std::slice::from_raw_parts(acts, count as usize);
                arr.iter().flatten().cloned().collect()
            };
            unsafe { CoTaskMemFree(Some(acts as *const c_void)) };
            for act in entries {
                if let Ok(transform) = unsafe { act.ActivateObject::<IMFTransform>() } {
                    transforms.push(transform);
                }
            }
        } else if !acts.is_null() {
            unsafe { CoTaskMemFree(Some(acts as *const c_void)) };
        }
    }

    if transforms.is_empty() {
        Err("no usable MF transform found".to_string())
    } else {
        Ok(transforms)
    }
}

fn unlock_transform_async_mode(transform: &IMFTransform) {
    if let Ok(attrs) = unsafe { transform.GetAttributes() } {
        unsafe {
            let _ = attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1);
            let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);
        }
    }
}

fn create_video_type(
    subtype: GUID,
    width: u32,
    height: u32,
    fps: u64,
) -> Result<IMFMediaType, String> {
    let mt = unsafe { MFCreateMediaType() }.map_err(|e| fmt_err("MFCreateMediaType failed", &e))?;
    unsafe {
        mt.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|e| fmt_err("SetGUID major failed", &e))?;
        mt.SetGUID(&MF_MT_SUBTYPE, &subtype)
            .map_err(|e| fmt_err("SetGUID subtype failed", &e))?;
        mt.SetUINT64(&MF_MT_FRAME_SIZE, pack_u32(width, height))
            .map_err(|e| fmt_err("SetUINT64 frame_size failed", &e))?;
        mt.SetUINT64(
            &MF_MT_FRAME_RATE,
            pack_u32(min(fps, u32::MAX as u64) as u32, 1),
        )
        .map_err(|e| fmt_err("SetUINT64 frame_rate failed", &e))?;
        mt.SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u32(1, 1))
            .map_err(|e| fmt_err("SetUINT64 aspect failed", &e))?;
        mt.SetUINT32(&MF_MT_INTERLACE_MODE, MFVideoInterlace_Progressive.0 as u32)
            .map_err(|e| fmt_err("SetUINT32 interlace failed", &e))?;
    }
    Ok(mt)
}

fn init_encoder_transform(
    width: u32,
    height: u32,
    fps: u64,
    bitrate_kbps: u32,
) -> Result<
    (
        IMFTransform,
        u32,
        u32,
        Option<Vec<u8>>,
        usize,
        Option<MfDxgiDeviceContext>,
    ),
    String,
> {
    let mut last_err = None::<String>;
    for transform in create_transforms(MFT_CATEGORY_VIDEO_ENCODER, Some(MFVideoFormat_H264), None)?
    {
        match try_init_encoder_candidate(transform, width, height, fps, bitrate_kbps) {
            Ok(v) => return Ok(v),
            Err(err) => last_err = Some(err),
        }
    }
    Err(last_err.unwrap_or_else(|| "no encoder transform could be initialized".to_string()))
}

fn try_init_encoder_candidate(
    transform: IMFTransform,
    width: u32,
    height: u32,
    fps: u64,
    bitrate_kbps: u32,
) -> Result<
    (
        IMFTransform,
        u32,
        u32,
        Option<Vec<u8>>,
        usize,
        Option<MfDxgiDeviceContext>,
    ),
    String,
> {
    unlock_transform_async_mode(&transform);
    let dxgi = set_transform_d3d_manager(&transform)?;
    let (input_stream_id, output_stream_id) = resolve_stream_ids(&transform)?;

    let out = create_video_type(MFVideoFormat_H264, width, height, fps)?;
    unsafe {
        out.SetUINT32(&MF_MT_AVG_BITRATE, bitrate_kbps.saturating_mul(1000))
            .map_err(|e| fmt_err("SetUINT32(MF_MT_AVG_BITRATE) failed", &e))?;
    }
    unsafe { transform.SetOutputType(output_stream_id, &out, 0) }
        .map_err(|e| fmt_err("encoder SetOutputType failed", &e))?;

    let inp = create_video_type(MFVideoFormat_NV12, width, height, fps)?;
    unsafe { transform.SetInputType(input_stream_id, &inp, 0) }
        .map_err(|e| fmt_err("encoder SetInputType(NV12) failed", &e))?;

    start_streaming(&transform)?;
    probe_encoder_transform(
        &transform,
        input_stream_id,
        output_stream_id,
        max(1, (10_000_000u64 / fps) as i64),
        width,
        height,
    )?;
    start_streaming(&transform)?;
    let (seq_header_annexb, nal_len_size) = read_seq_header(&transform, output_stream_id)?;

    Ok((
        transform,
        input_stream_id,
        output_stream_id,
        seq_header_annexb,
        nal_len_size,
        dxgi,
    ))
}

fn probe_encoder_transform(
    transform: &IMFTransform,
    input_stream_id: u32,
    output_stream_id: u32,
    frame_duration_100ns: i64,
    width: u32,
    height: u32,
) -> Result<(), String> {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || (w & 1) != 0 || (h & 1) != 0 {
        return Err(format!("unsupported probe frame size {}x{}", width, height));
    }
    let y_size = w * h;
    let uv_size = w * (h / 2);
    let mut nv12 = vec![0u8; y_size + uv_size];
    nv12[..y_size].fill(16);
    nv12[y_size..].fill(128);

    let sample = create_sample(&nv12, 0, frame_duration_100ns)?;
    unsafe { transform.ProcessInput(input_stream_id, &sample, 0) }
        .map_err(|e| fmt_err("encoder probe ProcessInput failed", &e))?;

    loop {
        let mut out = build_output_data_buffer(transform, output_stream_id)?;
        let mut status = 0u32;
        match unsafe { transform.ProcessOutput(0, std::slice::from_mut(&mut out), &mut status) } {
            Ok(()) => {
                let _ = unsafe { ManuallyDrop::take(&mut out.pSample) };
                let _ = unsafe { ManuallyDrop::take(&mut out.pEvents) };
            }
            Err(err) if err.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
            Err(err) if err.code() == MF_E_TRANSFORM_STREAM_CHANGE => continue,
            Err(err) => return Err(fmt_err("encoder probe ProcessOutput failed", &err)),
        }
    }

    Ok(())
}

fn start_streaming(transform: &IMFTransform) -> Result<(), String> {
    let _ = unsafe { transform.ProcessMessage(MFT_MESSAGE_COMMAND_FLUSH, 0) };
    unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0) }
        .map_err(|e| fmt_err("ProcessMessage BEGIN failed", &e))?;
    unsafe { transform.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0) }
        .map_err(|e| fmt_err("ProcessMessage START failed", &e))
}

fn resolve_stream_ids(transform: &IMFTransform) -> Result<(u32, u32), String> {
    let mut input_count = 0u32;
    let mut output_count = 0u32;
    unsafe { transform.GetStreamCount(&mut input_count, &mut output_count) }
        .map_err(|e| fmt_err("GetStreamCount failed", &e))?;
    if input_count == 0 || output_count == 0 {
        return Err(format!(
            "invalid stream count input={} output={}",
            input_count, output_count
        ));
    }

    let mut input_ids = vec![0u32; input_count as usize];
    let mut output_ids = vec![0u32; output_count as usize];
    let ids_ok = unsafe { transform.GetStreamIDs(&mut input_ids, &mut output_ids) }.is_ok();
    if !ids_ok {
        for (i, v) in input_ids.iter_mut().enumerate() {
            *v = i as u32;
        }
        for (i, v) in output_ids.iter_mut().enumerate() {
            *v = i as u32;
        }
    }

    Ok((input_ids[0], output_ids[0]))
}

fn set_transform_d3d_manager(
    transform: &IMFTransform,
) -> Result<Option<MfDxgiDeviceContext>, String> {
    let d3d11_aware = if let Ok(attrs) = unsafe { transform.GetAttributes() } {
        let d3d11 = unsafe { attrs.GetUINT32(&MF_SA_D3D11_AWARE) }
            .ok()
            .unwrap_or(0);
        d3d11 != 0
    } else {
        false
    };

    if !d3d11_aware {
        return Ok(None);
    }

    let dxgi = MfDxgiDeviceContext::new()?;
    let unknown: IUnknown = dxgi
        .manager
        .cast()
        .map_err(|e| fmt_err("cast IMFDXGIDeviceManager to IUnknown failed", &e))?;
    unsafe { transform.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, unknown.as_raw() as usize) }
        .map_err(|e| fmt_err("ProcessMessage SET_D3D_MANAGER failed", &e))?;
    Ok(Some(dxgi))
}

fn normalized_ts_us(base: &mut Option<u64>, ts_us: u64) -> u64 {
    let base = *base.get_or_insert(ts_us);
    ts_us.saturating_sub(base)
}

fn create_sample(data: &[u8], ts_us: u64, dur_100ns: i64) -> Result<IMFSample, String> {
    if data.len() > u32::MAX as usize {
        return Err("sample too large".to_string());
    }
    let sample = unsafe { MFCreateSample() }.map_err(|e| fmt_err("MFCreateSample failed", &e))?;
    let buf = unsafe { MFCreateMemoryBuffer(data.len() as u32) }
        .map_err(|e| fmt_err("MFCreateMemoryBuffer failed", &e))?;
    unsafe {
        let mut p = ptr::null_mut();
        buf.Lock(&mut p, None, None)
            .map_err(|e| fmt_err("buffer Lock failed", &e))?;
        ptr::copy_nonoverlapping(data.as_ptr(), p, data.len());
        buf.Unlock()
            .map_err(|e| fmt_err("buffer Unlock failed", &e))?;
        buf.SetCurrentLength(data.len() as u32)
            .map_err(|e| fmt_err("SetCurrentLength failed", &e))?;
        sample
            .AddBuffer(&buf)
            .map_err(|e| fmt_err("sample AddBuffer failed", &e))?;
        sample
            .SetSampleTime((ts_us as i64).saturating_mul(10))
            .map_err(|e| fmt_err("SetSampleTime failed", &e))?;
        sample
            .SetSampleDuration(dur_100ns)
            .map_err(|e| fmt_err("SetSampleDuration failed", &e))?;
    }
    Ok(sample)
}

fn drain_encoder(state: &mut EncoderState) -> Result<(), String> {
    loop {
        let mut out = build_output_data_buffer(&state.transform, state.output_stream_id)?;
        let mut status = 0u32;
        match unsafe {
            state
                .transform
                .ProcessOutput(0, std::slice::from_mut(&mut out), &mut status)
        } {
            Ok(()) => {
                let sample = unsafe { ManuallyDrop::take(&mut out.pSample) };
                let _ = unsafe { ManuallyDrop::take(&mut out.pEvents) };
                if let Some(sample) = sample {
                    if let Some(packet) = map_encoded_sample(
                        &sample,
                        state.seq_header_annexb.as_deref(),
                        state.nal_len_size,
                    )? {
                        match state.packet_tx.try_send(packet) {
                            Ok(()) | Err(TrySendError::Full(_)) => {}
                            Err(TrySendError::Disconnected(_)) => return Ok(()),
                        }
                    }
                }
            }
            Err(err) if err.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
            Err(err) if err.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                let (seq, len_size) = read_seq_header(&state.transform, state.output_stream_id)?;
                state.seq_header_annexb = seq;
                state.nal_len_size = len_size;
                continue;
            }
            Err(err) => return Err(fmt_err("encoder ProcessOutput failed", &err)),
        }
    }
    Ok(())
}

fn map_encoded_sample(
    sample: &IMFSample,
    seq_header: Option<&[u8]>,
    nal_len: usize,
) -> Result<Option<EncodedPacket>, String> {
    let data = sample_bytes(sample)?;
    if data.is_empty() {
        return Ok(None);
    }

    let mut annexb = if is_annexb(&data) {
        data
    } else {
        avcc_to_annexb(&data, nal_len)?
    };
    let clean = unsafe { sample.GetUINT32(&MFSampleExtension_CleanPoint) }
        .ok()
        .unwrap_or(0)
        != 0;
    let is_key = clean || has_idr(&annexb);
    if is_key && !has_sps_pps(&annexb) {
        if let Some(s) = seq_header {
            let mut merged = s.to_vec();
            merged.extend_from_slice(&annexb);
            annexb = merged;
        }
    }

    let ts_100ns = unsafe { sample.GetSampleTime() }.ok().unwrap_or(0);
    Ok(Some(EncodedPacket {
        data: annexb,
        is_keyframe: is_key,
        timestamp_us: if ts_100ns <= 0 {
            0
        } else {
            (ts_100ns as u64) / 10
        },
    }))
}

fn configure_decoder_types(transform: &IMFTransform, input_stream_id: u32) -> Result<(), String> {
    let in_t = unsafe { MFCreateMediaType() }
        .map_err(|e| fmt_err("MFCreateMediaType decoder input failed", &e))?;
    unsafe {
        in_t.SetGUID(&MF_MT_MAJOR_TYPE, &MFMediaType_Video)
            .map_err(|e| fmt_err("decoder input major failed", &e))?;
        in_t.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_H264)
            .map_err(|e| fmt_err("decoder input subtype failed", &e))?;
    }
    unsafe { transform.SetInputType(input_stream_id, &in_t, 0) }
        .map_err(|e| fmt_err("decoder SetInputType failed", &e))
}

fn read_decoder_output_format(
    transform: &IMFTransform,
    output_stream_id: u32,
) -> Result<(DecoderOutputFormat, u32, u32), String> {
    let mut chosen: Option<(IMFMediaType, DecoderOutputFormat)> = None;
    let mut idx = 0u32;
    loop {
        let t = match unsafe { transform.GetOutputAvailableType(output_stream_id, idx) } {
            Ok(v) => v,
            Err(_) => break,
        };
        let sub = unsafe { t.GetGUID(&MF_MT_SUBTYPE) }
            .ok()
            .unwrap_or(GUID::zeroed());
        if sub == MFVideoFormat_ARGB32 {
            chosen = Some((t, DecoderOutputFormat::Argb32));
            break;
        }
        if sub == MFVideoFormat_NV12 && chosen.is_none() {
            chosen = Some((t, DecoderOutputFormat::Nv12));
        }
        idx = idx.saturating_add(1);
    }
    let (ty, fmt) = chosen.ok_or_else(|| "decoder has no ARGB32/NV12 output".to_string())?;
    unsafe { transform.SetOutputType(output_stream_id, &ty, 0) }
        .map_err(|e| fmt_err("decoder SetOutputType failed", &e))?;
    let packed = unsafe { ty.GetUINT64(&MF_MT_FRAME_SIZE) }.ok().unwrap_or(0);
    Ok((fmt, (packed >> 32) as u32, packed as u32))
}

fn drain_decoder(state: &mut DecoderState) -> Result<(), String> {
    loop {
        let mut out = build_output_data_buffer(&state.transform, state.output_stream_id)?;
        let mut status = 0u32;
        match unsafe {
            state
                .transform
                .ProcessOutput(0, std::slice::from_mut(&mut out), &mut status)
        } {
            Ok(()) => {
                let sample = unsafe { ManuallyDrop::take(&mut out.pSample) };
                let _ = unsafe { ManuallyDrop::take(&mut out.pEvents) };
                if let Some(sample) = sample {
                    if let Some(frame) =
                        map_decoded_sample(&sample, state.out_fmt, state.width, state.height)?
                    {
                        match state.frame_tx.try_send(frame) {
                            Ok(()) | Err(TrySendError::Full(_)) => {}
                            Err(TrySendError::Disconnected(_)) => return Ok(()),
                        }
                    }
                }
            }
            Err(err) if err.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
            Err(err) if err.code() == MF_E_TRANSFORM_STREAM_CHANGE => {
                let (fmt, w, h) =
                    read_decoder_output_format(&state.transform, state.output_stream_id)?;
                state.out_fmt = fmt;
                if w > 0 {
                    state.width = w;
                }
                if h > 0 {
                    state.height = h;
                }
                continue;
            }
            Err(err) => return Err(fmt_err("decoder ProcessOutput failed", &err)),
        }
    }
    Ok(())
}

fn map_decoded_sample(
    sample: &IMFSample,
    fmt: DecoderOutputFormat,
    width: u32,
    height: u32,
) -> Result<Option<DuplexScapFrame>, String> {
    let data = sample_bytes(sample)?;
    if data.is_empty() || width == 0 || height == 0 {
        return Ok(None);
    }
    let ts_100ns = unsafe { sample.GetSampleTime() }.ok().unwrap_or(0);
    let ts_us = if ts_100ns <= 0 {
        0
    } else {
        (ts_100ns as u64) / 10
    };

    match fmt {
        DecoderOutputFormat::Argb32 => Ok(Some(DuplexScapFrame {
            data,
            width,
            height,
            stride: width.saturating_mul(4),
            timestamp_us: ts_us,
        })),
        DecoderOutputFormat::Nv12 => {
            let bgra = nv12_to_bgra(&data, width, height)?;
            Ok(Some(DuplexScapFrame {
                data: bgra,
                width,
                height,
                stride: width.saturating_mul(4),
                timestamp_us: ts_us,
            }))
        }
    }
}

fn build_output_data_buffer(
    transform: &IMFTransform,
    output_stream_id: u32,
) -> Result<MFT_OUTPUT_DATA_BUFFER, String> {
    let info = unsafe { transform.GetOutputStreamInfo(output_stream_id) }
        .map_err(|e| fmt_err("GetOutputStreamInfo failed", &e))?;

    let needs_sample = (info.dwFlags & (MFT_OUTPUT_STREAM_PROVIDES_SAMPLES.0 as u32)) == 0
        && (info.dwFlags & (MFT_OUTPUT_STREAM_CAN_PROVIDE_SAMPLES.0 as u32)) == 0;

    let sample = if needs_sample {
        let sample = unsafe { MFCreateSample() }
            .map_err(|e| fmt_err("MFCreateSample(output) failed", &e))?;
        let size = max(8_388_608u32, info.cbSize);
        let buffer = if info.cbAlignment > 1 {
            unsafe { MFCreateAlignedMemoryBuffer(size, info.cbAlignment) }
                .map_err(|e| fmt_err("MFCreateAlignedMemoryBuffer(output) failed", &e))?
        } else {
            unsafe { MFCreateMemoryBuffer(size) }
                .map_err(|e| fmt_err("MFCreateMemoryBuffer(output) failed", &e))?
        };
        unsafe {
            sample
                .AddBuffer(&buffer)
                .map_err(|e| fmt_err("AddBuffer(output) failed", &e))?;
        }
        Some(sample)
    } else {
        None
    };

    Ok(MFT_OUTPUT_DATA_BUFFER {
        dwStreamID: output_stream_id,
        pSample: ManuallyDrop::new(sample),
        dwStatus: 0,
        pEvents: ManuallyDrop::new(None),
    })
}

fn sample_bytes(sample: &IMFSample) -> Result<Vec<u8>, String> {
    let buf = unsafe { sample.ConvertToContiguousBuffer() }
        .map_err(|e| fmt_err("ConvertToContiguousBuffer failed", &e))?;
    unsafe {
        let mut p = ptr::null_mut();
        let mut max_len = 0u32;
        let mut cur_len = 0u32;
        buf.Lock(&mut p, Some(&mut max_len), Some(&mut cur_len))
            .map_err(|e| fmt_err("buffer Lock failed", &e))?;
        let out = if p.is_null() || cur_len == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(p, cur_len as usize).to_vec()
        };
        buf.Unlock()
            .map_err(|e| fmt_err("buffer Unlock failed", &e))?;
        Ok(out)
    }
}

fn read_seq_header(
    transform: &IMFTransform,
    output_stream_id: u32,
) -> Result<(Option<Vec<u8>>, usize), String> {
    let cur = match unsafe { transform.GetOutputCurrentType(output_stream_id) } {
        Ok(v) => v,
        Err(_) => return Ok((None, 4)),
    };
    unsafe {
        let mut p = ptr::null_mut();
        let mut n = 0u32;
        match cur.GetAllocatedBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut p, &mut n) {
            Ok(()) => {
                if p.is_null() || n == 0 {
                    if !p.is_null() {
                        CoTaskMemFree(Some(p as *const c_void));
                    }
                    return Ok((None, 4));
                }
                let blob = std::slice::from_raw_parts(p, n as usize).to_vec();
                CoTaskMemFree(Some(p as *const c_void));
                if let Some((annexb, len)) = avcc_seq_to_annexb(&blob) {
                    Ok((Some(annexb), len))
                } else {
                    Ok((None, 4))
                }
            }
            Err(_) => Ok((None, 4)),
        }
    }
}

fn avcc_seq_to_annexb(avcc: &[u8]) -> Option<(Vec<u8>, usize)> {
    if avcc.len() < 7 || avcc[0] != 1 {
        return None;
    }
    let nal_len = ((avcc[4] & 0x03) as usize) + 1;
    let mut i = 5usize;
    let sps_n = (avcc[i] & 0x1F) as usize;
    i += 1;
    let mut out = Vec::new();
    for _ in 0..sps_n {
        if i + 2 > avcc.len() {
            return None;
        }
        let n = u16::from_be_bytes([avcc[i], avcc[i + 1]]) as usize;
        i += 2;
        if i + n > avcc.len() {
            return None;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&avcc[i..i + n]);
        i += n;
    }
    if i >= avcc.len() {
        return None;
    }
    let pps_n = avcc[i] as usize;
    i += 1;
    for _ in 0..pps_n {
        if i + 2 > avcc.len() {
            return None;
        }
        let n = u16::from_be_bytes([avcc[i], avcc[i + 1]]) as usize;
        i += 2;
        if i + n > avcc.len() {
            return None;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&avcc[i..i + n]);
        i += n;
    }
    Some((out, nal_len))
}

fn avcc_to_annexb(data: &[u8], len_size: usize) -> Result<Vec<u8>, String> {
    let len_size = len_size.clamp(1, 4);
    let mut i = 0usize;
    let mut out = Vec::with_capacity(data.len() + 64);
    while i + len_size <= data.len() {
        let mut n = 0usize;
        for b in &data[i..i + len_size] {
            n = (n << 8) | (*b as usize);
        }
        i += len_size;
        if n == 0 || i + n > data.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&data[i..i + n]);
        i += n;
    }
    if out.is_empty() {
        return Err("invalid AVCC payload".to_string());
    }
    Ok(out)
}

fn is_annexb(data: &[u8]) -> bool {
    data.starts_with(&[0, 0, 1]) || data.starts_with(&[0, 0, 0, 1])
}

fn has_idr(annexb: &[u8]) -> bool {
    split_annexb(annexb)
        .iter()
        .any(|n| !n.is_empty() && (n[0] & 0x1F) == 5)
}

fn has_sps_pps(annexb: &[u8]) -> bool {
    let mut sps = false;
    let mut pps = false;
    for n in split_annexb(annexb) {
        if n.is_empty() {
            continue;
        }
        match n[0] & 0x1F {
            7 => sps = true,
            8 => pps = true,
            _ => {}
        }
    }
    sps && pps
}

fn split_annexb(data: &[u8]) -> Vec<&[u8]> {
    let mut out = Vec::new();
    let mut start = 0usize;
    let mut i = 0usize;
    while i + 3 <= data.len() {
        let sc = (data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1)
            || (i + 4 <= data.len()
                && data[i] == 0
                && data[i + 1] == 0
                && data[i + 2] == 0
                && data[i + 3] == 1);
        if sc {
            if start < i {
                out.push(&data[start..i]);
            }
            start = if data[i + 2] == 1 { i + 3 } else { i + 4 };
            i = start;
        } else {
            i += 1;
        }
    }
    if start < data.len() {
        out.push(&data[start..]);
    }
    out
}

fn bgra_to_nv12(frame: &DuplexScapFrame) -> Result<Vec<u8>, String> {
    let w = frame.width as usize;
    let h = frame.height as usize;
    let stride = frame.stride as usize;
    let row = w.saturating_mul(4);
    if w == 0 || h == 0 || stride < row || frame.data.len() < stride.saturating_mul(h) {
        return Err("invalid BGRA frame".to_string());
    }
    if (w & 1) != 0 || (h & 1) != 0 {
        return Err(format!(
            "NV12 requires even width/height, got {}x{}",
            frame.width, frame.height
        ));
    }

    let y_size = w.saturating_mul(h);
    let uv_size = w.saturating_mul(h / 2);
    let mut out = vec![0u8; y_size + uv_size];

    for y in 0..h {
        let src_row = y * stride;
        let y_row = y * w;
        for x in 0..w {
            let si = src_row + x * 4;
            let b = frame.data[si] as i32;
            let g = frame.data[si + 1] as i32;
            let r = frame.data[si + 2] as i32;
            let luma = ((66 * r + 129 * g + 25 * b + 128) >> 8) + 16;
            out[y_row + x] = clamp(luma);
        }
    }

    for y in (0..h).step_by(2) {
        let src_row0 = y * stride;
        let src_row1 = (y + 1) * stride;
        let uv_row = y_size + (y / 2) * w;
        for x in (0..w).step_by(2) {
            let p00 = src_row0 + x * 4;
            let p01 = src_row0 + (x + 1) * 4;
            let p10 = src_row1 + x * 4;
            let p11 = src_row1 + (x + 1) * 4;

            let mut sum_u = 0i32;
            let mut sum_v = 0i32;
            for p in [p00, p01, p10, p11] {
                let b = frame.data[p] as i32;
                let g = frame.data[p + 1] as i32;
                let r = frame.data[p + 2] as i32;
                sum_u += ((-38 * r - 74 * g + 112 * b + 128) >> 8) + 128;
                sum_v += ((112 * r - 94 * g - 18 * b + 128) >> 8) + 128;
            }

            let ui = uv_row + x;
            out[ui] = clamp(sum_u / 4);
            out[ui + 1] = clamp(sum_v / 4);
        }
    }

    Ok(out)
}

fn nv12_to_bgra(data: &[u8], width: u32, height: u32) -> Result<Vec<u8>, String> {
    let w = width as usize;
    let h = height as usize;
    if w == 0 || h == 0 || (w & 1) != 0 || (h & 1) != 0 {
        return Err("invalid NV12 size".to_string());
    }
    let y_stride = w;
    let uv_stride = w;
    let y_size = y_stride * h;
    let uv_size = uv_stride * (h / 2);
    if data.len() < y_size + uv_size {
        return Err("NV12 data too small".to_string());
    }
    let y_plane = &data[..y_size];
    let uv_plane = &data[y_size..y_size + uv_size];
    let mut out = vec![0u8; w * h * 4];
    for y in 0..h {
        for x in 0..w {
            let yv = y_plane[y * y_stride + x] as i32;
            let uv = (y / 2) * uv_stride + (x / 2) * 2;
            let u = uv_plane[uv] as i32;
            let v = uv_plane[uv + 1] as i32;
            let c = max(0, yv - 16);
            let d = u - 128;
            let e = v - 128;
            let r = clamp((298 * c + 409 * e + 128) >> 8);
            let g = clamp((298 * c - 100 * d - 208 * e + 128) >> 8);
            let b = clamp((298 * c + 516 * d + 128) >> 8);
            let di = (y * w + x) * 4;
            out[di] = b;
            out[di + 1] = g;
            out[di + 2] = r;
            out[di + 3] = 255;
        }
    }
    Ok(out)
}

fn clamp(v: i32) -> u8 {
    if v < 0 {
        0
    } else if v > 255 {
        255
    } else {
        v as u8
    }
}

fn pack_u32(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) | (lo as u64)
}

fn fmt_err(ctx: &str, err: &WinError) -> String {
    format!("{ctx}: {err} (0x{:08X})", err.code().0 as u32)
}
