use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;

use windows::core::{GUID, PCWSTR};
use windows::Win32::Media::MediaFoundation::{
    eAVEncCommonRateControlMode_PeakConstrainedVBR, eAVEncH264VProfile_High, IMFAttributes,
    IMFMediaBuffer, IMFMediaType, IMFSample, IMFSinkWriter, IMFSourceReader, MFAudioFormat_AAC,
    MFAudioFormat_PCM, MFCreateAttributes, MFCreateMediaType, MFCreateMemoryBuffer, MFCreateSample,
    MFCreateSinkWriterFromURL, MFCreateSourceReaderFromURL, MFShutdown, MFStartup,
    MFTranscodeContainerType_MPEG4, MFVideoArea, MFVideoFormat_H264, MFVideoFormat_HEVC,
    MFVideoFormat_NV12, CODECAPI_AVEncCommonBufferSize, CODECAPI_AVEncCommonMaxBitRate,
    CODECAPI_AVEncCommonMeanBitRate, CODECAPI_AVEncCommonRateControlMode,
    CODECAPI_AVEncH264CABACEnable, CODECAPI_AVEncMPVDefaultBPictureCount,
    CODECAPI_AVEncMPVGOPSize, MFSTARTUP_NOSOCKET, MF_MT_AUDIO_AVG_BYTES_PER_SECOND,
    MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_BLOCK_ALIGNMENT, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_AVG_BITRATE, MF_MT_DEFAULT_STRIDE, MF_MT_FRAME_RATE,
    MF_MT_FRAME_SIZE, MF_MT_INTERLACE_MODE, MF_MT_MAJOR_TYPE, MF_MT_MINIMUM_DISPLAY_APERTURE,
    MF_MT_MPEG2_PROFILE, MF_MT_PIXEL_ASPECT_RATIO, MF_MT_SUBTYPE, MF_PD_DURATION,
    MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, MF_SINK_WRITER_DISABLE_THROTTLING,
    MF_SOURCE_READER_ALL_STREAMS, MF_SOURCE_READER_ANY_STREAM,
    MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, MF_SOURCE_READER_MEDIASOURCE,
    MF_TRANSCODE_CONTAINERTYPE, MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

use crate::config::AppConfig;
use crate::library::sanitize_filename;
use crate::win_ffi::to_wide_null;

pub const DISCORD_LIMIT_BYTES: u64 = 10_400_000;
const DISCORD_TARGET_BYTES: u64 = 10_050_000;

const MF_MEDIATYPE_VIDEO: GUID = GUID::from_u128(0x73646976_0000_0010_8000_00aa00389b71);
const MF_MEDIATYPE_AUDIO: GUID = GUID::from_u128(0x73647561_0000_0010_8000_00aa00389b71);

#[repr(C)]
struct RawI64PropVariant {
    vt: u16,
    w1: u16,
    w2: u16,
    w3: u16,
    val: i64,
    _pad: i64,
}

#[derive(Debug, Clone, Copy)]
struct EncodePlan {
    out_w: u32,
    out_h: u32,
    target_fps: u32,
    video_bps: u32,
    audio_bytes_per_sec: u32,
    prefer_hevc: bool,
}

fn pack_u32_pair(hi: u32, lo: u32) -> u64 {
    ((hi as u64) << 32) | (lo as u64)
}

fn unpack_u32_pair(v: u64) -> (u32, u32) {
    ((v >> 32) as u32, (v & 0xFFFF_FFFF) as u32)
}

fn compute_encode_plan(
    src_w: u32,
    src_h: u32,
    src_fps: u32,
    dur_secs: f32,
    has_audio: bool,
    target_bytes: u64,
) -> EncodePlan {
    let dur = dur_secs.max(0.5) as f64;
    let audio_bytes_per_sec = if !has_audio {
        0
    } else if dur <= 35.0 {
        16_000
    } else if dur <= 75.0 {
        12_000
    } else {
        8_000
    };
    let audio_bps = audio_bytes_per_sec * 8;
    let total_bps = ((target_bytes as f64 * 8.0) / dur) as u32;
    let video_bps = total_bps.saturating_sub(audio_bps).clamp(100_000, 12_000_000);
    let prefer_hevc = video_bps < 2_200_000;

    let (max_h, max_fps) = if video_bps >= 2_800_000 {
        (1080_u32, 60_u32)
    } else if video_bps >= 1_400_000 {
        (720_u32, 60_u32)
    } else if video_bps >= 420_000 {
        (720_u32, 30_u32)
    } else if video_bps >= 260_000 {
        (540_u32, 30_u32)
    } else {
        (480_u32, 24_u32)
    };

    let (out_w, out_h) = if src_h > max_h && src_h > 0 {
        let scale = max_h as f64 / src_h as f64;
        let w = (((src_w as f64 * scale).round() as u32) & !1).max(16);
        let h = (max_h & !1).max(16);
        (w, h)
    } else {
        ((src_w & !1).max(16), (src_h & !1).max(16))
    };

    let target_fps = src_fps.clamp(15, 240).min(max_fps).max(15);
    EncodePlan {
        out_w,
        out_h,
        target_fps,
        video_bps,
        audio_bytes_per_sec,
        prefer_hevc,
    }
}

pub fn discord_export_path(title: &str, clip_id: i64) -> PathBuf {
    let dir = AppConfig::app_data_dir().join("discord_exports");
    let _ = fs::create_dir_all(&dir);
    prune_old_discord_exports(&dir, 8);
    let safe = sanitize_filename(title);
    let stem = if safe.is_empty() {
        format!("clip_{clip_id}")
    } else {
        safe
    };
    dir.join(format!("{stem}_discord.mp4"))
}

fn prune_old_discord_exports(dir: &Path, keep: usize) {
    let Ok(rd) = fs::read_dir(dir) else {
        return;
    };
    let mut files: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter_map(|e| {
            let p = e.path();
            if p.extension().and_then(|s| s.to_str()) != Some("mp4") {
                return None;
            }
            let m = e.metadata().ok()?.modified().ok()?;
            Some((m, p))
        })
        .collect();
    if files.len() <= keep {
        return;
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, p) in files.into_iter().skip(keep) {
        let _ = fs::remove_file(p);
    }
}

pub fn encode_for_discord(
    src_path: &Path,
    out_path: &Path,
    trim_range: Option<(f32, f32)>,
    duration_hint_secs: f32,
    progress_pct: &Arc<AtomicU32>,
) -> Result<(PathBuf, u64), String> {
    encode_for_discord_cancellable(
        src_path,
        out_path,
        trim_range,
        duration_hint_secs,
        progress_pct,
        &AtomicBool::new(false),
    )
}

pub fn encode_for_discord_cancellable(
    src_path: &Path,
    out_path: &Path,
    trim_range: Option<(f32, f32)>,
    duration_hint_secs: f32,
    progress_pct: &Arc<AtomicU32>,
    cancel: &AtomicBool,
) -> Result<(PathBuf, u64), String> {
    if !src_path.exists() {
        return Err("Source clip file does not exist".to_owned());
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Cancelled".to_owned());
    }
    if let Some(parent) = out_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create export dir: {e}"))?;
    }
    progress_pct.store(1, Ordering::Release);

    let (src_w, src_h, src_fps, probed_dur, has_audio) =
        probe_source_video(src_path, duration_hint_secs)?;
    let full_dur = probed_dur.max(duration_hint_secs).max(0.5);
    let (start_s, end_s) = match trim_range {
        Some((s, e)) if e - s >= 0.3 => (s.clamp(0.0, full_dur), e.clamp(0.3, full_dur)),
        _ => (0.0, full_dur),
    };
    let seg_dur = (end_s - start_s).max(0.5);

    let mut plan = compute_encode_plan(
        src_w,
        src_h,
        src_fps,
        seg_dur,
        has_audio,
        DISCORD_TARGET_BYTES,
    );

    static TMP_SEQ: AtomicU64 = AtomicU64::new(1);
    let tmp_seq = TMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp_path = out_path.with_extension(format!("discord.{tmp_seq}.tmp.mp4"));
    run_with_fallbacks(
        src_path,
        &tmp_path,
        start_s,
        end_s,
        plan,
        has_audio,
        progress_pct,
        0,
        95,
        cancel,
    )?;

    if cancel.load(Ordering::Acquire) {
        let _ = fs::remove_file(&tmp_path);
        return Err("Cancelled".to_owned());
    }

    let mut actual_size = fs::metadata(&tmp_path)
        .map_err(|e| format!("read output size: {e}"))?
        .len();

    if actual_size > DISCORD_LIMIT_BYTES {
        let scaled_bps = ((plan.video_bps as u128 * 9_500_000) / (actual_size as u128))
            .clamp(80_000, plan.video_bps as u128) as u32;
        plan.video_bps = scaled_bps;
        if plan.video_bps < 400_000 && plan.out_h > 480 {
            let scale = 480.0 / (plan.out_h as f64);
            plan.out_w = (((plan.out_w as f64 * scale).round() as u32) & !1).max(16);
            plan.out_h = 480;
            plan.target_fps = plan.target_fps.min(24);
        }
        run_with_fallbacks(
            src_path,
            &tmp_path,
            start_s,
            end_s,
            plan,
            has_audio,
            progress_pct,
            50,
            99,
            cancel,
        )?;
        actual_size = fs::metadata(&tmp_path)
            .map_err(|e| format!("read retry output size: {e}"))?
            .len();
    }

    if cancel.load(Ordering::Acquire) {
        let _ = fs::remove_file(&tmp_path);
        return Err("Cancelled".to_owned());
    }
    if out_path.exists() {
        let _ = fs::remove_file(out_path);
    }
    fs::rename(&tmp_path, out_path).map_err(|e| format!("finalize export file: {e}"))?;
    progress_pct.store(100, Ordering::Release);
    Ok((out_path.to_path_buf(), actual_size))
}

