use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::mem::ManuallyDrop;

use windows::core::{Interface, GUID, VARIANT};
use windows::Graphics::Capture::{
    Direct3D11CaptureFramePool, GraphicsCaptureItem, GraphicsCaptureSession,
};
use windows::Graphics::DirectX::Direct3D11::IDirect3DDevice;
use windows::Graphics::DirectX::DirectXPixelFormat;
use windows::Graphics::SizeInt32;
use windows::Win32::Foundation::{HMODULE, POINT, RECT};
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
    ID3D11VideoContext, ID3D11VideoDevice, ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator,
    ID3D11VideoProcessorInputView, ID3D11VideoProcessorOutputView, D3D11_BIND_RENDER_TARGET,
    D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT, D3D11_SDK_VERSION,
    D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
    D3D11_VIDEO_PROCESSOR_COLOR_SPACE, D3D11_VIDEO_PROCESSOR_CONTENT_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
    D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D, D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_NV12, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::{
    CreateDXGIFactory1, IDXGIDevice, IDXGIFactory1, IDXGIOutput1, IDXGIOutputDuplication,
    DXGI_ERROR_WAIT_TIMEOUT, DXGI_OUTDUPL_FRAME_INFO,
};
use windows::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};
use windows::Win32::Media::MediaFoundation::{
    CODECAPI_AVEncCommonBufferSize, CODECAPI_AVEncCommonMaxBitRate, CODECAPI_AVEncCommonMeanBitRate,
    CODECAPI_AVEncCommonQuality, CODECAPI_AVEncCommonQualityVsSpeed,
    CODECAPI_AVEncCommonRateControlMode,
    CODECAPI_AVEncMPVDefaultBPictureCount, CODECAPI_AVEncMPVGOPSize, CODECAPI_AVEncVideoEncodeQP,
    CODECAPI_AVLowLatencyMode, ICodecAPI, IMFActivate, IMFDXGIDeviceManager, IMFMediaBuffer,
    IMFMediaEventGenerator, IMFSample, IMFTransform, MFCreateDXGIDeviceManager,
    MFCreateDXGISurfaceBuffer, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
    MFShutdown, MFStartup, MFTEnumEx, METransformHaveOutput, METransformNeedInput,
    MFSampleExtension_CleanPoint, MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_ASYNCMFT,
    MFT_ENUM_FLAG_HARDWARE, MFT_ENUM_FLAG_SORTANDFILTER,
    MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, MFT_MESSAGE_NOTIFY_START_OF_STREAM,
    MFT_MESSAGE_SET_D3D_MANAGER, MFT_OUTPUT_DATA_BUFFER, MFT_REGISTER_TYPE_INFO,
    MFVideoFormat_ARGB32, MFVideoFormat_H264, MFVideoFormat_HEVC, MFVideoFormat_NV12,
    MFSTARTUP_NOSOCKET, MF_EVENT_FLAG_NO_WAIT, MF_E_TRANSFORM_NEED_MORE_INPUT,
    MF_E_TRANSFORM_STREAM_CHANGE, MF_MT_AVG_BITRATE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE,
    MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MPEG_SEQUENCE_HEADER, MF_MT_SUBTYPE,
    MF_TRANSFORM_ASYNC, MF_TRANSFORM_ASYNC_UNLOCK, MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};
use windows::Win32::System::WinRT::Direct3D11::{
    CreateDirect3D11DeviceFromDXGIDevice, IDirect3DDxgiInterfaceAccess,
};
use windows::Win32::System::WinRT::Graphics::Capture::IGraphicsCaptureItemInterop;
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::config::{AppConfig, EncoderPreference, RateControl, VideoCodec};
use crate::ring_buffer::{EncodedPacket, PacketTrack};
use crate::win_ffi::{from_wide_slice, qpc_now_100ns, HighResTimer};

const MF_MEDIATYPE_VIDEO: GUID = GUID::from_u128(0x73646976_0000_0010_8000_00aa00389b71);
const MF_VIDEO_FORMAT_AV1: GUID = GUID::from_u128(0x31305641_0000_0010_8000_00aa00389b71);

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HardwareInfo {
    pub gpu_name: String,
    pub vendor_id: u32,
    pub encoder_backend: String,
    pub vram_mb: u32,
    pub supports_hevc: bool,
    pub supports_av1: bool,
    pub supports_h264: bool,
    pub wgc_available: bool,
}

impl Default for HardwareInfo {
    fn default() -> Self {
        Self {
            gpu_name: "Unknown GPU".to_owned(),
            vendor_id: 0,
            encoder_backend: "Hardware MFT".to_owned(),
            vram_mb: 0,
            supports_hevc: false,
            supports_av1: false,
            supports_h264: true,
            wgc_available: false,
        }
    }
}

impl HardwareInfo {
    pub fn native_encoder(&self) -> EncoderPreference {
        match self.vendor_id {
            0x10DE => EncoderPreference::Nvenc,
            0x1002 => EncoderPreference::Amf,
            0x8086 => EncoderPreference::Qsv,
            _ => EncoderPreference::Auto,
        }
    }

    pub fn is_encoder_supported(&self, enc: EncoderPreference) -> bool {
        match enc {
            EncoderPreference::Auto => true,
            EncoderPreference::Nvenc => self.vendor_id == 0x10DE,
            EncoderPreference::Amf => self.vendor_id == 0x1002,
            EncoderPreference::Qsv => self.vendor_id == 0x8086,
        }
    }

    pub fn is_codec_supported(&self, codec: VideoCodec) -> bool {
        match codec {
            VideoCodec::Hevc => self.supports_hevc,
            VideoCodec::Av1 => self.supports_av1,
            VideoCodec::H264 => self.supports_h264,
        }
    }
}

pub fn detect_hardware() -> HardwareInfo {
    let mut info = HardwareInfo::default();
    info.wgc_available = GraphicsCaptureSession::IsSupported().unwrap_or(false);

    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        let _ = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);

        if let Ok(factory) = CreateDXGIFactory1::<IDXGIFactory1>() {
            if let Ok(adapter) = factory.EnumAdapters1(0) {
                if let Ok(desc) = adapter.GetDesc1() {
                    info.gpu_name = from_wide_slice(&desc.Description).trim().to_owned();
                    info.vendor_id = desc.VendorId;
                    info.vram_mb = (desc.DedicatedVideoMemory / (1024 * 1024)) as u32;
                    info.encoder_backend = match desc.VendorId {
                        0x10DE => "NVIDIA NVENC".to_owned(),
                        0x1002 => "AMD AMF".to_owned(),
                        0x8086 => "Intel QuickSync".to_owned(),
                        _ => "DXGI Hardware MFT".to_owned(),
                    };
                }
            }
        }

        info.supports_hevc = has_hardware_encoder(MFVideoFormat_HEVC);
        info.supports_av1 = has_hardware_encoder(MF_VIDEO_FORMAT_AV1);
        info.supports_h264 = has_hardware_encoder(MFVideoFormat_H264);

        let _ = MFShutdown();
    }

    info
}

unsafe fn has_hardware_encoder(subtype: GUID) -> bool {
    let out_type = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MF_MEDIATYPE_VIDEO,
        guidSubtype: subtype,
    };
    let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0_u32;
    let res = MFTEnumEx(
        MFT_CATEGORY_VIDEO_ENCODER,
        MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_ASYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
        None,
        Some(&out_type),
        &mut activates,
        &mut count,
    );
    let found = res.is_ok() && count > 0;
    if !activates.is_null() {
        for i in 0..count as usize {
            let slot = activates.add(i);
            std::ptr::drop_in_place(slot);
        }
        CoTaskMemFree(Some(activates.cast()));
    }
    found
}

fn pack_u32_pair(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) | (lo as u64)
}

struct WgcMonitorCapture {
    _item: GraphicsCaptureItem,
    frame_pool: Direct3D11CaptureFramePool,
    session: GraphicsCaptureSession,
    winrt_device: IDirect3DDevice,
    pool_size: SizeInt32,
}

