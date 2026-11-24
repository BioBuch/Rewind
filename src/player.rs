use std::collections::VecDeque;
use std::mem::ManuallyDrop;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use egui::{Color32, ColorImage};
use image::{imageops::FilterType, ImageBuffer, Rgb, Rgba};
use windows::core::{Interface, GUID, PCWSTR};
use windows::Win32::Foundation::RECT;
use windows::Win32::Graphics::Direct3D::{
    D3D_DRIVER_TYPE_HARDWARE, D3D_FEATURE_LEVEL_11_0, D3D_FEATURE_LEVEL_11_1,
};
use windows::Win32::Graphics::Direct3D11::{
    D3D11CreateDevice, ID3D11Device, ID3D11DeviceContext, ID3D11Multithread, ID3D11Texture2D,
    ID3D11VideoContext, ID3D11VideoDevice, ID3D11VideoProcessor, ID3D11VideoProcessorEnumerator,
    ID3D11VideoProcessorOutputView, D3D11_BIND_RENDER_TARGET,
    D3D11_CPU_ACCESS_READ, D3D11_CREATE_DEVICE_BGRA_SUPPORT, D3D11_CREATE_DEVICE_VIDEO_SUPPORT,
    D3D11_MAPPED_SUBRESOURCE, D3D11_MAP_READ, D3D11_SDK_VERSION, D3D11_TEX2D_VPIV,
    D3D11_TEX2D_VPOV, D3D11_TEXTURE2D_DESC, D3D11_USAGE_DEFAULT, D3D11_USAGE_STAGING,
    D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE, D3D11_VIDEO_PROCESSOR_COLOR_SPACE,
    D3D11_VIDEO_PROCESSOR_CONTENT_DESC, D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC,
    D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0, D3D11_VIDEO_PROCESSOR_STREAM,
    D3D11_VIDEO_USAGE_OPTIMAL_SPEED, D3D11_VPIV_DIMENSION_TEXTURE2D,
    D3D11_VPOV_DIMENSION_TEXTURE2D,
};
use windows::Win32::Graphics::Dxgi::Common::{
    DXGI_FORMAT_B8G8R8A8_UNORM, DXGI_FORMAT_R8G8B8A8_UNORM, DXGI_RATIONAL, DXGI_SAMPLE_DESC,
};
use windows::Win32::Graphics::Dxgi::IDXGIDevice;
use windows::Win32::Media::MediaFoundation::{
    IMFAttributes, IMFDXGIBuffer, IMFDXGIDeviceManager, IMFMediaBuffer, IMFMediaType, IMFSample,
    IMFSourceReader, MFAudioFormat_PCM, MFCreateAttributes, MFCreateDXGIDeviceManager,
    MFCreateMediaType, MFCreateSourceReaderFromURL, MFShutdown, MFStartup, MFVideoArea,
    MFVideoFormat_NV12, MFVideoFormat_RGB32, MFSTARTUP_NOSOCKET, MF_LOW_LATENCY,
    MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_NUM_CHANNELS, MF_MT_AUDIO_SAMPLES_PER_SECOND,
    MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE, MF_MT_FRAME_SIZE, MF_MT_GEOMETRIC_APERTURE,
    MF_MT_MAJOR_TYPE, MF_MT_MINIMUM_DISPLAY_APERTURE, MF_MT_SUBTYPE, MF_MT_VIDEO_NOMINAL_RANGE,
    MF_MT_YUV_MATRIX, MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_D3D_MANAGER,
    MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_FIRST_AUDIO_STREAM,
    MF_SOURCE_READER_FIRST_VIDEO_STREAM, MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, COINIT_MULTITHREADED};

use crate::audio::PcmWaveOutPlayer;
use crate::win_ffi::to_wide_null;

const MF_MEDIATYPE_VIDEO: GUID = GUID::from_u128(0x73646976_0000_0010_8000_00aa00389b71);
const MF_MEDIATYPE_AUDIO: GUID = GUID::from_u128(0x73647561_0000_0010_8000_00aa00389b71);
const MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS: GUID =
    GUID::from_u128(0xa634a91c_822b_41b9_a494_4de4643612b0);

#[repr(C)]
struct RawI64PropVariant {
    vt: u16,
    w1: u16,
    w2: u16,
    w3: u16,
    val: i64,
    _pad: u64,
}

fn unpack_u32_pair(packed: u64) -> (u32, u32) {
    ((packed >> 32) as u32, (packed & 0xFFFF_FFFF) as u32)
}

struct BlitPipeline {
    in_w: u32,
    in_h: u32,
    crop_x: u32,
    crop_y: u32,
    out_w: u32,
    out_h: u32,
    output_is_bgra: bool,
    enumerator: ID3D11VideoProcessorEnumerator,
    processor: ID3D11VideoProcessor,
    rt_tex: ID3D11Texture2D,
    out_view: ID3D11VideoProcessorOutputView,
    staging_tex: ID3D11Texture2D,
}

struct DxvaVideoBlitter {
    device: ID3D11Device,
    context: ID3D11DeviceContext,
    video_device: ID3D11VideoDevice,
    video_context: ID3D11VideoContext,
    dxgi_manager: IMFDXGIDeviceManager,
    pipeline: Option<BlitPipeline>,
}

impl DxvaVideoBlitter {
    fn new() -> Option<Self> {
        unsafe {
            let levels = [D3D_FEATURE_LEVEL_11_1, D3D_FEATURE_LEVEL_11_0];
            let mut dev: Option<ID3D11Device> = None;
            let mut ctx: Option<ID3D11DeviceContext> = None;
            D3D11CreateDevice(
                None,
                D3D_DRIVER_TYPE_HARDWARE,
                None,
                D3D11_CREATE_DEVICE_VIDEO_SUPPORT | D3D11_CREATE_DEVICE_BGRA_SUPPORT,
                Some(&levels),
                D3D11_SDK_VERSION,
                Some(&mut dev),
                None,
                Some(&mut ctx),
            )
            .ok()?;

            let device = dev?;
            let context = ctx?;
            if let Ok(mt) = device.cast::<ID3D11Multithread>() {
                let _ = mt.SetMultithreadProtected(true);
            }
            if let Ok(dxgi_dev) = device.cast::<IDXGIDevice>() {
                let _ = dxgi_dev.SetGPUThreadPriority(-2);
            }
            let video_device: ID3D11VideoDevice = device.cast().ok()?;
            let video_context: ID3D11VideoContext = context.cast().ok()?;

            let mut reset_token = 0_u32;
            let mut mgr: Option<IMFDXGIDeviceManager> = None;
            MFCreateDXGIDeviceManager(&mut reset_token, &mut mgr).ok()?;
            let dxgi_manager = mgr?;
            dxgi_manager.ResetDevice(&device, reset_token).ok()?;

            Some(Self {
                device,
                context,
                video_device,
                video_context,
                dxgi_manager,
                pipeline: None,
            })
        }
    }