fn probe_source_video(
    src_path: &Path,
    dur_hint: f32,
) -> Result<(u32, u32, u32, f32, bool), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).map_err(|e| format!("MFStartup: {e}"))?;
        let res = (|| -> Result<(u32, u32, u32, f32, bool), String> {
            let wide = to_wide_null(&src_path.to_string_lossy());
            let reader = MFCreateSourceReaderFromURL(PCWSTR(wide.as_ptr()), None)
                .map_err(|e| format!("open source clip: {e}"))?;
            let (vid_idx, aud_idx) = find_stream_indices(&reader);
            let vid_idx = vid_idx.ok_or_else(|| "Source clip has no video stream".to_owned())?;
            let mt = reader
                .GetNativeMediaType(vid_idx, 0)
                .or_else(|_| reader.GetCurrentMediaType(vid_idx))
                .map_err(|e| format!("read video media type: {e}"))?;
            let packed_sz = mt
                .GetUINT64(&MF_MT_FRAME_SIZE)
                .unwrap_or(pack_u32_pair(1920, 1080));
            let (coded_w, coded_h) = unpack_u32_pair(packed_sz);
            let (w, h) = read_display_dims(&mt).unwrap_or((coded_w, coded_h));
            let fps_packed = mt
                .GetUINT64(&MF_MT_FRAME_RATE)
                .unwrap_or(pack_u32_pair(60, 1));
            let (fnum, fden) = unpack_u32_pair(fps_packed);
            let fps = if fnum > 0 && fden > 0 {
                ((fnum as f64 / fden as f64).round() as u32).clamp(15, 240)
            } else {
                60
            };
            let dur_secs = read_reader_duration_secs(&reader).unwrap_or(dur_hint.max(0.5));
            Ok((w.max(16), h.max(16), fps, dur_secs, aud_idx.is_some()))
        })();
        let _ = MFShutdown();
        CoUninitialize();
        res
    }
}