impl WgcMonitorCapture {
    fn open(device: &ID3D11Device) -> Option<Self> {
        unsafe {
            if !GraphicsCaptureSession::IsSupported().unwrap_or(false) {
                return None;
            }
            let hmon = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
            if hmon.is_invalid() {
                return None;
            }
            let dxgi_dev: IDXGIDevice = device.cast().ok()?;
            let inspectable = CreateDirect3D11DeviceFromDXGIDevice(&dxgi_dev).ok()?;
            let winrt_device: IDirect3DDevice = inspectable.cast().ok()?;
            let interop: IGraphicsCaptureItemInterop =
                windows::core::factory::<GraphicsCaptureItem, IGraphicsCaptureItemInterop>().ok()?;
            let item: GraphicsCaptureItem = interop.CreateForMonitor(hmon).ok()?;
            let size = item.Size().ok()?;
            if size.Width <= 0 || size.Height <= 0 {
                return None;
            }
            let frame_pool = Direct3D11CaptureFramePool::CreateFreeThreaded(
                &winrt_device,
                DirectXPixelFormat::B8G8R8A8UIntNormalized,
                2,
                size,
            )
            .ok()?;
            let session = frame_pool.CreateCaptureSession(&item).ok()?;
            let _ = session.SetIsBorderRequired(false);
            let _ = session.SetIsCursorCaptureEnabled(true);
            session.StartCapture().ok()?;
            Some(Self {
                _item: item,
                frame_pool,
                session,
                winrt_device,
                pool_size: size,
            })
        }
    }

    fn capture_latest<F>(&mut self, mut on_tex: F) -> (bool, Option<(u32, u32)>)
    where
        F: FnMut(&ID3D11Texture2D) -> bool,
    {
        let mut latest = None;
        while let Ok(f) = self.frame_pool.TryGetNextFrame() {
            if let Some(prev) = latest.replace(f) {
                let _ = prev.Close();
            }
        }
        let Some(frame) = latest else {
            return (false, None);
        };
        let new_sz = frame.ContentSize().ok();
        let tex = frame
            .Surface()
            .ok()
            .and_then(|s| s.cast::<IDirect3DDxgiInterfaceAccess>().ok())
            .and_then(|acc| unsafe { acc.GetInterface::<ID3D11Texture2D>().ok() });
        let mut dims = None;
        let mut copied = false;
        if let Some(src_tex) = tex {
            let mut desc = D3D11_TEXTURE2D_DESC::default();
            unsafe {
                src_tex.GetDesc(&mut desc);
            }
            dims = Some((desc.Width, desc.Height));
            copied = on_tex(&src_tex);
        }
        let _ = frame.Close();
        if let Some(cs) = new_sz {
            if cs != self.pool_size && cs.Width > 0 && cs.Height > 0 {
                let _ = self.frame_pool.Recreate(
                    &self.winrt_device,
                    DirectXPixelFormat::B8G8R8A8UIntNormalized,
                    2,
                    cs,
                );
                self.pool_size = cs;
            }
        }
        (copied, dims)
    }
}

impl Drop for WgcMonitorCapture {
    fn drop(&mut self) {
        let _ = self.session.Close();
        let _ = self.frame_pool.Close();
    }
}

pub struct GpuCapturePipeline {
    pub active_codec: VideoCodec,
    pub width: u32,
    pub height: u32,
    in_width: u32,
    in_height: u32,
    pub fps: u32,
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    _dxgi_manager: IMFDXGIDeviceManager,
    wgc: Option<WgcMonitorCapture>,
    duplication: Option<IDXGIOutputDuplication>,
    encoder: Option<IMFTransform>,
    encoder_uses_nv12: bool,
    is_async_mft: bool,
    ev_gen: Option<IMFMediaEventGenerator>,
    need_input_count: usize,
    have_output_count: usize,
    vp_device: Option<ID3D11VideoDevice>,
    vp_context: Option<ID3D11VideoContext>,
    vp_enum: Option<ID3D11VideoProcessorEnumerator>,
    vp: Option<ID3D11VideoProcessor>,
    vp_in_view: Option<ID3D11VideoProcessorInputView>,
    nv12_pool: Vec<(ID3D11Texture2D, ID3D11VideoProcessorOutputView)>,
    next_nv12_slot: usize,
    last_nv12_slot: usize,
    has_valid_nv12: bool,
    last_bgra_tex: Option<ID3D11Texture2D>,
    wait_timer: Option<HighResTimer>,
    seq_header: Vec<u8>,
    start_qpc: i64,
    started: bool,
    last_submitted_slot: i64,
    in_flight_abs_pts: VecDeque<i64>,
}

impl GpuCapturePipeline {
    pub fn open(cfg: &AppConfig, hw: &HardwareInfo) -> Option<Self> {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let _ = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);