    unsafe fn ensure_pipeline(
        &mut self,
        in_w: u32,
        in_h: u32,
        crop_x: u32,
        crop_y: u32,
        out_w: u32,
        out_h: u32,
        fps: u32,
        yuv_matrix_bit: u32,
        nominal_range: u32,
    ) -> Option<&mut BlitPipeline> {
        let rebuild = match &self.pipeline {
            Some(p) => {
                p.in_w != in_w
                    || p.in_h != in_h
                    || p.crop_x != crop_x
                    || p.crop_y != crop_y
                    || p.out_w != out_w
                    || p.out_h != out_h
            }
            None => true,
        };
        if rebuild {
            self.pipeline = None;
            let content = D3D11_VIDEO_PROCESSOR_CONTENT_DESC {
                InputFrameFormat: D3D11_VIDEO_FRAME_FORMAT_PROGRESSIVE,
                InputFrameRate: DXGI_RATIONAL {
                    Numerator: fps.max(1),
                    Denominator: 1,
                },
                InputWidth: in_w,
                InputHeight: in_h,
                OutputFrameRate: DXGI_RATIONAL {
                    Numerator: fps.max(1),
                    Denominator: 1,
                },
                OutputWidth: out_w,
                OutputHeight: out_h,
                Usage: D3D11_VIDEO_USAGE_OPTIMAL_SPEED,
            };
            let enumerator = self
                .video_device
                .CreateVideoProcessorEnumerator(&content)
                .ok()?;
            let processor = self.video_device.CreateVideoProcessor(&enumerator, 0).ok()?;

            let sx = crop_x.min(in_w.saturating_sub(out_w));
            let sy = crop_y.min(in_h.saturating_sub(out_h));
            let src_rect = RECT {
                left: sx as i32,
                top: sy as i32,
                right: (sx + out_w).min(in_w) as i32,
                bottom: (sy + out_h).min(in_h) as i32,
            };
            let dst_rect = RECT {
                left: 0,
                top: 0,
                right: out_w as i32,
                bottom: out_h as i32,
            };
            self.video_context
                .VideoProcessorSetStreamSourceRect(&processor, 0, true, Some(&src_rect));
            self.video_context
                .VideoProcessorSetStreamDestRect(&processor, 0, true, Some(&dst_rect));
            self.video_context
                .VideoProcessorSetOutputTargetRect(&processor, true, Some(&dst_rect));

            let stream_cs = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
                _bitfield: (yuv_matrix_bit << 2) | (nominal_range << 4),
            };
            let out_cs = D3D11_VIDEO_PROCESSOR_COLOR_SPACE {
                _bitfield: 2 << 4,
            };
            self.video_context
                .VideoProcessorSetStreamColorSpace(&processor, 0, &stream_cs);
            self.video_context
                .VideoProcessorSetOutputColorSpace(&processor, &out_cs);

            let out_desc = D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC {
                ViewDimension: D3D11_VPOV_DIMENSION_TEXTURE2D,
                Anonymous: D3D11_VIDEO_PROCESSOR_OUTPUT_VIEW_DESC_0 {
                    Texture2D: D3D11_TEX2D_VPOV { MipSlice: 0 },
                },
            };

            let mut chosen = None;
            for (fmt, is_bgra) in [
                (DXGI_FORMAT_R8G8B8A8_UNORM, false),
                (DXGI_FORMAT_B8G8R8A8_UNORM, true),
            ] {
                let rt_desc = D3D11_TEXTURE2D_DESC {
                    Width: out_w,
                    Height: out_h,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: fmt,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_DEFAULT,
                    BindFlags: D3D11_BIND_RENDER_TARGET.0 as u32,
                    CPUAccessFlags: 0,
                    MiscFlags: 0,
                };
                let mut rt_opt = None;
                if self
                    .device
                    .CreateTexture2D(&rt_desc, None, Some(&mut rt_opt))
                    .is_err()
                {
                    continue;
                }
                let Some(rt_tex) = rt_opt else {
                    continue;
                };
                let mut ov_opt = None;
                if self
                    .video_device
                    .CreateVideoProcessorOutputView(&rt_tex, &enumerator, &out_desc, Some(&mut ov_opt))
                    .is_err()
                {
                    continue;
                }
                let Some(out_view) = ov_opt else {
                    continue;
                };
                let stg_desc = D3D11_TEXTURE2D_DESC {
                    Width: out_w,
                    Height: out_h,
                    MipLevels: 1,
                    ArraySize: 1,
                    Format: fmt,
                    SampleDesc: DXGI_SAMPLE_DESC {
                        Count: 1,
                        Quality: 0,
                    },
                    Usage: D3D11_USAGE_STAGING,
                    BindFlags: 0,
                    CPUAccessFlags: D3D11_CPU_ACCESS_READ.0 as u32,
                    MiscFlags: 0,
                };
                let mut stg_opt = None;
                if self
                    .device
                    .CreateTexture2D(&stg_desc, None, Some(&mut stg_opt))
                    .is_ok()
                {
                    if let Some(staging_tex) = stg_opt {
                        chosen = Some((rt_tex, out_view, staging_tex, is_bgra));
                        break;
                    }
                }
            }
            let (rt_tex, out_view, staging_tex, output_is_bgra) = chosen?;
            self.pipeline = Some(BlitPipeline {
                in_w,
                in_h,
                crop_x,
                crop_y,
                out_w,
                out_h,
                output_is_bgra,
                enumerator,
                processor,
                rt_tex,
                out_view,
                staging_tex,
            });
        }
        self.pipeline.as_mut()
    }

    unsafe fn sample_to_color_image(
        &mut self,
        sample: &IMFSample,
        crop_x: u32,
        crop_y: u32,
        out_w: u32,
        out_h: u32,
        fps: u32,
        yuv_matrix_bit: u32,
        nominal_range: u32,
        pool: &FrameBufferPool,
    ) -> Option<ColorImage> {
        let buf = sample.GetBufferByIndex(0).ok()?;
        let dxgi_buf: IMFDXGIBuffer = buf.cast().ok()?;
        let mut raw: *mut std::ffi::c_void = std::ptr::null_mut();
        dxgi_buf
            .GetResource(&ID3D11Texture2D::IID, &mut raw)
            .ok()?;
        if raw.is_null() {
            return None;
        }
        let src_tex = ID3D11Texture2D::from_raw(raw);
        let subresource = dxgi_buf.GetSubresourceIndex().unwrap_or(0);

        let mut src_desc = D3D11_TEXTURE2D_DESC::default();
        src_tex.GetDesc(&mut src_desc);
        let in_w = src_desc.Width.max(crop_x + out_w);
        let in_h = src_desc.Height.max(crop_y + out_h);

        let (vp, enumerator, out_view, rt_tex, staging_tex, is_bgra) = {
            let p = self.ensure_pipeline(
                in_w,
                in_h,
                crop_x,
                crop_y,
                out_w,
                out_h,
                fps,
                yuv_matrix_bit,
                nominal_range,
            )?;
            (
                p.processor.clone(),
                p.enumerator.clone(),
                p.out_view.clone(),
                p.rt_tex.clone(),
                p.staging_tex.clone(),
                p.output_is_bgra,
            )
        };

        let in_desc = D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC {
            FourCC: 0,
            ViewDimension: D3D11_VPIV_DIMENSION_TEXTURE2D,
            Anonymous: D3D11_VIDEO_PROCESSOR_INPUT_VIEW_DESC_0 {
                Texture2D: D3D11_TEX2D_VPIV {
                    MipSlice: 0,
                    ArraySlice: subresource,
                },
            },
        };
        let mut iv_opt = None;
        self.video_device
            .CreateVideoProcessorInputView(&src_tex, &enumerator, &in_desc, Some(&mut iv_opt))
            .ok()?;
        let in_view = iv_opt?;

        let mut stream = D3D11_VIDEO_PROCESSOR_STREAM {
            Enable: true.into(),
            OutputIndex: 0,
            InputFrameOrField: 0,
            PastFrames: 0,
            FutureFrames: 0,
            ppPastSurfaces: std::ptr::null_mut(),
            pInputSurface: ManuallyDrop::new(Some(in_view)),
            ppFutureSurfaces: std::ptr::null_mut(),
            ppPastSurfacesRight: std::ptr::null_mut(),
            pInputSurfaceRight: ManuallyDrop::new(None),
            ppFutureSurfacesRight: std::ptr::null_mut(),
        };
        let blt_res =
            self.video_context
                .VideoProcessorBlt(&vp, &out_view, 0, std::slice::from_ref(&stream));
        ManuallyDrop::drop(&mut stream.pInputSurface);
        drop(src_tex);
        blt_res.ok()?;

        self.context.CopyResource(&staging_tex, &rt_tex);
        let mut mapped = D3D11_MAPPED_SUBRESOURCE::default();
        self.context
            .Map(&staging_tex, 0, D3D11_MAP_READ, 0, Some(&mut mapped))
            .ok()?;

        let wu = out_w as usize;
        let hu = out_h as usize;
        let ok = if !mapped.pData.is_null() {
            let mut pixels = pool.take(wu * hu);
            let pitch = mapped.RowPitch as usize;
            let base = mapped.pData as *const u8;
            for y in 0..hu {
                let row = std::slice::from_raw_parts(base.add(y * pitch), wu * 4);
                let dst = &mut pixels[y * wu..(y + 1) * wu];
                if is_bgra {
                    for (px, out) in row.chunks_exact(4).zip(dst.iter_mut()) {
                        *out = Color32::from_rgb(px[2], px[1], px[0]);
                    }
                } else {
                    for (px, out) in row.chunks_exact(4).zip(dst.iter_mut()) {
                        *out = Color32::from_rgb(px[0], px[1], px[2]);
                    }
                }
            }
            self.context.Unmap(&staging_tex, 0);
            Some(ColorImage {
                size: [wu, hu],
                pixels,
            })
        } else {
            self.context.Unmap(&staging_tex, 0);
            None
        };
        ok
    }
}

unsafe fn read_display_crop_from_mt(mt: &IMFMediaType) -> Option<(u32, u32, u32, u32)> {
    let area_size = std::mem::size_of::<MFVideoArea>();
    for key in [&MF_MT_MINIMUM_DISPLAY_APERTURE, &MF_MT_GEOMETRIC_APERTURE] {
        let mut buf = [0_u8; 32];
        if area_size <= buf.len() {
            let mut got = 0_u32;
            if mt.GetBlob(key, &mut buf[..area_size], Some(&mut got)).is_ok()
                && got as usize == area_size
            {
                let area: MFVideoArea = std::ptr::read_unaligned(buf.as_ptr() as *const MFVideoArea);
                if area.Area.cx > 0 && area.Area.cy > 0 {
                    return Some((
                        area.OffsetX.value.max(0) as u32,
                        area.OffsetY.value.max(0) as u32,
                        area.Area.cx as u32,
                        area.Area.cy as u32,
                    ));
                }
            }
        }
    }
    None
}