unsafe fn find_stream_indices(reader: &IMFSourceReader) -> (Option<u32>, Option<u32>) {
    let mut vid = None;
    let mut aud = None;
    for idx in 0..8_u32 {
        let Ok(mt) = reader.GetNativeMediaType(idx, 0) else {
            continue;
        };
        if let Ok(major) = mt.GetMajorType() {
            if major == MF_MEDIATYPE_VIDEO && vid.is_none() {
                vid = Some(idx);
            } else if major == MF_MEDIATYPE_AUDIO && aud.is_none() {
                aud = Some(idx);
            }
        }
    }
    (vid, aud)
}

unsafe fn read_display_dims(mt: &IMFMediaType) -> Option<(u32, u32)> {
    let mut area = MFVideoArea::default();
    let bytes = std::slice::from_raw_parts_mut(
        (&mut area as *mut MFVideoArea) as *mut u8,
        std::mem::size_of::<MFVideoArea>(),
    );
    if mt.GetBlob(&MF_MT_MINIMUM_DISPLAY_APERTURE, bytes, None).is_ok()
        && area.Area.cx > 0
        && area.Area.cy > 0
    {
        return Some((area.Area.cx as u32, area.Area.cy as u32));
    }
    None
}

unsafe fn read_reader_duration_secs(reader: &IMFSourceReader) -> Option<f32> {
    let pv = reader
        .GetPresentationAttribute(MF_SOURCE_READER_MEDIASOURCE.0 as u32, &MF_PD_DURATION)
        .ok()?;
    let raw = &*(&pv as *const windows::core::PROPVARIANT as *const RawI64PropVariant);
    if raw.vt == 20 || raw.vt == 21 {
        let dur_100ns = raw.val;
        if dur_100ns > 0 {
            return Some((dur_100ns as f64 / 10_000_000.0) as f32);
        }
    }
    None
}

unsafe fn seek_reader_to(reader: &IMFSourceReader, secs: f32) {
    let pos_100ns = (secs.max(0.0) as f64 * 10_000_000.0) as i64;
    let pv = RawI64PropVariant {
        vt: 20,
        w1: 0,
        w2: 0,
        w3: 0,
        val: pos_100ns,
        _pad: 0,
    };
    let _ = reader.SetCurrentPosition(
        &GUID::zeroed(),
        &pv as *const RawI64PropVariant as *const windows::core::PROPVARIANT,
    );
}

fn run_with_fallbacks(
    src_path: &Path,
    tmp_path: &Path,
    start_s: f32,
    end_s: f32,
    plan: EncodePlan,
    has_audio: bool,
    progress_pct: &Arc<AtomicU32>,
    pct_min: u32,
    pct_max: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    let attempts: &[(bool, bool)] = if plan.prefer_hevc {
        &[(true, true), (true, false), (false, true), (false, false)]
    } else {
        &[(false, true), (false, false)]
    };
    let mut last_err = String::from("Transcode failed");
    for &(use_hevc, use_hw) in attempts {
        let _ = fs::remove_file(tmp_path);
        if cancel.load(Ordering::Acquire) {
            return Err("Cancelled".to_owned());
        }
        match transcode_pass(
            src_path,
            tmp_path,
            start_s,
            end_s,
            plan,
            has_audio,
            use_hevc,
            use_hw,
            progress_pct,
            pct_min,
            pct_max,
            cancel,
        ) {
            Ok(()) => return Ok(()),
            Err(e) => {
                if cancel.load(Ordering::Acquire) {
                    let _ = fs::remove_file(tmp_path);
                    return Err("Cancelled".to_owned());
                }
                last_err = e;
            }
        }
    }
    Err(last_err)
}