            let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
            let mut device: Option<ID3D11Device> = None;
            let mut context: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut device),
                None,
                Some(&mut context),
            )
            .ok()?;

            let device = device?;
            let context = context?;
            if let Ok(mt) = device.cast::<ID3D11Multithread>() {
                let _ = mt.SetMultithreadProtected(true);
            }
            if let Ok(dxgi_dev) = device.cast::<IDXGIDevice>() {
                let _ = dxgi_dev.SetGPUThreadPriority(-7);
            }

            let mut reset_token = 0_u32;
            let mut dxgi_manager: Option<IMFDXGIDeviceManager> = None;
            MFCreateDXGIDeviceManager(&mut reset_token, &mut dxgi_manager).ok()?;
            let dxgi_manager = dxgi_manager?;
            dxgi_manager.ResetDevice(&device, reset_token).ok()?;

            let (out_w, out_h) = cfg.resolution.dimensions();
            let wgc = WgcMonitorCapture::open(&device);
            let duplication = if wgc.is_none() {
                create_desktop_duplication(&device).ok()
            } else {
                None
            };
            let (in_w, in_h) = if let Some(w) = &wgc {
                (w.pool_size.Width as u32, w.pool_size.Height as u32)
            } else if let Some(dup) = &duplication {
                let desc = dup.GetDesc();
                let w = desc.ModeDesc.Width;
                let h = desc.ModeDesc.Height;
                if w > 0 && h > 0 {
                    (w, h)
                } else {
                    (out_w, out_h)
                }
            } else {
                (out_w, out_h)
            };

            let fps = cfg.fps.as_u32();
            let primary_codec = choose_supported_codec(cfg.codec, hw);
            let (chosen_codec, encoder, encoder_uses_nv12, is_async_mft, ev_gen) =
                match create_hardware_encoder_mft(
                    primary_codec,
                    out_w,
                    out_h,
                    fps,
                    cfg,
                    &dxgi_manager,
                ) {
                    Some((mft, nv12, is_async, eg)) => {
                        (primary_codec, Some(mft), nv12, is_async, eg)
                    }
                    None if primary_codec != VideoCodec::H264 => {
                        match create_hardware_encoder_mft(
                            VideoCodec::H264,
                            out_w,
                            out_h,
                            fps,
                            cfg,
                            &dxgi_manager,
                        ) {
                            Some((mft, nv12, is_async, eg)) => {
                                (VideoCodec::H264, Some(mft), nv12, is_async, eg)
                            }
                            None => (primary_codec, None, true, false, None),
                        }
                    }
                    None => (primary_codec, None, true, false, None),
                };

            let mut vp_device = None;
            let mut vp_context = None;
            let mut vp_enum = None;
            let mut vp = None;
            let mut nv12_pool = Vec::new();

            if encoder.is_some() && (encoder_uses_nv12 || in_w != out_w || in_h != out_h) {
                if let (Ok(vdev), Ok(vctx)) = (
                    device.cast::<ID3D11VideoDevice>(),
                    context.cast::<ID3D11VideoContext>(),
                ) {
                    if let Ok((en, proc_inst, pool)) = create_nv12_video_processor(
                        &device,
                        &vdev,
                        &vctx,
                        in_w,
                        in_h,
                        out_w,
                        out_h,
                        fps,
                        encoder_uses_nv12,
                    ) {
                        vp_device = Some(vdev);
                        vp_context = Some(vctx);
                        vp_enum = Some(en);
                        vp = Some(proc_inst);
                        nv12_pool = pool;
                    }
                }
            }

            let now_start = qpc_now_100ns();
            let mut seq_header = Vec::new();
            if let Some(mft) = &encoder {
                if let Ok(cur_out_mt) = mft.GetOutputCurrentType(0) {
                    let mut blob_ptr: *mut u8 = std::ptr::null_mut();
                    let mut blob_sz = 0_u32;
                    if cur_out_mt
                        .GetAllocatedBlob(&MF_MT_MPEG_SEQUENCE_HEADER, &mut blob_ptr, &mut blob_sz)
                        .is_ok()
                        && !blob_ptr.is_null()
                        && blob_sz > 0
                    {
                        seq_header = std::slice::from_raw_parts(blob_ptr, blob_sz as usize).to_vec();
                        CoTaskMemFree(Some(blob_ptr.cast()));
                    }
                }
            }

            Some(Self {
                active_codec: chosen_codec,
                width: out_w,
                height: out_h,
                in_width: in_w,
                in_height: in_h,
                fps,
                device,
                context,
                _dxgi_manager: dxgi_manager,
                wgc,
                duplication,
                encoder,
                encoder_uses_nv12,
                is_async_mft,
                ev_gen,
                need_input_count: 0,
                have_output_count: 0,
                vp_device,
                vp_context,
                vp_enum,
                vp,
                vp_in_view: None,
                nv12_pool,
                next_nv12_slot: 0,
                last_nv12_slot: 0,
                has_valid_nv12: false,
                last_bgra_tex: None,
                wait_timer: HighResTimer::new(),
                seq_header,
                start_qpc: now_start,
                started: false,
                last_submitted_slot: -1,
                in_flight_abs_pts: VecDeque::with_capacity(16),
            })
        }
    }

    pub fn capture_and_encode_frame(&mut self) -> Vec<EncodedPacket> {
        let now_qpc = qpc_now_100ns();
        let fps = self.fps.max(1) as i128;
        let nominal_dur = (10_000_000_i64) / (fps as i64);

        unsafe {
            let mut got_new_frame = false;
            let mut observed_dims: Option<(u32, u32)> = None;

            if self.wgc.is_some() {
                let device = self.device.clone();
                let context = self.context.clone();
                let mut bgra_slot = self.last_bgra_tex.take();
                let mut vp_iv = self.vp_in_view.take();
                let mut invalid_nv12 = false;

                if let Some(wgc) = &mut self.wgc {
                    let (copied, dims) = wgc.capture_latest(|src| {
                        let mut desc = D3D11_TEXTURE2D_DESC::default();
                        src.GetDesc(&mut desc);
                        let need_new = match &bgra_slot {
                            Some(cur) => {
                                let mut cd = D3D11_TEXTURE2D_DESC::default();
                                cur.GetDesc(&mut cd);
                                cd.Width != desc.Width
                                    || cd.Height != desc.Height
                                    || cd.Format != desc.Format
                            }
                            None => true,
                        };
                        if need_new {
                            desc.Usage = D3D11_USAGE_DEFAULT;
                            desc.BindFlags = D3D11_BIND_RENDER_TARGET.0 as u32;
                            desc.CPUAccessFlags = 0;
                            desc.MiscFlags = 0;
                            let mut copy_tex: Option<ID3D11Texture2D> = None;
                            if device.CreateTexture2D(&desc, None, Some(&mut copy_tex)).is_ok() {
                                bgra_slot = copy_tex;
                                vp_iv = None;
                                invalid_nv12 = true;
                            }
                        }
                        if let Some(dst) = &bgra_slot {
                            context.CopyResource(dst, src);
                            true
                        } else {
                            false
                        }
                    });
                    got_new_frame = copied;
                    observed_dims = dims;
                }

                self.last_bgra_tex = bgra_slot;
                self.vp_in_view = vp_iv;
                if invalid_nv12 {
                    self.has_valid_nv12 = false;
                }
            } else {
                if self.duplication.is_none() {
                    self.duplication = create_desktop_duplication(&self.device).ok();
                }

                let wait_ms = if self.last_bgra_tex.is_none() { 25 } else { 0 };
                if let Some(dup) = self.duplication.clone() {
                    let mut frame_info = DXGI_OUTDUPL_FRAME_INFO::default();
                    let mut desktop_res = None;
                    match dup.AcquireNextFrame(wait_ms, &mut frame_info, &mut desktop_res) {
                        Ok(()) => {
                            let updated = frame_info.LastPresentTime != 0
                                || frame_info.AccumulatedFrames > 0
                                || self.last_bgra_tex.is_none();
                            if updated {
                                if let Some(res) = desktop_res {
                                    if let Ok(tex) = res.cast::<ID3D11Texture2D>() {
                                        let mut desc = D3D11_TEXTURE2D_DESC::default();
                                        tex.GetDesc(&mut desc);
                                        observed_dims = Some((desc.Width, desc.Height));
                                        self.ensure_bgra_copy_texture(&tex);
                                        if let Some(dst) = &self.last_bgra_tex {
                                            self.context.CopyResource(dst, &tex);
                                            got_new_frame = true;
                                        }
                                    }
                                }
                            }
                            let _ = dup.ReleaseFrame();
                        }
                        Err(e) if e.code() == DXGI_ERROR_WAIT_TIMEOUT => {}
                        Err(_) => {
                            self.duplication = None;
                        }
                    }
                }
            }

            if let Some((nw, nh)) = observed_dims {
                if nw > 0 && nh > 0 && (nw != self.in_width || nh != self.in_height) {
                    self.rebuild_video_processor_for_input(nw, nh);
                }
            }

            if self.last_bgra_tex.is_none() {
                self.create_fallback_vram_bgra_texture();
            }

            let mut out_pkts = Vec::new();
            let poll_async_events = |eg_opt: &Option<IMFMediaEventGenerator>,
                                     need_in: &mut usize,
                                     have_out: &mut usize| {
                if let Some(eg) = eg_opt {
                    while let Ok(ev) = eg.GetEvent(MF_EVENT_FLAG_NO_WAIT) {
                        if let Ok(t) = ev.GetType() {
                            if t == METransformNeedInput.0 as u32 {
                                *need_in += 1;
                            } else if t == METransformHaveOutput.0 as u32 {
                                *have_out += 1;
                            }
                        }
                    }
                }
            };

            if self.is_async_mft {
                poll_async_events(&self.ev_gen, &mut self.need_input_count, &mut self.have_output_count);
            }

            if self.encoder.is_none() {
                return Vec::new();
            };

            let prepared_tex: Option<(ID3D11Texture2D, Option<usize>)> = if !self.nv12_pool.is_empty() {
                let pool_len = self.nv12_pool.len();
                let mut nv_slot = self.next_nv12_slot % pool_len;
                if !got_new_frame && self.has_valid_nv12 && nv_slot == self.last_nv12_slot && pool_len > 1
                {
                    nv_slot = (nv_slot + 1) % pool_len;
                }
                if got_new_frame || !self.has_valid_nv12 {
                    if self.vp_in_view.is_none() {
                        if let (Some(src), Some(vdev), Some(venum)) =
                            (&self.last_bgra_tex, &self.vp_device, &self.vp_enum)
                        {
                            let in_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
                                FourCC: 0,
                                ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
                                Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                                    Texture2D:
                                        windows::Win32::Graphics::Direct3D11::D3D11_TEX2D_VPIV {
                                            MipSlice: 0,
                                            ArraySlice: 0,
                                        },
                                },
                            };
                            let mut iv = None;
                            if vdev
                                .CreateVideoProcessorInputView(src, venum, &in_desc, Some(&mut iv))
                                .is_ok()
                            {
                                self.vp_in_view = iv;
                            }
                        }
                    }
                    let (ref dst_tex, ref out_v) = self.nv12_pool[nv_slot];
                    if let (Some(vctx), Some(vproc), Some(in_v)) =
                        (&self.vp_context, &self.vp, &self.vp_in_view)
                    {
                        if blt_bgra_to_nv12(vctx, vproc, in_v, out_v).is_ok() {
                            self.context.Flush();
                            self.last_nv12_slot = nv_slot;
                            self.has_valid_nv12 = true;
                            Some((dst_tex.clone(), Some(nv_slot)))
                        } else {
                            None
                        }
                    } else {
                        None
                    }
                } else if nv_slot != self.last_nv12_slot {
                    let src_tex = self.nv12_pool[self.last_nv12_slot].0.clone();
                    let dst_tex = self.nv12_pool[nv_slot].0.clone();
                    self.context.CopyResource(&dst_tex, &src_tex);
                    Some((dst_tex, Some(nv_slot)))
                } else {
                    Some((self.nv12_pool[nv_slot].0.clone(), Some(nv_slot)))
                }
            } else {
                self.last_bgra_tex.clone().map(|t| (t, None))
            };

            if self.is_async_mft && self.have_output_count > 0 {
                if let Some(mft) = &self.encoder {
                    out_pkts.extend(drain_encoder_outputs(
                        mft,
                        self.active_codec,
                        self.start_qpc,
                        now_qpc,
                        nominal_dur,
                        &mut self.seq_header,
                        &mut self.in_flight_abs_pts,
                        Some(&mut self.have_output_count),
                    ));
                }
                poll_async_events(&self.ev_gen, &mut self.need_input_count, &mut self.have_output_count);
            }

            if self.is_async_mft && self.need_input_count == 0 {
                let wait_budget = (nominal_dur / 3).clamp(10_000, 30_000);
                let wait_deadline = qpc_now_100ns() + wait_budget;
                while qpc_now_100ns() < wait_deadline {
                    if self.have_output_count > 0 {
                        if let Some(mft) = &self.encoder {
                            out_pkts.extend(drain_encoder_outputs(
                                mft,
                                self.active_codec,
                                self.start_qpc,
                                qpc_now_100ns(),
                                nominal_dur,
                                &mut self.seq_header,
                                &mut self.in_flight_abs_pts,
                                Some(&mut self.have_output_count),
                            ));
                        }
                    }
                    poll_async_events(&self.ev_gen, &mut self.need_input_count, &mut self.have_output_count);
                    if self.need_input_count > 0 {
                        break;
                    }
                    if let Some(t) = &self.wait_timer {
                        t.wait_100ns(2_000);
                    } else {
                        std::thread::yield_now();
                    }
                }
            }

            let mut feed_qpc = qpc_now_100ns();
            if self.started && (!self.is_async_mft || self.need_input_count > 0) {
                let next_slot = self.last_submitted_slot + 1;
                let next_slot_rel = ((next_slot as i128 * 10_000_000) / fps) as i64;
                let elapsed_100ns = feed_qpc.saturating_sub(self.start_qpc).max(0);
                let early_100ns = next_slot_rel - elapsed_100ns;
                if early_100ns > 2_000 && early_100ns <= nominal_dur {
                    if let Some(t) = &self.wait_timer {
                        t.wait_100ns(early_100ns);
                    } else {
                        std::thread::sleep(std::time::Duration::from_micros((early_100ns / 10) as u64));
                    }
                    feed_qpc = qpc_now_100ns();
                }
            }
            let chosen_slot: Option<(i64, i64, i64)> = if !self.is_async_mft || self.need_input_count > 0 {
                if !self.started {
                    Some((0, 0, now_qpc))
                } else {
                    let elapsed_100ns = feed_qpc.saturating_sub(self.start_qpc).max(0);
                    let next_slot = self.last_submitted_slot + 1;
                    let next_slot_rel = ((next_slot as i128 * 10_000_000) / fps) as i64;
                    let behind_100ns = elapsed_100ns - next_slot_rel;
                    let max_jitter = (nominal_dur * 3).max(400_000);
                    let chosen = if behind_100ns >= max_jitter {
                        let wall_slot = ((elapsed_100ns as i128 * fps) / 10_000_000) as i64;
                        wall_slot.max(next_slot)
                    } else {
                        next_slot
                    };
                    let rel = ((chosen as i128 * 10_000_000) / fps) as i64;
                    Some((chosen, rel, self.start_qpc + rel))
                }
            } else {
                None
            };

            if let (Some((slot, cfr_rel_pts, abs_pts)), Some((gpu_tex, used_nv_slot))) =
                (chosen_slot, prepared_tex)
            {
                if let Some(nv_slot) = used_nv_slot {
                    self.next_nv12_slot = (nv_slot + 1) % self.nv12_pool.len().max(1);
                }
                if let Ok(buf) = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, &gpu_tex, 0, false) {
                    if let Ok(len) = buf.GetMaxLength() {
                        let _ = buf.SetCurrentLength(len);
                    }
                    if let Ok(sample) = MFCreateSample() {
                        let _ = sample.AddBuffer(&buf);
                        let _ = sample.SetSampleTime(cfr_rel_pts);
                        let _ = sample.SetSampleDuration(nominal_dur);
                        if let Some(mft) = &self.encoder {
                            if mft.ProcessInput(0, &sample, 0).is_ok() {
                                if !self.started {
                                    self.start_qpc = now_qpc;
                                    self.started = true;
                                }
                                if self.is_async_mft {
                                    self.need_input_count = self.need_input_count.saturating_sub(1);
                                }
                                self.last_submitted_slot = slot;
                                if self.in_flight_abs_pts.len() >= 32 {
                                    self.in_flight_abs_pts.pop_front();
                                }
                                self.in_flight_abs_pts.push_back(abs_pts);
                            }
                        }
                    }
                }
            }

            if self.is_async_mft {
                poll_async_events(&self.ev_gen, &mut self.need_input_count, &mut self.have_output_count);
            }

            if !self.is_async_mft || self.have_output_count > 0 {
                if let Some(mft) = &self.encoder {
                    out_pkts.extend(drain_encoder_outputs(
                        mft,
                        self.active_codec,
                        self.start_qpc,
                        qpc_now_100ns(),
                        nominal_dur,
                        &mut self.seq_header,
                        &mut self.in_flight_abs_pts,
                        if self.is_async_mft {
                            Some(&mut self.have_output_count)
                        } else {
                            None
                        },
                    ));
                }
            }
            out_pkts
        }
    }

    unsafe fn ensure_bgra_copy_texture(&mut self, src: &ID3D11Texture2D) {
        let mut desc = D3D11_TEXTURE2D_DESC::default();
        src.GetDesc(&mut desc);
        if let Some(cur) = &self.last_bgra_tex {
            let mut cur_desc = D3D11_TEXTURE2D_DESC::default();
            cur.GetDesc(&mut cur_desc);
            if cur_desc.Width == desc.Width
                && cur_desc.Height == desc.Height
                && cur_desc.Format == desc.Format
            {
                return;
            }
        }
        desc.Usage = D3D11_USAGE_DEFAULT;
        desc.BindFlags = D3D11_BIND_RENDER_TARGET.0 as u32;
        desc.CPUAccessFlags = 0;
        desc.MiscFlags = 0;
        let mut copy_tex: Option<ID3D11Texture2D> = None;
        if self.device.CreateTexture2D(&desc, None, Some(&mut copy_tex)).is_ok() {
            self.last_bgra_tex = copy_tex;
            self.vp_in_view = None;
            self.has_valid_nv12 = false;
        }
    }

    unsafe fn rebuild_video_processor_for_input(&mut self, in_w: u32, in_h: u32) {
        self.in_width = in_w;
        self.in_height = in_h;
        self.vp_in_view = None;
        self.has_valid_nv12 = false;
        if self.encoder.is_none() {
            return;
        }
        if let (Some(vdev), Some(vctx)) = (&self.vp_device, &self.vp_context) {
            if let Ok((en, proc_inst, pool)) = create_nv12_video_processor(
                &self.device,
                vdev,
                vctx,
                in_w,
                in_h,
                self.width,
                self.height,
                self.fps,
                self.encoder_uses_nv12,
            ) {
                self.vp_enum = Some(en);
                self.vp = Some(proc_inst);
                self.nv12_pool = pool;
                self.next_nv12_slot = 0;
                self.last_nv12_slot = 0;
            }
        }
    }

    unsafe fn create_fallback_vram_bgra_texture(&mut self) {
        if self.last_bgra_tex.is_some() {
            return;
        }
        let desc = D3D11_TEXTURE2D_DESC {
            Width: self.in_width.max(64),
            Height: self.in_height.max(64),
            MipLevels: 1,
            ArraySize: 1,
            Format: DXGI_FORMAT_B8G8R8A8_UNORM,
            SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
            Usage: D3D11_USAGE_DEFAULT,
            BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
            CPUAccessFlags: 0,
            MiscFlags: 0,
        };
        let mut tex: Option<ID3D11Texture2D> = None;
        if self.device.CreateTexture2D(&desc, None, Some(&mut tex)).is_ok() {
            self.last_bgra_tex = tex;
            self.vp_in_view = None;
            self.has_valid_nv12 = false;
        }
    }
}