unsafe fn read_native_dims(reader: &IMFSourceReader, stream_idx: u32) -> Option<(u32, u32)> {
    let nat_mt = reader.GetNativeMediaType(stream_idx, 0).ok()?;
    if let Some((_, _, aw, ah)) = read_display_crop_from_mt(&nat_mt) {
        return Some((aw, ah));
    }
    let packed = nat_mt.GetUINT64(&MF_MT_FRAME_SIZE).ok()?;
    let (w, h) = unpack_u32_pair(packed);
    if w > 0 && h > 0 {
        Some((w, h))
    } else {
        None
    }
}

struct HwVideoReader {
    reader: IMFSourceReader,
    blitter: Option<DxvaVideoBlitter>,
    container_dims: Option<(u32, u32)>,
    width: u32,
    height: u32,
    coded_height: u32,
    crop_x: u32,
    crop_y: u32,
    stride: u32,
    fps_num: u32,
    yuv_matrix_bit: u32,
    nominal_range: u32,
    is_nv12: bool,
}

impl HwVideoReader {
    fn open(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Err("Video file does not exist".to_owned());
        }
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).map_err(|e| format!("MFStartup: {e}"))?;

            let wide = to_wide_null(&path.to_string_lossy());
            let stream_idx = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

            let dxva_attempt = (|| -> Option<Self> {
                let blitter = DxvaVideoBlitter::new()?;
                let mut attrs: Option<IMFAttributes> = None;
                MFCreateAttributes(&mut attrs, 3).ok()?;
                let attrs = attrs?;
                attrs
                    .SetUnknown(&MF_SOURCE_READER_D3D_MANAGER, &blitter.dxgi_manager)
                    .ok()?;
                let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
                let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);

                let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), &attrs).ok()?;
                let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
                reader.SetStreamSelection(stream_idx, true).ok()?;
                let container_dims = read_native_dims(&reader, stream_idx);

                let nv12_mt = MFCreateMediaType().ok()?;
                nv12_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_VIDEO).ok()?;
                nv12_mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).ok()?;
                reader.SetCurrentMediaType(stream_idx, None, &nv12_mt).ok()?;

                let mut me = Self {
                    reader,
                    blitter: Some(blitter),
                    container_dims,
                    width: 1280,
                    height: 720,
                    coded_height: 720,
                    crop_x: 0,
                    crop_y: 0,
                    stride: 1280,
                    fps_num: 60,
                    yuv_matrix_bit: 1,
                    nominal_range: 1,
                    is_nv12: true,
                };
                me.refresh_media_type();
                Some(me)
            })();

            if let Some(me) = dxva_attempt {
                return Ok(me);
            }

            let sw_attempt = (|| -> Result<Self, String> {
                let mut attrs: Option<IMFAttributes> = None;
                MFCreateAttributes(&mut attrs, 3).map_err(|e| e.to_string())?;
                let attrs = attrs.ok_or_else(|| "Missing IMFAttributes".to_owned())?;
                let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
                let _ = attrs.SetUINT32(&MF_LOW_LATENCY, 1);

                let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), &attrs)
                    .map_err(|e| format!("MFCreateSourceReaderFromURL: {e}"))?;
                let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
                reader
                    .SetStreamSelection(stream_idx, true)
                    .map_err(|e| format!("SetStreamSelection: {e}"))?;
                let container_dims = read_native_dims(&reader, stream_idx);

                let nv12_mt = MFCreateMediaType().map_err(|e| e.to_string())?;
                nv12_mt
                    .SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_VIDEO)
                    .map_err(|e| e.to_string())?;
                nv12_mt
                    .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)
                    .map_err(|e| e.to_string())?;
                let is_nv12 = reader.SetCurrentMediaType(stream_idx, None, &nv12_mt).is_ok();
                if !is_nv12 {
                    drop(reader);
                    let mut attrs2: Option<IMFAttributes> = None;
                    MFCreateAttributes(&mut attrs2, 2).map_err(|e| e.to_string())?;
                    let attrs2 = attrs2.ok_or_else(|| "Missing IMFAttributes".to_owned())?;
                    let _ = attrs2.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
                    let _ = attrs2.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1);
                    let reader2 = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), &attrs2)
                        .map_err(|e| e.to_string())?;
                    let _ =
                        reader2.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
                    reader2
                        .SetStreamSelection(stream_idx, true)
                        .map_err(|e| e.to_string())?;
                    let rgb_mt = MFCreateMediaType().map_err(|e| e.to_string())?;
                    rgb_mt
                        .SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_VIDEO)
                        .map_err(|e| e.to_string())?;
                    rgb_mt
                        .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
                        .map_err(|e| e.to_string())?;
                    reader2
                        .SetCurrentMediaType(stream_idx, None, &rgb_mt)
                        .map_err(|e| format!("SetCurrentMediaType RGB32: {e}"))?;
                    let mut me = Self {
                        reader: reader2,
                        blitter: None,
                        container_dims,
                        width: 1280,
                        height: 720,
                        coded_height: 720,
                        crop_x: 0,
                        crop_y: 0,
                        stride: 1280,
                        fps_num: 60,
                        yuv_matrix_bit: 1,
                        nominal_range: 1,
                        is_nv12: false,
                    };
                    me.refresh_media_type();
                    return Ok(me);
                }
                let mut me = Self {
                    reader,
                    blitter: None,
                    container_dims,
                    width: 1280,
                    height: 720,
                    coded_height: 720,
                    crop_x: 0,
                    crop_y: 0,
                    stride: 1280,
                    fps_num: 60,
                    yuv_matrix_bit: 1,
                    nominal_range: 1,
                    is_nv12: true,
                };
                me.refresh_media_type();
                Ok(me)
            })();

            if sw_attempt.is_err() {
                let _ = MFShutdown();
            }
            sw_attempt
        }
    }

    unsafe fn refresh_media_type(&mut self) {
        let stream_idx = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        if let Ok(cur_mt) = self.reader.GetCurrentMediaType(stream_idx) {
            let packed = cur_mt
                .GetUINT64(&MF_MT_FRAME_SIZE)
                .unwrap_or(((1280_u64) << 32) | 720);
            let (coded_w, coded_h) = unpack_u32_pair(packed);
            self.coded_height = coded_h.max(2);
            if let Some((cx, cy, aw, ah)) = read_display_crop_from_mt(&cur_mt) {
                self.crop_x = cx;
                self.crop_y = cy;
                self.width = aw.min(coded_w).max(2);
                self.height = ah.min(coded_h).max(2);
            } else if let Some((cw, ch)) = self.container_dims {
                self.crop_x = 0;
                self.crop_y = 0;
                self.width = if cw > 0 && cw <= coded_w { cw } else { coded_w }.max(2);
                self.height = if ch > 0 && ch <= coded_h { ch } else { coded_h }.max(2);
            } else {
                self.crop_x = 0;
                self.crop_y = 0;
                self.width = coded_w.max(2);
                self.height = coded_h.max(2);
            }
            let raw_stride = cur_mt
                .GetUINT32(&MF_MT_DEFAULT_STRIDE)
                .unwrap_or(coded_w.max(self.width)) as i32;
            self.stride = raw_stride.unsigned_abs().max(coded_w).max(self.width).max(2);
            let fps_packed = cur_mt
                .GetUINT64(&MF_MT_FRAME_RATE)
                .unwrap_or(((60_u64) << 32) | 1);
            let (num, den) = unpack_u32_pair(fps_packed);
            if num > 0 && den > 0 {
                self.fps_num = (num / den).clamp(15, 240);
            }
            self.yuv_matrix_bit = match cur_mt.GetUINT32(&MF_MT_YUV_MATRIX) {
                Ok(2) => 0,
                Ok(1) => 1,
                _ => {
                    if self.height >= 720 {
                        1
                    } else {
                        0
                    }
                }
            };
            self.nominal_range = match cur_mt.GetUINT32(&MF_MT_VIDEO_NOMINAL_RANGE) {
                Ok(1) => 2,
                _ => 1,
            };
        }
    }

    fn seek(&mut self, secs: f32) {
        let pos_100ns = (secs.max(0.0) * 10_000_000.0) as i64;
        let pv = RawI64PropVariant {
            vt: 20,
            w1: 0,
            w2: 0,
            w3: 0,
            val: pos_100ns,
            _pad: 0,
        };
        unsafe {
            let _ = self.reader.SetCurrentPosition(
                &GUID::zeroed(),
                &pv as *const _ as *const windows::core::PROPVARIANT,
            );
        }
    }

    fn decode_sample(&mut self, sample: &IMFSample, pool: &FrameBufferPool) -> Option<ColorImage> {
        unsafe {
            if let Some(blitter) = &mut self.blitter {
                if let Some(img) = blitter.sample_to_color_image(
                    sample,
                    self.crop_x,
                    self.crop_y,
                    self.width,
                    self.height,
                    self.fps_num,
                    self.yuv_matrix_bit,
                    self.nominal_range,
                    pool,
                ) {
                    return Some(img);
                }
            }

            let Ok(buf): Result<IMFMediaBuffer, _> = sample.ConvertToContiguousBuffer() else {
                return None;
            };
            let mut ptr: *mut u8 = std::ptr::null_mut();
            let mut len = 0_u32;
            if buf.Lock(&mut ptr, None, Some(&mut len)).is_err() {
                return None;
            }
            let out = if !ptr.is_null() && len > 0 {
                let raw = std::slice::from_raw_parts(ptr, len as usize);
                if self.is_nv12 {
                    let need = (self.stride as usize) * (self.coded_height as usize) * 3 / 2;
                    if raw.len() >= need {
                        Some(nv12_to_full_color_image(
                            raw,
                            self.stride,
                            self.coded_height,
                            self.width,
                            self.height,
                            pool,
                        ))
                    } else {
                        None
                    }
                } else {
                    let expected = (self.width as usize) * (self.height as usize) * 4;
                    if raw.len() >= expected {
                        Some(bgra_to_full_color_image(
                            &raw[..expected],
                            self.width,
                            self.height,
                            pool,
                        ))
                    } else {
                        None
                    }
                }
            } else {
                None
            };
            let _ = buf.Unlock();
            out
        }
    }

    fn read_next_frame(&mut self, pool: &FrameBufferPool) -> Result<Option<(f32, ColorImage)>, ()> {
        let stream_idx = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        unsafe {
            for _ in 0..8 {
                let mut flags = 0_u32;
                let mut ts_100ns = 0_i64;
                let mut sample: Option<IMFSample> = None;
                if self
                    .reader
                    .ReadSample(
                        stream_idx,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut ts_100ns),
                        Some(&mut sample),
                    )
                    .is_err()
                {
                    return Err(());
                }
                if (flags & 0x10) != 0 {
                    self.refresh_media_type();
                }
                if (flags & 0x02) != 0 {
                    return Ok(None);
                }
                if let Some(s) = sample {
                    if let Some(img) = self.decode_sample(&s, pool) {
                        let pts = ((ts_100ns as f32) / 10_000_000.0).max(0.0);
                        return Ok(Some((pts, img)));
                    }
                }
            }
        }
        Err(())
    }

    fn seek_and_land(
        &mut self,
        target_secs: f32,
        pool: &FrameBufferPool,
        cancel: &AtomicBool,
    ) -> Option<(f32, ColorImage)> {
        self.seek(target_secs);
        let stream_idx = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        let threshold = (target_secs - 0.025).max(0.0);
        let mut last_sample: Option<(f32, IMFSample)> = None;

        unsafe {
            for _ in 0..180 {
                if cancel.load(Ordering::Acquire) {
                    return None;
                }
                let mut flags = 0_u32;
                let mut ts_100ns = 0_i64;
                let mut sample: Option<IMFSample> = None;
                if self
                    .reader
                    .ReadSample(
                        stream_idx,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut ts_100ns),
                        Some(&mut sample),
                    )
                    .is_err()
                {
                    break;
                }
                if (flags & 0x10) != 0 {
                    self.refresh_media_type();
                }
                if (flags & 0x02) != 0 {
                    break;
                }
                if let Some(s) = sample {
                    let pts = ((ts_100ns as f32) / 10_000_000.0).max(0.0);
                    if pts >= threshold {
                        if let Some(img) = self.decode_sample(&s, pool) {
                            return Some((pts, img));
                        }
                    }
                    last_sample = Some((pts, s));
                }
            }
            if let Some((pts, s)) = last_sample {
                return self.decode_sample(&s, pool).map(|img| (pts, img));
            }
        }
        None
    }
}