fn transcode_pass(
    src_path: &Path,
    out_path: &Path,
    start_secs: f32,
    end_secs: f32,
    plan: EncodePlan,
    want_audio: bool,
    use_hevc: bool,
    use_hardware_encoder: bool,
    progress_pct: &Arc<AtomicU32>,
    pct_min: u32,
    pct_max: u32,
    cancel: &AtomicBool,
) -> Result<(), String> {
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).map_err(|e| format!("MFStartup: {e}"))?;

        let result = (|| -> Result<(), String> {
            let mut r_attrs: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut r_attrs, 2).map_err(|e| format!("reader attrs: {e}"))?;
            let r_attrs = r_attrs.ok_or_else(|| "missing reader attrs".to_owned())?;
            let _ = r_attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
            let _ = r_attrs.SetUINT32(&MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, 1);

            let src_wide = to_wide_null(&src_path.to_string_lossy());
            let reader = MFCreateSourceReaderFromURL(PCWSTR(src_wide.as_ptr()), &r_attrs)
                .map_err(|e| format!("open reader: {e}"))?;

            let (vid_idx, aud_idx) = find_stream_indices(&reader);
            let vid_idx = vid_idx.ok_or_else(|| "no video stream".to_owned())?;
            let _ = reader.SetStreamSelection(MF_SOURCE_READER_ALL_STREAMS.0 as u32, false);
            reader
                .SetStreamSelection(vid_idx, true)
                .map_err(|e| format!("select video: {e}"))?;

            let reader_vid_mt = MFCreateMediaType().map_err(|e| format!("vid mt: {e}"))?;
            reader_vid_mt
                .SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_VIDEO)
                .map_err(|e| e.to_string())?;
            reader_vid_mt
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)
                .map_err(|e| e.to_string())?;
            reader
                .SetCurrentMediaType(vid_idx, None, &reader_vid_mt)
                .map_err(|e| format!("set reader NV12: {e}"))?;

            let cur_vid_mt = reader
                .GetCurrentMediaType(vid_idx)
                .map_err(|e| format!("get cur vid mt: {e}"))?;
            let (mut src_coded_w, mut src_coded_h) = unpack_u32_pair(
                cur_vid_mt
                    .GetUINT64(&MF_MT_FRAME_SIZE)
                    .unwrap_or(pack_u32_pair(plan.out_w, plan.out_h)),
            );
            let (mut src_vis_w, mut src_vis_h) =
                read_display_dims(&cur_vid_mt).unwrap_or((src_coded_w, src_coded_h));
            src_vis_w = src_vis_w.min(src_coded_w).max(2);
            src_vis_h = src_vis_h.min(src_coded_h).max(2);
            let mut src_stride = (cur_vid_mt
                .GetUINT32(&MF_MT_DEFAULT_STRIDE)
                .unwrap_or(src_coded_w) as i32)
                .unsigned_abs()
                .max(src_coded_w);

            let active_aud_idx = if want_audio {
                if let Some(a_idx) = aud_idx {
                    if reader.SetStreamSelection(a_idx, true).is_ok() {
                        let pcm_mt = MFCreateMediaType().map_err(|e| e.to_string())?;
                        pcm_mt
                            .SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_AUDIO)
                            .map_err(|e| e.to_string())?;
                        pcm_mt
                            .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)
                            .map_err(|e| e.to_string())?;
                        pcm_mt
                            .SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)
                            .map_err(|e| e.to_string())?;
                        pcm_mt
                            .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48_000)
                            .map_err(|e| e.to_string())?;
                        pcm_mt
                            .SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)
                            .map_err(|e| e.to_string())?;
                        pcm_mt
                            .SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)
                            .map_err(|e| e.to_string())?;
                        pcm_mt
                            .SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 192_000)
                            .map_err(|e| e.to_string())?;
                        if reader.SetCurrentMediaType(a_idx, None, &pcm_mt).is_ok() {
                            Some(a_idx)
                        } else {
                            let _ = reader.SetStreamSelection(a_idx, false);
                            None
                        }
                    } else {
                        None
                    }
                } else {
                    None
                }
            } else {
                None
            };

            let mut w_attrs: Option<IMFAttributes> = None;
            MFCreateAttributes(&mut w_attrs, 3).map_err(|e| format!("writer attrs: {e}"))?;
            let w_attrs = w_attrs.ok_or_else(|| "missing writer attrs".to_owned())?;
            if use_hardware_encoder {
                let _ = w_attrs.SetUINT32(&MF_READWRITE_ENABLE_HARDWARE_TRANSFORMS, 1);
            }
            let _ = w_attrs.SetUINT32(&MF_SINK_WRITER_DISABLE_THROTTLING, 1);
            let _ = w_attrs.SetGUID(&MF_TRANSCODE_CONTAINERTYPE, &MFTranscodeContainerType_MPEG4);

            let out_wide = to_wide_null(&out_path.to_string_lossy());
            let writer: IMFSinkWriter =
                MFCreateSinkWriterFromURL(PCWSTR(out_wide.as_ptr()), None, &w_attrs)
                    .map_err(|e| format!("create sink writer: {e}"))?;

            let vid_out_mt = MFCreateMediaType().map_err(|e| e.to_string())?;
            vid_out_mt
                .SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_VIDEO)
                .map_err(|e| e.to_string())?;
            vid_out_mt
                .SetGUID(
                    &MF_MT_SUBTYPE,
                    if use_hevc { &MFVideoFormat_HEVC } else { &MFVideoFormat_H264 },
                )
                .map_err(|e| e.to_string())?;
            if !use_hevc {
                vid_out_mt
                    .SetUINT32(&MF_MT_MPEG2_PROFILE, eAVEncH264VProfile_High.0 as u32)
                    .map_err(|e| e.to_string())?;
            }
            vid_out_mt
                .SetUINT32(&MF_MT_AVG_BITRATE, plan.video_bps)
                .map_err(|e| e.to_string())?;
            vid_out_mt
                .SetUINT32(&MF_MT_INTERLACE_MODE, 2)
                .map_err(|e| e.to_string())?;
            vid_out_mt
                .SetUINT64(&MF_MT_FRAME_SIZE, pack_u32_pair(plan.out_w, plan.out_h))
                .map_err(|e| e.to_string())?;
            vid_out_mt
                .SetUINT64(&MF_MT_FRAME_RATE, pack_u32_pair(plan.target_fps, 1))
                .map_err(|e| e.to_string())?;
            vid_out_mt
                .SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u32_pair(1, 1))
                .map_err(|e| e.to_string())?;
            let sink_vid_idx = writer
                .AddStream(&vid_out_mt)
                .map_err(|e| format!("add H264 stream: {e}"))?;

            let vid_in_mt = MFCreateMediaType().map_err(|e| e.to_string())?;
            vid_in_mt
                .SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_VIDEO)
                .map_err(|e| e.to_string())?;
            vid_in_mt
                .SetGUID(&MF_MT_SUBTYPE, &MFVideoFormat_NV12)
                .map_err(|e| e.to_string())?;
            vid_in_mt
                .SetUINT32(&MF_MT_INTERLACE_MODE, 2)
                .map_err(|e| e.to_string())?;
            vid_in_mt
                .SetUINT64(&MF_MT_FRAME_SIZE, pack_u32_pair(plan.out_w, plan.out_h))
                .map_err(|e| e.to_string())?;
            vid_in_mt
                .SetUINT64(&MF_MT_FRAME_RATE, pack_u32_pair(plan.target_fps, 1))
                .map_err(|e| e.to_string())?;
            vid_in_mt
                .SetUINT64(&MF_MT_PIXEL_ASPECT_RATIO, pack_u32_pair(1, 1))
                .map_err(|e| e.to_string())?;
            vid_in_mt
                .SetUINT32(&MF_MT_DEFAULT_STRIDE, plan.out_w)
                .map_err(|e| e.to_string())?;

            let max_bps = (plan.video_bps as u64 * 2).min(14_000_000) as u32;
            let buf_sz = (plan.video_bps / 2).max(200_000);
            let gop = (plan.target_fps * 5).max(120);

            let mut enc_attrs_opt: Option<IMFAttributes> = None;
            if MFCreateAttributes(&mut enc_attrs_opt, 7).is_ok() {
                if let Some(ea) = &enc_attrs_opt {
                    let _ = ea.SetUINT32(
                        &CODECAPI_AVEncCommonRateControlMode,
                        eAVEncCommonRateControlMode_PeakConstrainedVBR.0 as u32,
                    );
                    let _ = ea.SetUINT32(&CODECAPI_AVEncCommonMeanBitRate, plan.video_bps);
                    let _ = ea.SetUINT32(&CODECAPI_AVEncCommonMaxBitRate, max_bps);
                    let _ = ea.SetUINT32(&CODECAPI_AVEncCommonBufferSize, buf_sz);
                    if !use_hevc {
                        let _ = ea.SetUINT32(&CODECAPI_AVEncH264CABACEnable, 1);
                    }
                    let _ = ea.SetUINT32(&CODECAPI_AVEncMPVDefaultBPictureCount, 0);
                    let _ = ea.SetUINT32(&CODECAPI_AVEncMPVGOPSize, gop);
                }
            }

            let set_res = match &enc_attrs_opt {
                Some(ea) => writer
                    .SetInputMediaType(sink_vid_idx, &vid_in_mt, ea)
                    .or_else(|_| writer.SetInputMediaType(sink_vid_idx, &vid_in_mt, None)),
                None => writer.SetInputMediaType(sink_vid_idx, &vid_in_mt, None),
            };
            set_res.map_err(|e| format!("set H264 input NV12: {e}"))?;

            let sink_aud_idx = if active_aud_idx.is_some() && plan.audio_bytes_per_sec > 0 {
                let aud_out_mt = MFCreateMediaType().map_err(|e| e.to_string())?;
                aud_out_mt
                    .SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_AUDIO)
                    .map_err(|e| e.to_string())?;
                aud_out_mt
                    .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC)
                    .map_err(|e| e.to_string())?;
                aud_out_mt
                    .SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)
                    .map_err(|e| e.to_string())?;
                aud_out_mt
                    .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48_000)
                    .map_err(|e| e.to_string())?;
                aud_out_mt
                    .SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)
                    .map_err(|e| e.to_string())?;
                aud_out_mt
                    .SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, plan.audio_bytes_per_sec)
                    .map_err(|e| e.to_string())?;
                let s_idx = writer
                    .AddStream(&aud_out_mt)
                    .map_err(|e| format!("add AAC stream: {e}"))?;

                let aud_in_mt = MFCreateMediaType().map_err(|e| e.to_string())?;
                aud_in_mt
                    .SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_AUDIO)
                    .map_err(|e| e.to_string())?;
                aud_in_mt
                    .SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM)
                    .map_err(|e| e.to_string())?;
                aud_in_mt
                    .SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16)
                    .map_err(|e| e.to_string())?;
                aud_in_mt
                    .SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48_000)
                    .map_err(|e| e.to_string())?;
                aud_in_mt
                    .SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2)
                    .map_err(|e| e.to_string())?;
                aud_in_mt
                    .SetUINT32(&MF_MT_AUDIO_BLOCK_ALIGNMENT, 4)
                    .map_err(|e| e.to_string())?;
                aud_in_mt
                    .SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 192_000)
                    .map_err(|e| e.to_string())?;
                writer
                    .SetInputMediaType(s_idx, &aud_in_mt, None)
                    .map_err(|e| format!("set AAC input PCM: {e}"))?;
                Some(s_idx)
            } else {
                None
            };

            writer
                .BeginWriting()
                .map_err(|e| format!("BeginWriting: {e}"))?;

            if start_secs > 0.05 {
                seek_reader_to(&reader, start_secs);
            }

            let start_100ns = (start_secs.max(0.0) as f64 * 10_000_000.0) as i64;
            let end_100ns = (end_secs.max(start_secs + 0.1) as f64 * 10_000_000.0) as i64;
            let seg_span_100ns = (end_100ns - start_100ns).max(1_000_000);
            let frame_dur_100ns = (10_000_000_i64) / (plan.target_fps.max(1) as i64);
            let out_nv12_bytes = (plan.out_w as usize) * (plan.out_h as usize) * 3 / 2;

            let mut base_100ns: Option<i64> = None;
            let mut next_vid_pts: i64 = 0;
            let mut vid_done = false;
            let mut aud_done = sink_aud_idx.is_none();
            let mut written_vid_frames = 0_u64;
            let mut last_pct = pct_min;

            while !(vid_done && aud_done) {
                if cancel.load(Ordering::Acquire) {
                    return Err("Cancelled".to_owned());
                }
                let mut actual_stream = 0_u32;
                let mut flags = 0_u32;
                let mut ts_100ns = 0_i64;
                let mut sample_opt: Option<IMFSample> = None;
                reader
                    .ReadSample(
                        MF_SOURCE_READER_ANY_STREAM.0 as u32,
                        0,
                        Some(&mut actual_stream),
                        Some(&mut flags),
                        Some(&mut ts_100ns),
                        Some(&mut sample_opt),
                    )
                    .map_err(|e| format!("ReadSample: {e}"))?;

                if (flags & 0x10) != 0 && actual_stream == vid_idx {
                    if let Ok(mt) = reader.GetCurrentMediaType(vid_idx) {
                        let (cw, ch) = unpack_u32_pair(
                            mt.GetUINT64(&MF_MT_FRAME_SIZE)
                                .unwrap_or(pack_u32_pair(src_coded_w, src_coded_h)),
                        );
                        src_coded_w = cw.max(2);
                        src_coded_h = ch.max(2);
                        let (vw, vh) = read_display_dims(&mt).unwrap_or((src_coded_w, src_coded_h));
                        src_vis_w = vw.min(src_coded_w).max(2);
                        src_vis_h = vh.min(src_coded_h).max(2);
                        src_stride = (mt.GetUINT32(&MF_MT_DEFAULT_STRIDE).unwrap_or(src_coded_w) as i32)
                            .unsigned_abs()
                            .max(src_coded_w);
                    }
                }

                if (flags & 0x02) != 0 {
                    if actual_stream == vid_idx {
                        vid_done = true;
                    } else if active_aud_idx == Some(actual_stream) {
                        aud_done = true;
                    } else {
                        break;
                    }
                    continue;
                }

                let Some(sample) = sample_opt else {
                    continue;
                };

                if ts_100ns < start_100ns {
                    continue;
                }
                if ts_100ns > end_100ns {
                    if actual_stream == vid_idx {
                        vid_done = true;
                    } else if active_aud_idx == Some(actual_stream) {
                        aud_done = true;
                    }
                    continue;
                }

                let base = *base_100ns.get_or_insert(ts_100ns);
                let rel_ts = (ts_100ns - base).max(0);

                if actual_stream == vid_idx && !vid_done {
                    if rel_ts + (frame_dur_100ns / 4) < next_vid_pts {
                        continue;
                    }
                    let out_ts = rel_ts.max(next_vid_pts);
                    let next_step = (out_ts / frame_dur_100ns) + 1;
                    next_vid_pts = next_step * frame_dur_100ns;

                    let buf: IMFMediaBuffer = sample
                        .ConvertToContiguousBuffer()
                        .map_err(|e| format!("vid buffer: {e}"))?;
                    let mut ptr: *mut u8 = std::ptr::null_mut();
                    let mut cur_len = 0_u32;
                    buf.Lock(&mut ptr, None, Some(&mut cur_len))
                        .map_err(|e| format!("lock vid: {e}"))?;

                    let write_res = if !ptr.is_null() && cur_len > 0 {
                        let raw = std::slice::from_raw_parts(ptr, cur_len as usize);
                        let exact_match = src_vis_w == plan.out_w
                            && src_vis_h == plan.out_h
                            && src_stride == plan.out_w
                            && raw.len() == out_nv12_bytes;

                        if exact_match {
                            let _ = buf.Unlock();
                            let _ = sample.SetSampleTime(out_ts);
                            let _ = sample.SetSampleDuration(frame_dur_100ns);
                            writer.WriteSample(sink_vid_idx, &sample)
                        } else {
                            let out_sample = MFCreateSample().map_err(|e| e.to_string())?;
                            let out_buf = MFCreateMemoryBuffer(out_nv12_bytes as u32)
                                .map_err(|e| e.to_string())?;
                            let mut dst_ptr: *mut u8 = std::ptr::null_mut();
                            out_buf
                                .Lock(&mut dst_ptr, None, None)
                                .map_err(|e| e.to_string())?;
                            if !dst_ptr.is_null() {
                                let dst = std::slice::from_raw_parts_mut(dst_ptr, out_nv12_bytes);
                                scale_nv12_frame(
                                    raw,
                                    src_stride as usize,
                                    src_coded_h as usize,
                                    src_vis_w as usize,
                                    src_vis_h as usize,
                                    plan.out_w as usize,
                                    plan.out_h as usize,
                                    dst,
                                );
                            }
                            let _ = out_buf.Unlock();
                            let _ = out_buf.SetCurrentLength(out_nv12_bytes as u32);
                            let _ = buf.Unlock();
                            let _ = out_sample.AddBuffer(&out_buf);
                            let _ = out_sample.SetSampleTime(out_ts);
                            let _ = out_sample.SetSampleDuration(frame_dur_100ns);
                            writer.WriteSample(sink_vid_idx, &out_sample)
                        }
                    } else {
                        let _ = buf.Unlock();
                        Ok(())
                    };

                    write_res.map_err(|e| format!("write video sample: {e}"))?;
                    written_vid_frames += 1;

                    let frac = (rel_ts as f64 / seg_span_100ns as f64).clamp(0.0, 1.0);
                    let pct = pct_min + ((pct_max - pct_min) as f64 * frac) as u32;
                    if pct > last_pct {
                        last_pct = pct;
                        progress_pct.store(pct, Ordering::Release);
                    }
                } else if active_aud_idx == Some(actual_stream) && !aud_done {
                    if let Some(s_aud) = sink_aud_idx {
                        let _ = sample.SetSampleTime(rel_ts);
                        writer
                            .WriteSample(s_aud, &sample)
                            .map_err(|e| format!("write audio sample: {e}"))?;
                    }
                }
            }

            if written_vid_frames == 0 {
                return Err("No video frames decoded from clip".to_owned());
            }

            writer.Finalize().map_err(|e| format!("Finalize MP4: {e}"))?;
            Ok(())
        })();

        let _ = MFShutdown();
        CoUninitialize();
        result
    }
}