impl Drop for GpuCapturePipeline {
    fn drop(&mut self) {
        unsafe {
            self.wgc = None;
            self.encoder = None;
            self.duplication = None;
            self.vp_in_view = None;
            self.nv12_pool.clear();
            self.last_bgra_tex = None;
            let _ = MFShutdown();
        }
    }
}

fn choose_supported_codec(requested: VideoCodec, hw: &HardwareInfo) -> VideoCodec {
    match requested {
        VideoCodec::Av1 if hw.supports_av1 => VideoCodec::Av1,
        VideoCodec::Av1 if hw.supports_hevc => VideoCodec::Hevc,
        VideoCodec::Hevc if hw.supports_hevc => VideoCodec::Hevc,
        VideoCodec::H264 => VideoCodec::H264,
        _ if hw.supports_h264 => VideoCodec::H264,
        _ => VideoCodec::H264,
    }
}

unsafe fn create_desktop_duplication(device: &ID3D11Device) -> windows::core::Result<IDXGIOutputDuplication> {
    let dxgi_dev: IDXGIDevice = device.cast()?;
    let adapter = dxgi_dev.GetAdapter()?;
    let output = adapter.EnumOutputs(0)?;
    let output1: IDXGIOutput1 = output.cast()?;
    output1.DuplicateOutput(device)
}