impl Drop for HwVideoReader {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
        }
    }
}

fn commit_u8_pages(buf: &mut [u8]) {
    if buf.is_empty() {
        return;
    }
    let ptr = buf.as_mut_ptr();
    let len = buf.len();
    let mut i = 0_usize;
    while i < len {
        unsafe {
            std::ptr::write_volatile(ptr.add(i), 0);
        }
        i += 4096;
    }
    unsafe {
        std::ptr::write_volatile(ptr.add(len - 1), 0);
    }
}

fn preload_audio_pcm(path: &Path, dur_hint: f32, cancel: &AtomicBool) -> Vec<u8> {
    if !path.exists() || cancel.load(Ordering::Acquire) {
        return Vec::new();
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        if MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).is_err() {
            return Vec::new();
        }
        let pcm = (|| -> Option<Vec<u8>> {
            let wide = to_wide_null(&path.to_string_lossy());
            let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), None).ok()?;
            let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
            let audio_idx = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
            reader.SetStreamSelection(audio_idx, true).ok()?;
            if !configure_audio_pcm_stream(&reader) {
                return None;
            }
            let max_pcm_bytes: usize = 128 * 1024 * 1024;
            let est_bytes = (((dur_hint.max(1.0) + 1.0) * 48_000.0 * 4.0) as usize)
                .clamp(48_000 * 4, max_pcm_bytes);
            let mut out = Vec::new();
            if out.try_reserve_exact(est_bytes).is_ok() {
                out.resize(est_bytes, 0);
                commit_u8_pages(&mut out);
                out.clear();
            }
            loop {
                if cancel.load(Ordering::Acquire) {
                    return Some(out);
                }
                let mut flags = 0_u32;
                let mut ts = 0_i64;
                let mut sample: Option<IMFSample> = None;
                if reader
                    .ReadSample(
                        audio_idx,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut ts),
                        Some(&mut sample),
                    )
                    .is_err()
                {
                    break;
                }
                if (flags & 0x02) != 0 {
                    break;
                }
                let Some(s) = sample else {
                    continue;
                };
                let Ok(buf) = s.ConvertToContiguousBuffer() else {
                    continue;
                };
                let mut ptr: *mut u8 = std::ptr::null_mut();
                let mut len = 0_u32;
                if buf.Lock(&mut ptr, None, Some(&mut len)).is_err() {
                    continue;
                }
                if !ptr.is_null() && len > 0 {
                    let take = (len as usize).min(max_pcm_bytes.saturating_sub(out.len()));
                    if take > 0 {
                        out.extend_from_slice(std::slice::from_raw_parts(ptr, take));
                    }
                }
                let _ = buf.Unlock();
                if out.len() >= max_pcm_bytes {
                    break;
                }
            }
            Some(out)
        })()
        .unwrap_or_default();
        let _ = MFShutdown();
        pcm
    }
}

struct SyncMfReader {
    reader: IMFSourceReader,
    width: u32,
    height: u32,
    stride: u32,
    is_nv12: bool,
    has_audio: bool,
}

impl SyncMfReader {
    fn open(path: &Path) -> Result<Self, String> {
        if !path.exists() {
            return Err("Video file does not exist".to_owned());
        }
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).map_err(|e| format!("MFStartup: {e}"))?;

            let wide = to_wide_null(&path.to_string_lossy());
            let stream_idx = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;