fn scale_nv12_frame(
    src: &[u8],
    src_stride: usize,
    src_coded_h: usize,
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
    dst: &mut [u8],
) {
    let stride = src_stride.max(src_w).max(1);
    let plane_h = if src_coded_h >= src_h && stride * src_coded_h * 3 / 2 <= src.len() {
        src_coded_h
    } else {
        ((src.len() * 2) / (3 * stride)).max(src_h)
    };
    let src_uv_base = stride * plane_h;
    let dst_uv_base = dst_w * dst_h;
    let src_len = src.len();

    if src_w == dst_w && src_h == dst_h {
        for y in 0..dst_h {
            let s_off = y * stride;
            let d_off = y * dst_w;
            if s_off + dst_w <= src_len {
                dst[d_off..d_off + dst_w].copy_from_slice(&src[s_off..s_off + dst_w]);
            }
        }
        let uv_h = dst_h / 2;
        for y in 0..uv_h {
            let s_off = src_uv_base + y * stride;
            let d_off = dst_uv_base + y * dst_w;
            if s_off + dst_w <= src_len {
                dst[d_off..d_off + dst_w].copy_from_slice(&src[s_off..s_off + dst_w]);
            }
        }
        return;
    }

    for dy in 0..dst_h {
        let sy0 = (dy * src_h) / dst_h;
        let sy1 = (((dy + 1) * src_h) / dst_h).max(sy0 + 1).min(src_h);
        let d_row = dy * dst_w;
        if (sy1 - 1) * stride + src_w > src_len {
            dst[d_row..d_row + dst_w].fill(16);
            continue;
        }
        for dx in 0..dst_w {
            let sx0 = (dx * src_w) / dst_w;
            let sx1 = (((dx + 1) * src_w) / dst_w).max(sx0 + 1).min(src_w);
            let mut sum = 0_u32;
            let mut cnt = 0_u32;
            for sy in sy0..sy1 {
                let row = sy * stride;
                for sx in sx0..sx1 {
                    sum += src[row + sx] as u32;
                    cnt += 1;
                }
            }
            dst[d_row + dx] = ((sum + (cnt >> 1)) / cnt) as u8;
        }
    }

    let dst_uv_h = dst_h / 2;
    let src_uv_h = (src_h / 2).max(1);
    let dst_uv_pairs = dst_w / 2;
    let src_uv_pairs = (src_w / 2).max(1);
    for dy in 0..dst_uv_h {
        let sy0 = (dy * src_uv_h) / dst_uv_h;
        let sy1 = (((dy + 1) * src_uv_h) / dst_uv_h)
            .max(sy0 + 1)
            .min(src_uv_h);
        let d_row = dst_uv_base + dy * dst_w;
        if src_uv_base + (sy1 - 1) * stride + src_uv_pairs * 2 > src_len {
            dst[d_row..d_row + dst_w].fill(128);
            continue;
        }
        for px in 0..dst_uv_pairs {
            let sx0 = (px * src_uv_pairs) / dst_uv_pairs;
            let sx1 = (((px + 1) * src_uv_pairs) / dst_uv_pairs)
                .max(sx0 + 1)
                .min(src_uv_pairs);
            let mut sum_u = 0_u32;
            let mut sum_v = 0_u32;
            let mut cnt = 0_u32;
            for sy in sy0..sy1 {
                let row = src_uv_base + sy * stride;
                for sx in sx0..sx1 {
                    sum_u += src[row + sx * 2] as u32;
                    sum_v += src[row + sx * 2 + 1] as u32;
                    cnt += 1;
                }
            }
            let half = cnt >> 1;
            dst[d_row + px * 2] = ((sum_u + half) / cnt) as u8;
            dst[d_row + px * 2 + 1] = ((sum_v + half) / cnt) as u8;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find_subslice(hay: &[u8], needle: &[u8]) -> Option<usize> {
        hay.windows(needle.len()).position(|w| w == needle)
    }

    fn inspect_h264_avcc_and_stss(mp4_path: &Path) -> (u8, u32) {
        let bytes = fs::read(mp4_path).unwrap();
        let moov = bytes
            .windows(4)
            .rposition(|w| w == b"moov")
            .map(|i| &bytes[i..])
            .unwrap_or(&bytes);
        let profile = find_subslice(moov, b"avcC")
            .map(|i| moov[i + 5])
            .unwrap_or(0);
        let kf_count = find_subslice(moov, b"stss")
            .map(|i| u32::from_be_bytes([moov[i + 8], moov[i + 9], moov[i + 10], moov[i + 11]]))
            .unwrap_or(0);
        (profile, kf_count)
    }

    #[test]
    fn discord_plan_stays_under_10mb_across_durations() {
        for dur in [5.0_f32, 20.0, 30.0, 60.0, 120.0, 300.0] {
            let p = compute_encode_plan(1920, 1080, 120, dur, true, DISCORD_TARGET_BYTES);
            let total_bps = (p.video_bps as u64) + (p.audio_bytes_per_sec as u64) * 8;
            let est_bytes = (total_bps as f64 * dur as f64) / 8.0;
            assert!(
                est_bytes <= DISCORD_LIMIT_BYTES as f64,
                "dur={dur}s est_bytes={est_bytes} exceeded limit"
            );
            assert_eq!(p.out_w % 2, 0);
            assert_eq!(p.out_h % 2, 0);
        }
    }

    #[test]
    fn encodes_real_mp4_strictly_under_10mb_for_discord() {
        let desktop_clip =
            Path::new(r"C:\Users\Administrator\Videos\Rewind\Desktop\Desktop_1795372441.mp4");
        let long_clip =
            Path::new(r"C:\Users\Administrator\Videos\Rewind\ROB RUN\ROB RUN_1795373718.mp4");
        let dir = std::env::temp_dir().join("rewind_discord_enc_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();

        if desktop_clip.exists() {
            let out = dir.join("out_desktop_discord.mp4");
            let prog = Arc::new(AtomicU32::new(0));
            let (path, bytes) =
                encode_for_discord(desktop_clip, &out, None, 3.0, &prog).unwrap();
            assert!(path.exists());
            assert!(
                bytes > 10_000 && bytes <= DISCORD_LIMIT_BYTES,
                "desktop bytes={bytes}"
            );
            assert_eq!(prog.load(Ordering::Acquire), 100);
            let (profile, _) = inspect_h264_avcc_and_stss(&path);
            assert_eq!(profile, 100, "expected High Profile (100), got {profile}");
            let meta = crate::remuxer::probe_mp4_metadata(&path).unwrap();
            assert!(meta.duration_secs >= 1.5);
            assert!(meta.width >= 320 && meta.height >= 240);
            let pcm = crate::player::probe_audio_stream_pcm(&path).unwrap();
            assert!(!pcm.is_empty());
        }

        if long_clip.exists() {
            let out_trim = dir.join("out_rob_trim_discord.mp4");
            let prog = Arc::new(AtomicU32::new(0));
            let (path, bytes) =
                encode_for_discord(long_clip, &out_trim, Some((5.0, 25.0)), 119.0, &prog).unwrap();
            assert!(path.exists());
            assert!(
                bytes > 100_000 && bytes <= DISCORD_LIMIT_BYTES,
                "trimmed rob bytes={bytes}"
            );
            assert_eq!(prog.load(Ordering::Acquire), 100);
            let (profile, kf_count) = inspect_h264_avcc_and_stss(&path);
            assert!(profile == 100 || find_subslice(&fs::read(&path).unwrap(), b"hvcC").is_some(), "profile={profile}");
            assert!(kf_count <= 8, "expected >=5s GOP (<=8 kfs in 20s), got {kf_count}");
            let meta = crate::remuxer::probe_mp4_metadata(&path).unwrap();
            assert!(
                meta.duration_secs >= 16.0 && meta.duration_secs <= 24.0,
                "trimmed dur={:.2}",
                meta.duration_secs
            );
            let pcm = crate::player::probe_audio_stream_pcm(&path).unwrap();
            assert!(!pcm.is_empty());
        }

        let _ = fs::remove_dir_all(&dir);
    }
}