unsafe fn create_nv12_video_processor(
    device: &ID3D11Device,
    vdev: &ID3D11VideoDevice,
    vctx: &ID3D11VideoContext,
    in_w: u32,
    in_h: u32,
    out_w: u32,
    out_h: u32,
    fps: u32,
    use_nv12: bool,
) -> windows::core::Result<(
    ID3D11VideoProcessorEnumerator,
    ID3D11VideoProcessor,
    Vec<(ID3D11Texture2D, ID3D11VideoProcessorOutputView)>,
)> {
    let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
        InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
        InputFrameRate: DXGI_RATIONAL { Numerator: fps, Denominator: 1 },
        InputWidth: in_w,
        InputHeight: in_h,
        OutputFrameRate: DXGI_RATIONAL { Numerator: fps, Denominator: 1 },
        OutputWidth: out_w,
        OutputHeight: out_h,
        Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
    };
    let enumerator = vdev.CreateVideoProcessorEnumerator(&content)?;
    let processor = vdev.CreateVideoProcessor(&enumerator, 0)?;

    let src_rect = RECT {
        left: 0,
        top: 0,
        right: in_w as i32,
        bottom: in_h as i32,
    };
    let dst_rect = RECT {
        left: 0,
        top: 0,
        right: out_w as i32,
        bottom: out_h as i32,
    };
    vctx.VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&src_rect));
    vctx.VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&dst_rect));
    vctx.VideoProcessorSetOutputTargetRect(&processor, true, Some(&dst_rect));

    let in_cs = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
        _bitfield: (1 << 2) | (2 << 4),
    };
    vctx.VideoProcessorSetStreamColorSpace(&processor, 0, &in_cs);

    let out_cs = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
        _bitfield: (1 << 2) | (1 << 4),
    };
    vctx.VideoProcessorSetOutputColorSpace(&processor, &out_cs);

    let tex_desc = D3D11_TEXTURE2D_DESC {
        Width: out_w,
        Height: out_h,
        MipLevels: 1,
        ArraySize: 1,
        Format: if use_nv12 {
            DXGI_FORMAT_NV12
        } else {
            DXGI_FORMAT_B8G8R8A8_UNORM
        },
        SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
        Usage: D3D11_USAGE_DEFAULT,
        BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
        CPUAccessFlags: 0,
        MiscFlags: 0,
    };
    let out_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
        ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
        Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
            Texture2D: windows::Win32::Graphics::Direct3D11::D3D11_TEX2D_VPOV { MipSlice: 0 },
        },
    };
    let mut pool = Vec::with_capacity(6);
    for _ in 0..6 {
        let mut nv12_tex = None;
        device.CreateTexture2D(&tex_desc, None, Some(&mut nv12_tex))?;
        let tex = nv12_tex.unwrap();
        let mut out_view: Option<ID3D11VideoProcessorOutputView> = None;
        vdev.CreateVideoProcessorOutputView(&tex, &enumerator, &out_desc, Some(&mut out_view))?;
        pool.push((tex, out_view.unwrap()));
    }
    Ok((enumerator, processor, pool))
}

unsafe fn blt_bgra_to_nv12(
    vctx: &ID3D11VideoContext,
    vp: &ID3D11VideoProcessor,
    in_view: &ID3D11VideoProcessorInputView,
    out_view: &ID3D11VideoProcessorOutputView,
) -> windows::core::Result<()> {
    let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
        Enable: true.into(),
        OutputIndex: 0,
        InputFrameOrField: 0,
        PastFrames: 0,
        FutureFrames: 0,
        ppPastSurfaces: std::ptr::null_mut(),
        pInputSurface: ManuallyDrop::new(Some(in_view.clone())),
        ppFutureSurfaces: std::ptr::null_mut(),
        ppPastSurfacesRight: std::ptr::null_mut(),
        pInputSurfaceRight: ManuallyDrop::new(None),
        ppFutureSurfacesRight: std::ptr::null_mut(),
    };
    let res = vctx.VideoProcessorBlt(vp, out_view, 0, std::slice::from_ref(&stream));
    ManuallyDrop::drop(&mut stream.pInputSurface);
    res
}