            let nv12_attempt = (|| -> Option<(IMFSourceReader, u32, u32, u32)> {
                let mut attrs: Option<IMFAttributes> = None;
                MFCreateAttributes(&mut attrs, 1).ok()?;
                let attrs = attrs?;
                let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);

                let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), &attrs).ok()?;
                let nv12_mt = MFCreateMediaType().ok()?;
                nv12_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_VIDEO).ok()?;
                nv12_mt.SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12).ok()?;
                reader.SetCurrentMediaType(stream_idx, None, &nv12_mt).ok()?;

                let cur_mt = reader.GetCurrentMediaType(stream_idx).ok()?;
                let packed = cur_mt
                    .GetUINT64(&MF_MT_FRAME_SIZE)
                    .unwrap_or(((1280_u64) << 32) | 720);
                let (w, h) = unpack_u32_pair(packed);
                let raw_stride = cur_mt.GetUINT32(&MF_MT_DEFAULT_STRIDE).unwrap_or(w) as i32;
                let stride = raw_stride.unsigned_abs().max(w).max(1);
                Some((reader, w.max(1), h.max(1), stride))
            })();

            if let Some((reader, width, height, stride)) = nv12_attempt {
                let has_audio = configure_audio_pcm_stream(&reader);
                return Ok(Self {
                    reader,
                    width,
                    height,
                    stride,
                    is_nv12: true,
                    has_audio,
                });
            }

            let res = (|| -> Result<Self, String> {
                let mut attrs: Option<IMFAttributes> = None;
                MFCreateAttributes(&mut attrs, 2).map_err(|e| format!("MFCreateAttributes: {e}"))?;
                let attrs = attrs.ok_or_else(|| "Missing IMFAttributes".to_owned())?;
                let _ = attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
                let _ = attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1);

                let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), &attrs)
                    .map_err(|e| format!("MFCreateSourceReaderFromURL: {e}"))?;
                let rgb_mt = MFCreateMediaType().map_err(|e| format!("MFCreateMediaType: {e}"))?;
                rgb_mt
                    .SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_VIDEO)
                    .map_err(|e| e.to_string())?;
                rgb_mt
                    .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_RGB32)
                    .map_err(|e| e.to_string())?;
                reader
                    .SetCurrentMediaType(stream_idx, None, &rgb_mt)
                    .map_err(|e| format!("SetCurrentMediaType RGB32: {e}"))?;

                let cur_mt = reader
                    .GetCurrentMediaType(stream_idx)
                    .map_err(|e| format!("GetCurrentMediaType: {e}"))?;
                let packed = cur_mt
                    .GetUINT64(&MF_MT_FRAME_SIZE)
                    .unwrap_or(((1280_u64) << 32) | 720);
                let (w, h) = unpack_u32_pair(packed);
                let has_audio = configure_audio_pcm_stream(&reader);
                Ok(Self {
                    reader,
                    width: w.max(1),
                    height: h.max(1),
                    stride: w.max(1),
                    is_nv12: false,
                    has_audio,
                })
            })();

            if res.is_err() {
                let _ = MFShutdown();
            }
            res
        }
    }

    fn seek(&mut self, secs: f32) {
        let pos_100ns = (secs.max(0.0) * 10_000_000.0) as i64;
        let pv = RawI64PropVariant {
            vt: 20,
            w1: 0,
            w2: 0,
            w3: 0,
            val: pos_100ns,
            _pad: 0,
        };
        unsafe {
            let _ = self.reader.SetCurrentPosition(
                &GUID::zeroed(),
                &pv as *const _ as *const windows::core::PROPVARIANT,
            );
        }
    }

    fn read_video_frame_rgba(&mut self, max_w: u32) -> Result<Option<(u32, u32, Vec<u8>, f32)>, ()> {
        let stream_idx = MF_SOURCE_READER_FIRST_VIDEO_STREAM.0 as u32;
        unsafe {
            for _ in 0..8 {
                let mut flags = 0_u32;
                let mut ts_100ns = 0_i64;
                let mut sample: Option<IMFSample> = None;
                if self
                    .reader
                    .ReadSample(
                        stream_idx,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut ts_100ns),
                        Some(&mut sample),
                    )
                    .is_err()
                {
                    return Err(());
                }
                if (flags & 0x02) != 0 {
                    return Ok(None);
                }
                if let Some(s) = sample {
                    let Ok(buf): Result<IMFMediaBuffer, _> = s.ConvertToContiguousBuffer() else {
                        continue;
                    };
                    let mut ptr: *mut u8 = std::ptr::null_mut();
                    let mut len = 0_u32;
                    if buf.Lock(&mut ptr, None, Some(&mut len)).is_err() {
                        continue;
                    }
                    let out = if !ptr.is_null() && len > 0 {
                        let raw = std::slice::from_raw_parts(ptr, len as usize);
                        if self.is_nv12 {
                            let need = (self.stride as usize) * (self.height as usize) * 3 / 2;
                            if raw.len() >= need {
                                Some(nv12_to_display_rgba(
                                    raw,
                                    self.stride,
                                    self.width,
                                    self.height,
                                    max_w,
                                ))
                            } else {
                                None
                            }
                        } else {
                            let expected = (self.width as usize) * (self.height as usize) * 4;
                            if raw.len() >= expected {
                                Some(bgra_to_display_rgba(
                                    &raw[..expected],
                                    self.width,
                                    self.height,
                                    max_w,
                                ))
                            } else {
                                None
                            }
                        }
                    } else {
                        None
                    };
                    let _ = buf.Unlock();
                    if let Some((dw, dh, rgba)) = out {
                        let pts = (ts_100ns as f32) / 10_000_000.0;
                        return Ok(Some((dw, dh, rgba, pts)));
                    }
                }
            }
        }
        Err(())
    }

    fn read_audio_sample(&mut self, gain: f32) -> Result<Option<(f32, Vec<u8>)>, ()> {
        if !self.has_audio {
            return Ok(None);
        }
        let audio_idx = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
        unsafe {
            let mut flags = 0_u32;
            let mut ts_100ns = 0_i64;
            let mut sample: Option<IMFSample> = None;
            if self
                .reader
                .ReadSample(
                    audio_idx,
                    0,
                    None,
                    Some(&mut flags),
                    Some(&mut ts_100ns),
                    Some(&mut sample),
                )
                .is_err()
            {
                return Err(());
            }
            if (flags & 0x02) != 0 {
                return Ok(None);
            }
            let Some(s) = sample else {
                return Err(());
            };
            let Ok(buf) = s.ConvertToContiguousBuffer() else {
                return Err(());
            };
            let mut ptr: *mut u8 = std::ptr::null_mut();
            let mut len = 0_u32;
            if buf.Lock(&mut ptr, None, Some(&mut len)).is_err() {
                return Err(());
            }
            let out = if !ptr.is_null() && len > 0 {
                let mut pcm = std::slice::from_raw_parts(ptr, len as usize).to_vec();
                if (gain - 1.0).abs() > 0.01 {
                    for chunk in pcm.chunks_exact_mut(2) {
                        let val = i16::from_le_bytes([chunk[0], chunk[1]]);
                        let scaled = ((val as f32) * gain).clamp(-32768.0, 32767.0) as i16;
                        let b = scaled.to_le_bytes();
                        chunk[0] = b[0];
                        chunk[1] = b[1];
                    }
                }
                let pts = (ts_100ns as f32) / 10_000_000.0;
                Some((pts, pcm))
            } else {
                None
            };
            let _ = buf.Unlock();
            Ok(out)
        }
    }
}

impl Drop for SyncMfReader {
    fn drop(&mut self) {
        unsafe {
            let _ = MFShutdown();
        }
    }
}

enum PlayerCmd {
    SetPlaying(bool),
    Seek(f32),
    SetGain(f32),
    Stop,
}

struct DecodedFrame {
    pts: f32,
    image: ColorImage,
}

struct PlayerShared {
    latest_frame: Option<ColorImage>,
    frame_seq: u64,
    pos_secs: f32,
    preload_ready: bool,
    preload_progress: f32,
}

struct FramePoolInner {
    pixel_len: usize,
    free: Vec<Vec<Color32>>,
    cap: usize,
}

#[derive(Clone)]
struct FrameBufferPool {
    inner: Arc<Mutex<FramePoolInner>>,
}

fn commit_color_pages(buf: &mut [Color32]) {
    if buf.is_empty() {
        return;
    }
    let stride = (4096 / std::mem::size_of::<Color32>()).max(1);
    let ptr = buf.as_mut_ptr();
    let len = buf.len();
    let mut i = 0_usize;
    while i < len {
        unsafe {
            std::ptr::write_volatile(ptr.add(i), Color32::BLACK);
        }
        i += stride;
    }
    unsafe {
        std::ptr::write_volatile(ptr.add(len - 1), Color32::BLACK);
    }
}

impl FrameBufferPool {
    fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(FramePoolInner {
                pixel_len: 0,
                free: Vec::new(),
                cap: 0,
            })),
        }
    }

    fn preallocate(&self, pixel_len: usize, count: usize) {
        if pixel_len == 0 || count == 0 {
            return;
        }
        let mut allocated = Vec::with_capacity(count);
        for _ in 0..count {
            let mut v = Vec::new();
            if v.try_reserve_exact(pixel_len).is_ok() {
                v.resize(pixel_len, Color32::BLACK);
                commit_color_pages(&mut v);
                allocated.push(v);
            } else {
                break;
            }
        }
        if let Ok(mut g) = self.inner.lock() {
            g.pixel_len = pixel_len;
            g.cap = count.max(4);
            g.free = allocated;
        }
    }

    fn take(&self, pixel_len: usize) -> Vec<Color32> {
        if let Ok(mut g) = self.inner.lock() {
            while let Some(mut v) = g.free.pop() {
                if v.capacity() >= pixel_len {
                    v.resize(pixel_len, Color32::BLACK);
                    return v;
                }
            }
        }
        let mut v = vec![Color32::BLACK; pixel_len];
        commit_color_pages(&mut v);
        v
    }

    fn recycle(&self, mut buf: Vec<Color32>) {
        if buf.capacity() == 0 {
            return;
        }
        if let Ok(mut g) = self.inner.lock() {
            if (g.pixel_len == 0 || buf.capacity() >= g.pixel_len) && g.free.len() < g.cap {
                buf.clear();
                g.free.push(buf);
            }
        }
    }

    fn recycle_image(&self, img: ColorImage) {
        self.recycle(img.pixels);
    }
}

pub struct VideoDecoder {
    cmd_tx: Sender<PlayerCmd>,
    shared: Arc<Mutex<PlayerShared>>,
    cancel: Arc<AtomicBool>,
    last_playing: AtomicBool,
    last_gain_bits: AtomicU32,
    pool: FrameBufferPool,
    worker: Option<JoinHandle<()>>,
}