unsafe fn create_hardware_encoder_mft(
    codec: VideoCodec,
    w: u32,
    h: u32,
    fps: u32,
    cfg: &AppConfig,
    dxgi_mgr: &IMFDXGIDeviceManager,
) -> Option<(IMFTransform, bool, bool, Option<IMFMediaEventGenerator>)> {
    let subtype = match codec {
        VideoCodec::Hevc => MFVideoFormat_HEVC,
        VideoCodec::Av1 => MF_VIDEO_FORMAT_AV1,
        VideoCodec::H264 => MFVideoFormat_H264,
    };

    let out_info = MFT_REGISTER_TYPE_INFO {
        guidMajorType: MF_MEDIATYPE_VIDEO,
        guidSubtype: subtype,
    };

    let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
    let mut count = 0_u32;
    MFTEnumEx(
        MFT_CATEGORY_VIDEO_ENCODER,
        MFT_ENUM_FLAG_HARDWARE | MFT_ENUM_FLAG_ASYNCMFT | MFT_ENUM_FLAG_SORTANDFILTER,
        None,
        Some(&out_info),
        &mut activates,
        &mut count,
    )
    .ok()?;

    if count == 0 || activates.is_null() {
        return None;
    }

    let mut chosen_mft: Option<IMFTransform> = None;
    for i in 0..count as usize {
        if chosen_mft.is_none() {
            if let Some(act) = &*activates.add(i) {
                let want_filter = match cfg.encoder_preference {
                    EncoderPreference::Auto => true,
                    EncoderPreference::Nvenc => true,
                    EncoderPreference::Amf => true,
                    EncoderPreference::Qsv => true,
                };
                if want_filter {
                    if let Ok(mft) = act.ActivateObject::<IMFTransform>() {
                        chosen_mft = Some(mft);
                    }
                }
            }
        }
        std::ptr::drop_in_place(activates.add(i));
    }
    CoTaskMemFree(Some(activates.cast()));

    let mft = chosen_mft?;
    let mut is_async = false;
    if let Ok(attrs) = mft.GetAttributes() {
        if attrs.GetUINT32(&MF_TRANSFORM_ASYNC).unwrap_or(0) != 0 {
            let _ = attrs.SetUINT32(&MF_TRANSFORM_ASYNC_UNLOCK, 1);
            is_async = true;
        }
    }

    let mgr_raw = dxgi_mgr.as_raw() as usize;
    let _ = mft.ProcessMessage(MFT_MESSAGE_SET_D3D_MANAGER, mgr_raw);

    apply_codec_api_settings(&mft, fps, cfg);

    let out_mt = MFCreateMediaType().ok()?;
    out_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_VIDEO).ok()?;
    out_mt.SetGUID(&MF_MT_SUBTYPE, &subtype).ok()?;
    out_mt.SetUINT64(&MF_MT_FRAME_SIZE, pack_u32_pair(w, h)).ok()?;
    out_mt.SetUINT64(&MF_MT_FRAME_RATE, pack_u32_pair(fps, 1)).ok()?;
    out_mt.SetUINT32(&MF_MT_INTERLACE_MODE, 2).ok()?;
    let target_mbps = if cfg.rate_control == RateControl::Cqp {
        cfg.bitrate_mbps.max(80)
    } else {
        cfg.bitrate_mbps.clamp(5, 150)
    };
    out_mt
        .SetUINT32(&MF_MT_AVG_BITRATE, target_mbps.saturating_mul(1_000_000))
        .ok()?;
    mft.SetOutputType(0, &out_mt, 0).ok()?;

    let mut use_nv12 = true;
    let mut set_in_ok = false;
    for candidate in [MFVideoFormat_NV12, MFVideoFormat_ARGB32] {
        let in_mt = MFCreateMediaType().ok()?;
        in_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_VIDEO).ok()?;
        in_mt.SetGUID(&MF_MT_SUBTYPE, &candidate).ok()?;
        in_mt.SetUINT64(&MF_MT_FRAME_SIZE, pack_u32_pair(w, h)).ok()?;
        in_mt.SetUINT64(&MF_MT_FRAME_RATE, pack_u32_pair(fps, 1)).ok()?;
        in_mt.SetUINT32(&MF_MT_INTERLACE_MODE, 2).ok()?;
        if mft.SetInputType(0, &in_mt, 0).is_ok() {
            use_nv12 = candidate == MFVideoFormat_NV12;
            set_in_ok = true;
            break;
        }
    }
    if !set_in_ok {
        return None;
    }

    apply_codec_api_settings(&mft, fps, cfg);

    let _ = mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0);
    let _ = mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0);

    let ev_gen = if is_async {
        mft.cast::<IMFMediaEventGenerator>().ok()
    } else {
        None
    };

    Some((mft, use_nv12, is_async, ev_gen))
}

unsafe fn apply_codec_api_settings(mft: &IMFTransform, fps: u32, cfg: &AppConfig) {
    if let Ok(codec_api) = mft.cast::<ICodecAPI>() {
        let rc_mode = match cfg.rate_control {
            RateControl::Cqp => 3_u32,
            RateControl::Cbr => 0_u32,
        };
        let _ = codec_api.SetValue(&CODECAPI_AVEncCommonRateControlMode, &VARIANT::from(rc_mode));
        if cfg.rate_control == RateControl::Cqp {
            let qp_u32 = cfg.cqp_qp.clamp(12, 32);
            let quality = (100_u32).saturating_sub(qp_u32 * 2).clamp(35, 95);
            let _ = codec_api.SetValue(&CODECAPI_AVEncCommonQuality, &VARIANT::from(quality));
            let qp = qp_u32 as u64;
            let packed_qp = qp | (qp << 16) | (qp << 32);
            if codec_api
                .SetValue(&CODECAPI_AVEncVideoEncodeQP, &VARIANT::from(packed_qp))
                .is_err()
            {
                let _ = codec_api.SetValue(&CODECAPI_AVEncVideoEncodeQP, &VARIANT::from(qp_u32));
            }
            let cap_bps = cfg.bitrate_mbps.max(80).saturating_mul(1_000_000);
            let _ = codec_api.SetValue(&CODECAPI_AVEncCommonMaxBitRate, &VARIANT::from(cap_bps));
        } else {
            let bps = cfg.bitrate_mbps.clamp(5, 150).saturating_mul(1_000_000);
            let _ = codec_api.SetValue(&CODECAPI_AVEncCommonMeanBitRate, &VARIANT::from(bps));
            let _ = codec_api.SetValue(&CODECAPI_AVEncCommonMaxBitRate, &VARIANT::from(bps));
            let _ = codec_api.SetValue(&CODECAPI_AVEncCommonBufferSize, &VARIANT::from(bps / 2));
        }
        let gop_frames = fps.max(15);
        let _ = codec_api.SetValue(&CODECAPI_AVEncMPVGOPSize, &VARIANT::from(gop_frames));
        let low_lat = cfg.rate_control == RateControl::Cqp && cfg.codec != VideoCodec::H264;
        let _ = codec_api.SetValue(&CODECAPI_AVLowLatencyMode, &VARIANT::from(low_lat));
        let _ = codec_api.SetValue(&CODECAPI_AVEncMPVDefaultBPictureCount, &VARIANT::from(0_u32));
        let _ = codec_api.SetValue(&CODECAPI_AVEncCommonQualityVsSpeed, &VARIANT::from(0_u32));
    }
}

unsafe fn drain_encoder_outputs(
    mft: &IMFTransform,
    codec: VideoCodec,
    start_qpc: i64,
    fallback_abs_qpc: i64,
    fallback_dur: i64,
    seq_header: &mut Vec<u8>,
    in_flight_abs_pts: &mut VecDeque<i64>,
    mut async_have_output: Option<&mut usize>,
) -> Vec<EncodedPacket> {
    let mut out = Vec::new();
    let Ok(stream_info) = mft.GetOutputStreamInfo(0) else {
        return out;
    };
    let mft_allocates = (stream_info.dwFlags & 0x0000_0100) != 0;

    loop {
        if let Some(cnt) = async_have_output.as_ref() {
            if **cnt == 0 {
                break;
            }
        }

        let sample_opt = if mft_allocates {
            None
        } else {
            let Ok(s) = MFCreateSample() else { break };
            let buf_sz = stream_info.cbSize.max(512 * 1024);
            let Ok(b) = MFCreateMemoryBuffer(buf_sz) else { break };
            let _ = s.AddBuffer(&b);
            Some(s)
        };

        let mut out_buf = MFT_OUTPUT_DATA_BUFFER {
            dwStreamID: 0,
            pSample: ManuallyDrop::new(sample_opt),
            dwStatus: 0,
            pEvents: ManuallyDrop::new(None),
        };
        let mut status = 0_u32;
        let res = mft.ProcessOutput(0, std::slice::from_mut(&mut out_buf), &mut status);

        if let Some(cnt) = async_have_output.as_deref_mut() {
            *cnt = cnt.saturating_sub(1);
        }

        if let Some(sample) = ManuallyDrop::into_inner(out_buf.pSample) {
            if res.is_ok() {
                let queued_pts = in_flight_abs_pts.pop_front().unwrap_or(fallback_abs_qpc);
                if let Some(pkt) = extract_packet_from_sample(
                    &sample,
                    codec,
                    start_qpc,
                    queued_pts,
                    fallback_dur,
                    seq_header,
                ) {
                    out.push(pkt);
                }
            }
        }
        let _ = ManuallyDrop::into_inner(out_buf.pEvents);

        match res {
            Ok(()) => continue,
            Err(e)
                if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT
                    || e.code() == MF_E_TRANSFORM_STREAM_CHANGE =>
            {
                break;
            }
            Err(_) => break,
        }
    }
    out
}