impl VideoDecoder {
    pub fn open(path: &Path, duration_hint_secs: f32) -> Result<Self, String> {
        if !path.exists() {
            return Err("Video file does not exist".to_owned());
        }
        let dur_hint = if duration_hint_secs > 0.0 {
            duration_hint_secs
        } else {
            30.0
        };
        let path_owned = path.to_path_buf();
        let (cmd_tx, cmd_rx) = mpsc::channel::<PlayerCmd>();
        let shared = Arc::new(Mutex::new(PlayerShared {
            latest_frame: None,
            frame_seq: 0,
            pos_secs: 0.0,
            preload_ready: false,
            preload_progress: 0.0,
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let pool = FrameBufferPool::new();
        let shared_w = shared.clone();
        let cancel_w = cancel.clone();
        let pool_w = pool.clone();
        let worker = thread::spawn(move || {
            run_player_worker(path_owned, dur_hint, cmd_rx, shared_w, cancel_w, pool_w)
        });
        Ok(Self {
            cmd_tx,
            shared,
            cancel,
            last_playing: AtomicBool::new(true),
            last_gain_bits: AtomicU32::new(0.85_f32.to_bits()),
            pool,
            worker: Some(worker),
        })
    }

    pub fn recycle_frame_pixels(&self, pixels: Vec<Color32>) {
        self.pool.recycle(pixels);
    }

    pub fn set_playing(&self, playing: bool) {
        if self.last_playing.swap(playing, Ordering::AcqRel) != playing {
            let _ = self.cmd_tx.send(PlayerCmd::SetPlaying(playing));
        }
    }

    pub fn seek(&self, secs: f32) {
        let _ = self.cmd_tx.send(PlayerCmd::Seek(secs.max(0.0)));
    }

    pub fn set_audio_gain(&self, gain: f32) {
        let clamped = gain.clamp(0.0, 1.0);
        let prev = f32::from_bits(self.last_gain_bits.swap(clamped.to_bits(), Ordering::AcqRel));
        if (prev - clamped).abs() > 0.0005 {
            let _ = self.cmd_tx.send(PlayerCmd::SetGain(clamped));
        }
    }

    pub fn preload_progress(&self) -> (bool, f32) {
        self.shared
            .lock()
            .map(|g| (g.preload_ready, g.preload_progress))
            .unwrap_or((true, 1.0))
    }

    pub fn take_latest_frame(&self, last_seq: &mut u64) -> Option<ColorImage> {
        let mut guard = self.shared.lock().ok()?;
        if guard.frame_seq > *last_seq {
            *last_seq = guard.frame_seq;
            guard.latest_frame.take()
        } else {
            None
        }
    }

    pub fn current_pos_secs(&self) -> f32 {
        self.shared.lock().map(|g| g.pos_secs).unwrap_or(0.0)
    }
}

impl Drop for VideoDecoder {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
        let _ = self.cmd_tx.send(PlayerCmd::Stop);
        if let Some(h) = self.worker.take() {
            let _ = h.join();
        }
    }
}

fn buffer_capacity_for_width(w: u32) -> usize {
    if w > 2560 {
        10
    } else if w > 1920 {
        16
    } else {
        24
    }
}

fn push_frame_to_shared(
    shared: &Arc<Mutex<PlayerShared>>,
    img: ColorImage,
    pos_secs: Option<f32>,
    pool: &FrameBufferPool,
) {
    let displaced = if let Ok(mut g) = shared.lock() {
        let prev = g.latest_frame.replace(img);
        g.frame_seq += 1;
        if let Some(pos) = pos_secs {
            g.pos_secs = pos;
        }
        prev
    } else {
        None
    };
    if let Some(old) = displaced {
        pool.recycle_image(old);
    }
}

fn drain_queue_to_pool(queue: &mut VecDeque<DecodedFrame>, pool: &FrameBufferPool) {
    while let Some(f) = queue.pop_front() {
        pool.recycle_image(f.image);
    }
}

fn run_player_worker(
    path: std::path::PathBuf,
    dur_hint: f32,
    cmd_rx: Receiver<PlayerCmd>,
    shared: Arc<Mutex<PlayerShared>>,
    cancel: Arc<AtomicBool>,
    pool: FrameBufferPool,
) {
    let audio_path = path.clone();
    let audio_cancel = cancel.clone();
    let (audio_tx, audio_rx) = mpsc::channel::<Vec<u8>>();
    let audio_thread = thread::spawn(move || {
        let pcm = preload_audio_pcm(&audio_path, dur_hint, &audio_cancel);
        let _ = audio_tx.send(pcm);
    });

    let Ok(mut video) = HwVideoReader::open(&path) else {
        let _ = audio_thread.join();
        if let Ok(mut g) = shared.lock() {
            g.preload_ready = true;
            g.preload_progress = 1.0;
        }
        return;
    };

    let buffer_cap = buffer_capacity_for_width(video.width);
    let frame_pixels = (video.width as usize).saturating_mul(video.height as usize);
    pool.preallocate(frame_pixels, buffer_cap + 4);
    let mut queue: VecDeque<DecodedFrame> = VecDeque::with_capacity(buffer_cap + 2);
    let mut eof = false;
    let mut max_seen_pts = 0.0_f32;

    let mut playing = true;
    let mut audio_gain = 0.85_f32;
    let mut pending_seek: Option<f32> = None;

    for i in 0..buffer_cap {
        if cancel.load(Ordering::Acquire) {
            return;
        }
        while let Ok(cmd) = cmd_rx.try_recv() {
            match cmd {
                PlayerCmd::Stop => return,
                PlayerCmd::SetPlaying(p) => playing = p,
                PlayerCmd::Seek(s) => pending_seek = Some(s),
                PlayerCmd::SetGain(g) => audio_gain = g,
            }
        }
        match video.read_next_frame(&pool) {
            Ok(Some((pts, image))) => {
                max_seen_pts = max_seen_pts.max(pts);
                queue.push_back(DecodedFrame { pts, image });
                let prog = ((i + 1) as f32 / buffer_cap as f32).clamp(0.0, 0.99);
                if let Ok(mut g) = shared.lock() {
                    g.preload_progress = prog;
                }
            }
            Ok(None) | Err(()) => {
                eof = true;
                break;
            }
        }
    }

    let mut pcm_buf = Vec::new();
    let audio_deadline = Instant::now() + Duration::from_millis(2000);
    while Instant::now() < audio_deadline && !cancel.load(Ordering::Acquire) {
        match audio_rx.recv_timeout(Duration::from_millis(25)) {
            Ok(p) => {
                pcm_buf = p;
                break;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let mut audio_pending = pcm_buf.is_empty();
    let audio_dur = (pcm_buf.len() as f32) / (48_000.0 * 4.0);
    let mut total_dur = dur_hint.max(max_seen_pts).max(audio_dur).max(0.1);

    let mut wave_out = if !pcm_buf.is_empty() {
        PcmWaveOutPlayer::open()
    } else {
        None
    };

    if let Some(seek_t) = pending_seek.take() {
        let t = seek_t.clamp(0.0, total_dur);
        drain_queue_to_pool(&mut queue, &pool);
        eof = false;
        if let Some((pts, image)) = video.seek_and_land(t, &pool, &cancel) {
            push_frame_to_shared(&shared, image, Some(t.max(pts)), &pool);
        }
    } else if let Some(first) = queue.pop_front() {
        push_frame_to_shared(&shared, first.image, Some(0.0), &pool);
    }

    if let Ok(mut g) = shared.lock() {
        g.preload_progress = 1.0;
        g.preload_ready = true;
    }

    let start_pos = shared.lock().map(|g| g.pos_secs).unwrap_or(0.0);
    let mut audio_cursor = (((start_pos * 48_000.0) as usize) * 4).min(pcm_buf.len()) & !3;
    let mut play_anchor: Option<(Instant, f32)> = if playing {
        Some((Instant::now(), start_pos))
    } else {
        None
    };

    loop {
        if cancel.load(Ordering::Acquire) {
            break;
        }

        if audio_pending {
            if let Ok(p) = audio_rx.try_recv() {
                audio_pending = false;
                if !p.is_empty() {
                    let adur = (p.len() as f32) / (48_000.0 * 4.0);
                    total_dur = total_dur.max(adur);
                    pcm_buf = p;
                    wave_out = PcmWaveOutPlayer::open();
                    let cur = shared.lock().map(|g| g.pos_secs).unwrap_or(0.0);
                    audio_cursor = (((cur * 48_000.0) as usize) * 4).min(pcm_buf.len()) & !3;
                }
            }
        }

        let mut seek_req: Option<f32> = None;
        let mut stop = false;

        let mut mute_flushed = false;
        let handle_cmd = |cmd: PlayerCmd,
                          playing: &mut bool,
                          audio_gain: &mut f32,
                          seek_req: &mut Option<f32>,
                          stop: &mut bool,
                          mute_flushed: &mut bool,
                          wave_out: &mut Option<PcmWaveOutPlayer>,
                          play_anchor: &mut Option<(Instant, f32)>,
                          shared: &Arc<Mutex<PlayerShared>>,
                          total_dur: f32| {
            match cmd {
                PlayerCmd::Stop => *stop = true,
                PlayerCmd::SetPlaying(p) => {
                    if *playing != p {
                        *playing = p;
                        if !p {
                            if let Some(wo) = wave_out {
                                wo.reset();
                            }
                            if let Some((inst, origin_pts)) = *play_anchor {
                                let cur =
                                    (origin_pts + inst.elapsed().as_secs_f32()).clamp(0.0, total_dur);
                                if let Ok(mut g) = shared.lock() {
                                    g.pos_secs = cur;
                                }
                            }
                            *play_anchor = None;
                        } else {
                            let cur = shared.lock().map(|g| g.pos_secs).unwrap_or(0.0);
                            *play_anchor = Some((Instant::now(), cur));
                        }
                    }
                }
                PlayerCmd::Seek(s) => *seek_req = Some(s),
                PlayerCmd::SetGain(g) => {
                    if *audio_gain > 0.001 && g <= 0.001 {
                        if let Some(wo) = wave_out {
                            wo.reset();
                        }
                        *mute_flushed = true;
                    }
                    *audio_gain = g;
                }
            }
        };

        if !playing && seek_req.is_none() {
            if !eof && queue.len() < buffer_cap {
                while let Ok(c) = cmd_rx.try_recv() {
                    handle_cmd(
                        c,
                        &mut playing,
                        &mut audio_gain,
                        &mut seek_req,
                        &mut stop,
                        &mut mute_flushed,
                        &mut wave_out,
                        &mut play_anchor,
                        &shared,
                        total_dur,
                    );
                }
                if !stop && seek_req.is_none() && !playing {
                    match video.read_next_frame(&pool) {
                        Ok(Some((pts, image))) => {
                            total_dur = total_dur.max(pts);
                            queue.push_back(DecodedFrame { pts, image });
                        }
                        Ok(None) | Err(()) => eof = true,
                    }
                    continue;
                }
            } else {
                match cmd_rx.recv() {
                    Ok(c) => handle_cmd(
                        c,
                        &mut playing,
                        &mut audio_gain,
                        &mut seek_req,
                        &mut stop,
                        &mut mute_flushed,
                        &mut wave_out,
                        &mut play_anchor,
                        &shared,
                        total_dur,
                    ),
                    Err(_) => break,
                }
            }
        }

        while let Ok(c) = cmd_rx.try_recv() {
            handle_cmd(
                c,
                &mut playing,
                &mut audio_gain,
                &mut seek_req,
                &mut stop,
                &mut mute_flushed,
                &mut wave_out,
                &mut play_anchor,
                &shared,
                total_dur,
            );
        }
        if stop {
            break;
        }
        if mute_flushed {
            let cur = play_anchor
                .map(|(inst, origin_pts)| (origin_pts + inst.elapsed().as_secs_f32()).clamp(0.0, total_dur))
                .unwrap_or_else(|| shared.lock().map(|g| g.pos_secs).unwrap_or(0.0));
            audio_cursor = (((cur * 48_000.0) as usize) * 4).min(pcm_buf.len()) & !3;
        }

        if let Some(s) = seek_req {
            let t = s.clamp(0.0, total_dur);
            if let Some(wo) = &mut wave_out {
                wo.reset();
            }
            audio_cursor = (((t * 48_000.0) as usize) * 4).min(pcm_buf.len()) & !3;
            drain_queue_to_pool(&mut queue, &pool);
            eof = false;
            if let Some((_pts, image)) = video.seek_and_land(t, &pool, &cancel) {
                push_frame_to_shared(&shared, image, Some(t), &pool);
            } else if let Ok(mut g) = shared.lock() {
                g.pos_secs = t;
            }
            play_anchor = if playing {
                Some((Instant::now(), t))
            } else {
                None
            };
            continue;
        }

        if !playing {
            continue;
        }

        if play_anchor.is_none() {
            let cur = shared.lock().map(|g| g.pos_secs).unwrap_or(0.0);
            play_anchor = Some((Instant::now(), cur));
        }

        let (origin_inst, origin_pts) = play_anchor.unwrap();
        let target_pts = origin_pts + origin_inst.elapsed().as_secs_f32();
        if target_pts >= total_dur || (eof && queue.is_empty() && target_pts >= total_dur - 0.05) {
            if let Some(wo) = &mut wave_out {
                wo.reset();
            }
            audio_cursor = 0;
            drain_queue_to_pool(&mut queue, &pool);
            eof = false;
            if let Some((_pts, image)) = video.seek_and_land(0.0, &pool, &cancel) {
                push_frame_to_shared(&shared, image, Some(0.0), &pool);
            } else if let Ok(mut g) = shared.lock() {
                g.pos_secs = 0.0;
            }
            play_anchor = Some((Instant::now(), 0.0));
            continue;
        }

        let mut presented: Option<DecodedFrame> = None;
        while let Some(front) = queue.front() {
            if front.pts <= target_pts {
                if let Some(prev) = presented.replace(queue.pop_front().unwrap()) {
                    pool.recycle_image(prev.image);
                }
            } else {
                break;
            }
        }
        if let Some(frame) = presented {
            push_frame_to_shared(&shared, frame.image, Some(target_pts), &pool);
        } else if let Ok(mut g) = shared.lock() {
            g.pos_secs = target_pts;
        }

        if let Some(wo) = &mut wave_out {
            if !pcm_buf.is_empty() {
                let want_bytes =
                    ((((target_pts + 0.120) * 48_000.0) as usize) * 4).min(pcm_buf.len()) & !3;
                let chunk_bytes: usize = 7680;
                while audio_cursor + chunk_bytes <= want_bytes {
                    let slice = &pcm_buf[audio_cursor..(audio_cursor + chunk_bytes)];
                    let chunk = scale_pcm_chunk(slice, audio_gain);
                    wo.push_pcm(chunk);
                    audio_cursor += chunk_bytes;
                }
                if want_bytes == pcm_buf.len() && audio_cursor < want_bytes {
                    let slice = &pcm_buf[audio_cursor..want_bytes];
                    let chunk = scale_pcm_chunk(slice, audio_gain);
                    wo.push_pcm(chunk);
                    audio_cursor = want_bytes;
                }
            }
        }

        if !eof && queue.len() < buffer_cap {
            let burst = if queue.len() < 4 { 2 } else { 1 };
            for _ in 0..burst {
                if cancel.load(Ordering::Acquire) {
                    break;
                }
                match video.read_next_frame(&pool) {
                    Ok(Some((pts, image))) => {
                        total_dur = total_dur.max(pts);
                        queue.push_back(DecodedFrame { pts, image });
                    }
                    Ok(None) | Err(()) => {
                        eof = true;
                        break;
                    }
                }
            }
        }

        if let Some(front) = queue.front() {
            let now_pts = origin_pts + origin_inst.elapsed().as_secs_f32();
            if front.pts <= now_pts {
                let mut popped: Option<DecodedFrame> = None;
                while let Some(f) = queue.front() {
                    if f.pts <= now_pts {
                        if let Some(prev) = popped.replace(queue.pop_front().unwrap()) {
                            pool.recycle_image(prev.image);
                        }
                    } else {
                        break;
                    }
                }
                if let Some(frame) = popped {
                    push_frame_to_shared(&shared, frame.image, Some(now_pts), &pool);
                }
                continue;
            }
        }

        if queue.len() >= buffer_cap || eof {
            let wait_ms = queue
                .front()
                .map(|f| {
                    let now_pts = origin_pts + origin_inst.elapsed().as_secs_f32();
                    ((f.pts - now_pts) * 1000.0).clamp(1.0, 4.0) as u64
                })
                .unwrap_or(2);
            thread::sleep(Duration::from_millis(wait_ms));
        }
    }

    drop(video);
    let _ = audio_thread.join();
}

fn scale_pcm_chunk(slice: &[u8], gain: f32) -> Vec<u8> {
    let mut pcm = slice.to_vec();
    if (gain - 1.0).abs() > 0.01 {
        for ch in pcm.chunks_exact_mut(2) {
            let val = i16::from_le_bytes([ch[0], ch[1]]);
            let scaled = ((val as f32) * gain).clamp(-32768.0, 32767.0) as i16;
            let b = scaled.to_le_bytes();
            ch[0] = b[0];
            ch[1] = b[1];
        }
    }
    pcm
}

fn nv12_to_full_color_image(
    raw: &[u8],
    stride: u32,
    coded_h: u32,
    src_w: u32,
    src_h: u32,
    pool: &FrameBufferPool,
) -> ColorImage {
    let stride_u = (stride as usize).max(src_w as usize).max(1);
    let plane_h = ((raw.len() * 2) / (3 * stride_u)).max(coded_h as usize).max(src_h as usize);
    let uv_base = stride_u * plane_h;
    let wu = src_w as usize;
    let hu = src_h as usize;
    let mut pixels = pool.take(wu * hu);
    let len = raw.len();

    for y in 0..hu {
        let y_row = y * stride_u;
        let uv_row = uv_base + (y >> 1) * stride_u;
        let dst_row = &mut pixels[y * wu..(y + 1) * wu];
        if y_row + wu > len || uv_row + wu > len {
            dst_row.fill(Color32::BLACK);
            continue;
        }
        for x in 0..wu {
            let y_val = raw[y_row + x] as i32;
            let uv_x = x & !1;
            let u_val = raw[uv_row + uv_x] as i32;
            let v_val = raw[uv_row + uv_x + 1] as i32;

            let c = y_val - 16;
            let d = u_val - 128;
            let e = v_val - 128;

            let r = ((298 * c + 459 * e + 128) >> 8).clamp(0, 255) as u8;
            let g = ((298 * c - 55 * d - 136 * e + 128) >> 8).clamp(0, 255) as u8;
            let b = ((298 * c + 541 * d + 128) >> 8).clamp(0, 255) as u8;
            dst_row[x] = Color32::from_rgb(r, g, b);
        }
    }

    ColorImage {
        size: [wu, hu],
        pixels,
    }
}

fn bgra_to_full_color_image(
    bgra: &[u8],
    src_w: u32,
    src_h: u32,
    pool: &FrameBufferPool,
) -> ColorImage {
    let wu = src_w as usize;
    let hu = src_h as usize;
    let mut pixels = pool.take(wu * hu);
    for (px, out) in bgra[..(wu * hu * 4)]
        .chunks_exact(4)
        .zip(pixels.iter_mut())
    {
        *out = Color32::from_rgb(px[2], px[1], px[0]);
    }
    ColorImage {
        size: [wu, hu],
        pixels,
    }
}

fn nv12_to_display_rgba(
    nv12: &[u8],
    stride: u32,
    src_w: u32,
    src_h: u32,
    max_w: u32,
) -> (u32, u32, Vec<u8>) {
    let stride_u = (stride as usize).max(src_w as usize).max(1);
    let plane_h = ((nv12.len() * 2) / (3 * stride_u)).max(src_h as usize);
    let uv_base = stride_u * plane_h;

    let step = ((src_w as usize) / (max_w as usize).max(1)).max(1);
    let dst_w = ((src_w as usize) / step).max(1);
    let dst_h = ((src_h as usize) / step).max(1);
    let mut rgba = vec![255_u8; dst_w * dst_h * 4];
    let len = nv12.len();

    for y in 0..dst_h {
        let sy = y * step;
        let y_row = sy * stride_u;
        let uv_row = uv_base + (sy >> 1) * stride_u;
        let dst_row = &mut rgba[(y * dst_w * 4)..((y + 1) * dst_w * 4)];
        if y_row + (dst_w - 1) * step >= len || uv_row + ((dst_w - 1) * step) + 1 >= len {
            continue;
        }
        for x in 0..dst_w {
            let sx = x * step;
            let y_val = nv12[y_row + sx] as i32;
            let uv_x = sx & !1;
            let u_val = nv12[uv_row + uv_x] as i32;
            let v_val = nv12[uv_row + uv_x + 1] as i32;

            let c = y_val - 16;
            let d = u_val - 128;
            let e = v_val - 128;

            let r = (298 * c + 459 * e + 128) >> 8;
            let g = (298 * c - 55 * d - 136 * e + 128) >> 8;
            let b = (298 * c + 541 * d + 128) >> 8;

            let dp = x * 4;
            dst_row[dp] = r.clamp(0, 255) as u8;
            dst_row[dp + 1] = g.clamp(0, 255) as u8;
            dst_row[dp + 2] = b.clamp(0, 255) as u8;
            dst_row[dp + 3] = 255;
        }
    }

    (dst_w as u32, dst_h as u32, rgba)
}

fn bgra_to_display_rgba(
    bgra: &[u8],
    src_w: u32,
    src_h: u32,
    max_w: u32,
) -> (u32, u32, Vec<u8>) {
    let step = ((src_w as usize) / (max_w as usize).max(1)).max(1);
    if step > 1 && (src_w as usize) >= step && (src_h as usize) >= step {
        let dst_w = (src_w as usize) / step;
        let dst_h = (src_h as usize) / step;
        let stride = (src_w as usize) * 4;
        let mut rgba = vec![255_u8; dst_w * dst_h * 4];
        for y in 0..dst_h {
            let src_row = &bgra[(y * step * stride)..((y * step + 1) * stride)];
            let dst_row = &mut rgba[(y * dst_w * 4)..((y + 1) * dst_w * 4)];
            for x in 0..dst_w {
                let sp = x * step * 4;
                let dp = x * 4;
                dst_row[dp] = src_row[sp + 2];
                dst_row[dp + 1] = src_row[sp + 1];
                dst_row[dp + 2] = src_row[sp];
                dst_row[dp + 3] = 255;
            }
        }
        (dst_w as u32, dst_h as u32, rgba)
    } else {
        let mut rgba = vec![255_u8; (src_w as usize) * (src_h as usize) * 4];
        for (src, dst) in bgra.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
            dst[0] = src[2];
            dst[1] = src[1];
            dst[2] = src[0];
            dst[3] = 255;
        }
        (src_w, src_h, rgba)
    }
}

pub fn extract_mp4_thumbnail_jpg(
    video_path: &Path,
    thumb_path: &Path,
    seek_secs: f32,
) -> Result<(), String> {
    let mut mf = SyncMfReader::open(video_path)?;
    if seek_secs > 0.05 {
        mf.seek(seek_secs);
    }
    let (w, h, rgba, _) = mf
        .read_video_frame_rgba(960)
        .ok()
        .flatten()
        .ok_or_else(|| "Failed to decode frame for thumbnail".to_owned())?;

    let img: ImageBuffer<Rgba<u8>, Vec<u8>> =
        ImageBuffer::from_raw(w, h, rgba).ok_or_else(|| "Invalid RGBA buffer".to_owned())?;
    let resized = image::imageops::resize(&img, 360, 202, FilterType::Triangle);
    let mut rgb = ImageBuffer::<Rgb<u8>, Vec<u8>>::new(resized.width(), resized.height());
    for (x, y, px) in resized.enumerate_pixels() {
        rgb.put_pixel(x, y, Rgb([px[0], px[1], px[2]]));
    }
    if let Some(parent) = thumb_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    rgb.save_with_format(thumb_path, image::ImageFormat::Jpeg)
        .map_err(|e| format!("save jpg: {e}"))
}

pub fn probe_first_av_sync(path: &Path) -> Result<(bool, bool, bool), String> {
    let mut mf = SyncMfReader::open(path)?;
    let has_audio = mf.has_audio;
    let has_video_frame = mf.read_video_frame_rgba(960).ok().flatten().is_some();
    let has_audio_sample = mf.read_audio_sample(1.0).ok().flatten().is_some();
    Ok((has_audio, has_video_frame, has_audio_sample))
}

unsafe fn configure_audio_pcm_stream(reader: &IMFSourceReader) -> bool {
    let Ok(pcm_mt) = MFCreateMediaType() else {
        return false;
    };
    let _ = pcm_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_AUDIO);
    let _ = pcm_mt.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM);
    let _ = pcm_mt.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16);
    let _ = pcm_mt.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48_000);
    let _ = pcm_mt.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2);
    let audio_idx = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
    reader.SetCurrentMediaType(audio_idx, None, &pcm_mt).is_ok()
}