unsafe fn extract_packet_from_sample(
    sample: &IMFSample,
    codec: VideoCodec,
    start_qpc: i64,
    fallback_abs_qpc: i64,
    fallback_dur: i64,
    seq_header: &mut Vec<u8>,
) -> Option<EncodedPacket> {
    let buf: IMFMediaBuffer = sample.ConvertToContiguousBuffer().ok()?;
    let mut ptr: *mut u8 = std::ptr::null_mut();
    let mut cur_len = 0_u32;
    buf.Lock(&mut ptr, None, Some(&mut cur_len)).ok()?;
    let mut data = if !ptr.is_null() && cur_len > 0 {
        std::slice::from_raw_parts(ptr, cur_len as usize).to_vec()
    } else {
        Vec::new()
    };
    let _ = buf.Unlock();

    if data.is_empty() {
        return None;
    }

    let (has_param_sets, has_idr_nal) = inspect_annex_b_nals(&data, codec);
    if seq_header.is_empty() && has_param_sets {
        *seq_header = extract_param_set_prefix(&data, codec);
    }

    let clean = (sample.GetUINT32(&MFSampleExtension_CleanPoint).unwrap_or(0) != 0) || has_idr_nal;
    if clean && !has_param_sets && !seq_header.is_empty() {
        let mut combined = Vec::with_capacity(seq_header.len() + data.len());
        combined.extend_from_slice(seq_header);
        combined.extend_from_slice(&data);
        data = combined;
    }

    let abs_pts = match sample.GetSampleTime() {
        Ok(rel) if rel >= 0 => start_qpc + rel,
        _ => fallback_abs_qpc,
    };
    let dur = sample.GetSampleDuration().unwrap_or(fallback_dur).max(10_000);

    Some(EncodedPacket {
        track: PacketTrack::Video,
        pts_100ns: abs_pts,
        dts_100ns: abs_pts,
        duration_100ns: dur,
        is_idr_keyframe: clean,
        data,
    })
}

fn inspect_annex_b_nals(data: &[u8], codec: VideoCodec) -> (bool, bool) {
    let mut has_param = false;
    let mut has_idr = false;
    let mut i = 0;
    while i + 3 < data.len() {
        let sc = if i + 3 < data.len()
            && data[i] == 0
            && data[i + 1] == 0
            && data[i + 2] == 0
            && data[i + 3] == 1
        {
            4
        } else if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            3
        } else {
            i += 1;
            continue;
        };
        let hdr_idx = i + sc;
        if hdr_idx < data.len() {
            let b = data[hdr_idx];
            match codec {
                VideoCodec::H264 => {
                    let nt = b & 0x1F;
                    if nt == 7 || nt == 8 {
                        has_param = true;
                    } else if nt == 5 {
                        has_idr = true;
                    }
                }
                VideoCodec::Hevc => {
                    let nt = (b >> 1) & 0x3F;
                    if nt == 32 || nt == 33 || nt == 34 {
                        has_param = true;
                    } else if (19..=21).contains(&nt) {
                        has_idr = true;
                    }
                }
                VideoCodec::Av1 => {}
            }
        }
        i += sc;
    }
    (has_param, has_idr)
}