pub fn probe_audio_stream_pcm(path: &Path) -> Result<Vec<u8>, String> {
    if !path.exists() {
        return Err("File does not exist".to_owned());
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).map_err(|e| format!("MFStartup: {e}"))?;
        let res = (|| {
            let wide = to_wide_null(&path.to_string_lossy());
            let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), None)
                .map_err(|e| format!("MFCreateSourceReaderFromURL: {e}"))?;
            if !configure_audio_pcm_stream(&reader) {
                return Err("Audio stream not decodable as 48kHz stereo PCM".to_owned());
            }
            let audio_idx = MF_SOURCE_READER_FIRST_AUDIO_STREAM.0 as u32;
            let mut out = Vec::new();
            for _ in 0..8 {
                let mut flags = 0_u32;
                let mut ts = 0_i64;
                let mut sample: Option<IMFSample> = None;
                reader
                    .ReadSample(
                        audio_idx,
                        0,
                        None,
                        Some(&mut flags),
                        Some(&mut ts),
                        Some(&mut sample),
                    )
                    .map_err(|e| format!("ReadSample audio: {e}"))?;
                if (flags & 0x02) != 0 {
                    break;
                }
                let Some(s) = sample else { break };
                let buf = s.ConvertToContiguousBuffer().map_err(|e| e.to_string())?;
                let mut ptr: *mut u8 = std::ptr::null_mut();
                let mut len = 0_u32;
                buf.Lock(&mut ptr, None, Some(&mut len))
                    .map_err(|e| e.to_string())?;
                if !ptr.is_null() && len > 0 {
                    out.extend_from_slice(std::slice::from_raw_parts(ptr, len as usize));
                }
                let _ = buf.Unlock();
            }
            if out.is_empty() {
                return Err("No audio sample decoded".to_owned());
            }
            Ok(out)
        })();
        let _ = MFShutdown();
        res
    }
}