fn extract_param_set_prefix(data: &[u8], codec: VideoCodec) -> Vec<u8> {
    let mut out = Vec::new();
    let mut starts = Vec::new();
    let mut i = 0;
    while i + 2 < data.len() {
        if i + 3 < data.len() && data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 0 && data[i + 3] == 1 {
            starts.push((i, 4));
            i += 4;
        } else if data[i] == 0 && data[i + 1] == 0 && data[i + 2] == 1 {
            starts.push((i, 3));
            i += 3;
        } else {
            i += 1;
        }
    }
    for idx in 0..starts.len() {
        let (pos, sc) = starts[idx];
        let end = if idx + 1 < starts.len() { starts[idx + 1].0 } else { data.len() };
        if pos + sc < end {
            let b = data[pos + sc];
            let is_param = match codec {
                VideoCodec::H264 => matches!(b & 0x1F, 7 | 8),
                VideoCodec::Hevc => matches!((b >> 1) & 0x3F, 32..=34),
                VideoCodec::Av1 => false,
            };
            if is_param {
                out.extend_from_slice(&data[pos..end]);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode_synthetic_bytes(cfg: &AppConfig, frames: u32) -> usize {
        use windows::Win32::Graphics::Direct3D::D3D_DRIVER_TYPE_HARDWARE;
        use windows::Win32::Graphics::Direct3D11::D3D11_SUBRESOURCE_DATA;
        use windows::Win32::Media::MediaFoundation::MFT_MESSAGE_COMMAND_DRAIN;
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let _ = MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET);
            let mut d3d_opt = None;
            let mut ctx_opt = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                HMODULE::default(),
                D3D11_CREATE_DEVICE_BGRA_SUPPORT | D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
                None,
                D3D11_SDK_VERSION,
                Some(&mut d3d_opt),
                None,
                Some(&mut ctx_opt),
            )
            .expect("D3D11 device");
            let d3d = d3d_opt.unwrap();
            let d3d_ctx = ctx_opt.unwrap();
            let mut reset_token = 0_u32;
            let mut mgr_opt = None;
            MFCreateDXGIDeviceManager(&mut reset_token, &mut mgr_opt).expect("dxgi mgr");
            let dxgi_mgr = mgr_opt.unwrap();
            let _ = dxgi_mgr.ResetDevice(&d3d, reset_token);

            let w = 1280_u32;
            let h = 720_u32;
            let fps = 60_u32;
            let (mft, use_nv12, is_async, ev_gen) =
                create_hardware_encoder_mft(cfg.codec, w, h, fps, cfg, &dxgi_mgr)
                    .expect("hardware MFT");
            let vdev: ID3D11VideoDevice = d3d.cast().expect("video device");
            let vctx: ID3D11VideoContext = d3d_ctx.cast().expect("video ctx");
            let (vp_enum, vp, nv12_pool) =
                create_nv12_video_processor(&d3d, &vdev, &vctx, w, h, w, h, fps, use_nv12)
                    .expect("vp");

            let tex_desc = D3D11_TEXTURE2D_DESC {
                Width: w,
                Height: h,
                MipLevels: 1,
                ArraySize: 1,
                Format: DXGI_FORMAT_B8G8R8A8_UNORM,
                SampleDesc: DXGI_SAMPLE_DESC { Count: 1, Quality: 0 },
                Usage: D3D11_USAGE_DEFAULT,
                BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                CPUAccessFlags: 0,
                MiscFlags: 0,
            };
            let mut bgra_buf = vec![0_u8; (w * h * 4) as usize];
            let mut total_bytes = 0_usize;
            let mut seq_header = Vec::new();
            let mut in_flight = VecDeque::new();
            let mut async_need = if is_async { 1_usize } else { 0 };
            let mut async_out = 0_usize;
            let start_qpc = qpc_now_100ns();
            let frame_dur = 10_000_000_i64 / fps as i64;

            for idx in 0..frames {
                for y in 0..h {
                    for x in 0..w {
                        let off = ((y * w + x) * 4) as usize;
                        let gx = (x + idx * 7) / 8;
                        let gy = (y + idx * 3) / 8;
                        let v = ((gx.wrapping_mul(17) ^ gy.wrapping_mul(31)) & 0xFF) as u8;
                        bgra_buf[off] = v;
                        bgra_buf[off + 1] = v.wrapping_add((x / 6) as u8);
                        bgra_buf[off + 2] = v.wrapping_add((y / 6) as u8);
                        bgra_buf[off + 3] = 255;
                    }
                }
                let init = D3D11_SUBRESOURCE_DATA {
                    pSysMem: bgra_buf.as_ptr() as *const _,
                    SysMemPitch: w * 4,
                    SysMemSlicePitch: 0,
                };
                let mut src_tex: Option<ID3D11Texture2D> = None;
                d3d.CreateTexture2D(&tex_desc, Some(&init), Some(&mut src_tex))
                    .expect("src tex");
                let src_tex = src_tex.unwrap();

                let slot = (idx as usize) % nv12_pool.len();
                let (nv12_tex, out_view) = &nv12_pool[slot];
                let enc_tex = if use_nv12 {
                    let mut in_view = None;
                    let mut in_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC::default();
                    in_desc.ViewDimension = D3D11_VPIV_DIMENSION_TEXTURE2D;
                    vdev.CreateVideoProcessorInputView(&src_tex, &vp_enum, &in_desc, Some(&mut in_view))
                        .expect("in view");
                    let in_view = in_view.unwrap();
                    blt_bgra_to_nv12(&vctx, &vp, &in_view, out_view).expect("blt");
                    nv12_tex
                } else {
                    &src_tex
                };

                let poll_ev = |need: &mut usize, have: &mut usize| {
                    if let Some(ev) = &ev_gen {
                        while let Ok(e) = ev.GetEvent(MF_EVENT_FLAG_NO_WAIT) {
                            if let Ok(t) = e.GetType() {
                                if t == METransformNeedInput.0 as u32 {
                                    *need += 1;
                                } else if t == METransformHaveOutput.0 as u32 {
                                    *have += 1;
                                }
                            }
                        }
                    }
                };
                if is_async {
                    for _ in 0..20 {
                        poll_ev(&mut async_need, &mut async_out);
                        if async_need > 0 {
                            break;
                        }
                        std::thread::sleep(std::time::Duration::from_millis(2));
                    }
                }
                let pts = idx as i64 * frame_dur;
                if !is_async || async_need > 0 {
                    if let Ok(buf) = MFCreateDXGISurfaceBuffer(&ID3D11Texture2D::IID, enc_tex, 0, false) {
                        if let Ok(len) = buf.GetMaxLength() {
                            let _ = buf.SetCurrentLength(len);
                        }
                        if let Ok(sample) = MFCreateSample() {
                            let _ = sample.AddBuffer(&buf);
                            let _ = sample.SetSampleTime(pts);
                            let _ = sample.SetSampleDuration(frame_dur);
                            if mft.ProcessInput(0, &sample, 0).is_ok() {
                                in_flight.push_back(start_qpc + pts);
                                if is_async && async_need > 0 {
                                    async_need -= 1;
                                }
                            }
                        }
                    }
                }
                if is_async {
                    poll_ev(&mut async_need, &mut async_out);
                }
                let pkts = drain_encoder_outputs(
                    &mft,
                    cfg.codec,
                    start_qpc,
                    start_qpc + pts,
                    frame_dur,
                    &mut seq_header,
                    &mut in_flight,
                    if is_async { Some(&mut async_out) } else { None },
                );
                for p in pkts {
                    total_bytes += p.data.len();
                }
            }
            let _ = mft.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0);
            for _ in 0..30 {
                if is_async {
                    if let Some(ev) = &ev_gen {
                        while let Ok(e) = ev.GetEvent(MF_EVENT_FLAG_NO_WAIT) {
                            if let Ok(t) = e.GetType() {
                                if t == METransformHaveOutput.0 as u32 {
                                    async_out += 1;
                                }
                            }
                        }
                    }
                }
                let pkts = drain_encoder_outputs(
                    &mft,
                    cfg.codec,
                    start_qpc,
                    start_qpc,
                    frame_dur,
                    &mut seq_header,
                    &mut in_flight,
                    if is_async { Some(&mut async_out) } else { None },
                );
                if pkts.is_empty() {
                    std::thread::sleep(std::time::Duration::from_millis(5));
                } else {
                    for p in pkts {
                        total_bytes += p.data.len();
                    }
                }
            }
            total_bytes
        }
    }

    #[test]
    fn detects_real_dxgi_gpu_and_encoders() {
        let hw = detect_hardware();
        assert!(!hw.gpu_name.is_empty());
        assert_ne!(hw.gpu_name, "Unknown GPU");
        assert!(hw.vendor_id != 0);
    }

    #[test]
    fn cqp_and_cbr_rate_control_actually_scale_output_bytes() {
        let mut cfg_q16 = AppConfig::default();
        cfg_q16.rate_control = RateControl::Cqp;
        cfg_q16.cqp_qp = 16;
        let bytes_q16 = encode_synthetic_bytes(&cfg_q16, 45);

        let mut cfg_q28 = AppConfig::default();
        cfg_q28.rate_control = RateControl::Cqp;
        cfg_q28.cqp_qp = 28;
        let bytes_q28 = encode_synthetic_bytes(&cfg_q28, 45);

        let mut cfg_b6 = AppConfig::default();
        cfg_b6.rate_control = RateControl::Cbr;
        cfg_b6.bitrate_mbps = 6;
        let bytes_b6 = encode_synthetic_bytes(&cfg_b6, 120);

        let mut cfg_b45 = AppConfig::default();
        cfg_b45.rate_control = RateControl::Cbr;
        cfg_b45.bitrate_mbps = 45;
        let bytes_b45 = encode_synthetic_bytes(&cfg_b45, 120);

        eprintln!(
            "RATE_CONTROL_TEST: q16={bytes_q16} q28={bytes_q28} b6={bytes_b6} b45={bytes_b45}"
        );
        assert!(bytes_q16 > (bytes_q28 * 13 / 10), "CQP 16 ({bytes_q16}) should be much larger than CQP 28 ({bytes_q28})");
        assert!(bytes_b45 > (bytes_b6 * 2), "CBR 45Mbps ({bytes_b45}) should be >2x CBR 6Mbps ({bytes_b6})");
    }

    #[test]
    fn h264_cqp_pipeline_emits_video_packets() {
        let mut cfg = AppConfig::default();
        cfg.codec = VideoCodec::H264;
        cfg.encoder_preference = EncoderPreference::Amf;
        cfg.rate_control = RateControl::Cqp;
        cfg.cqp_qp = 19;
        cfg.fps = crate::config::FrameRate::Fps120;
        let hw = detect_hardware();
        let mut pipe = GpuCapturePipeline::open(&cfg, &hw).expect("open pipeline");
        assert!(pipe.encoder.is_some(), "encoder should be Some");
        let mut pkts = Vec::new();
        for _ in 0..120 {
            pkts.extend(pipe.capture_and_encode_frame());
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        let mut max_gap_ms = 0_i64;
        for w in pkts.windows(2) {
            let gap_ms = (w[1].pts_100ns - w[0].pts_100ns) / 10_000;
            if gap_ms > max_gap_ms {
                max_gap_ms = gap_ms;
            }
        }
        eprintln!(
            "H264_CQP_PKTS: total={} idr={} async={} started={} need_in={} have_out={} max_gap_ms={}",
            pkts.len(),
            pkts.iter().filter(|p| p.is_idr_keyframe).count(),
            pipe.is_async_mft,
            pipe.started,
            pipe.need_input_count,
            pipe.have_output_count,
            max_gap_ms,
        );
        assert!(!pkts.is_empty(), "expected video packets");
    }
}
