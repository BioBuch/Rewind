use std::fs::{self, File};
use std::io::{BufWriter, Seek, SeekFrom, Write};
use std::mem::ManuallyDrop;
use std::path::Path;

use windows::core::GUID;
use windows::Win32::Media::MediaFoundation::{
    IMFActivate, IMFTransform, MFCreateMediaType, MFCreateMemoryBuffer,
    MFCreateSample, MFShutdown, MFStartup, MFTEnumEx, MFAudioFormat_AAC, MFAudioFormat_PCM,
    MFT_CATEGORY_AUDIO_ENCODER, MFT_ENUM_FLAG_LOCALMFT, MFT_ENUM_FLAG_SORTANDFILTER,
    MFT_ENUM_FLAG_SYNCMFT, MFT_MESSAGE_COMMAND_DRAIN, MFT_MESSAGE_NOTIFY_BEGIN_STREAMING,
    MFT_MESSAGE_NOTIFY_START_OF_STREAM, MFT_OUTPUT_DATA_BUFFER, MFT_REGISTER_TYPE_INFO,
    MFSTARTUP_NOSOCKET, MF_E_TRANSFORM_NEED_MORE_INPUT, MF_MT_AAC_PAYLOAD_TYPE,
    MF_MT_AUDIO_AVG_BYTES_PER_SECOND, MF_MT_AUDIO_BITS_PER_SAMPLE, MF_MT_AUDIO_NUM_CHANNELS,
    MF_MT_AUDIO_SAMPLES_PER_SECOND, MF_MT_MAJOR_TYPE, MF_MT_SUBTYPE, MF_MT_USER_DATA, MF_VERSION,
};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_MULTITHREADED};

use crate::config::{ContainerFormat, VideoCodec};
use crate::ring_buffer::{EncodedPacket, PacketTrack};

const MF_MEDIATYPE_AUDIO: GUID = GUID::from_u128(0x73647561_0000_0010_8000_00aa00389b71);

#[derive(Debug, Clone)]
pub struct Mp4Metadata {
    pub duration_secs: f32,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub codec: String,
    pub keyframe_times_secs: Vec<f32>,
    pub track_count: usize,
}

#[derive(Debug, Clone)]
struct MuxedSample {
    offset: u64,
    size: u32,
    duration_ticks: u32,
    is_sync: bool,
}

pub fn mux_packets_to_file(
    path: &Path,
    packets: &[EncodedPacket],
    width: u32,
    height: u32,
    fps: u32,
    codec: VideoCodec,
    container: ContainerFormat,
    separate_audio_tracks: bool,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create_dir_all failed: {e}"))?;
    }
    match container {
        ContainerFormat::Mp4 => mux_packets_to_mp4(
            path,
            packets,
            width,
            height,
            fps,
            codec,
            separate_audio_tracks,
        ),
        ContainerFormat::Mkv => mux_packets_to_mkv(
            path,
            packets,
            width,
            height,
            fps,
            codec,
            separate_audio_tracks,
        ),
    }
}

pub fn mux_packets_to_mp4(
    path: &Path,
    packets: &[EncodedPacket],
    width: u32,
    height: u32,
    fps: u32,
    codec: VideoCodec,
    separate_audio_tracks: bool,
) -> Result<(), String> {
    let video_pkts: Vec<&EncodedPacket> = packets
        .iter()
        .filter(|p| p.track == PacketTrack::Video)
        .collect();
    if video_pkts.is_empty() {
        return Err("No video packets in snapshot".to_owned());
    }
    let raw_game_pkts: Vec<&EncodedPacket> = packets
        .iter()
        .filter(|p| p.track == PacketTrack::GameAudio)
        .collect();
    let raw_mic_pkts: Vec<&EncodedPacket> = packets
        .iter()
        .filter(|p| p.track == PacketTrack::MicAudio)
        .collect();
    let (v_start, total_frames) = video_timeline_span(&video_pkts);
    let mixed_pcm = mix_game_and_mic_pcm(&video_pkts, &raw_game_pkts, &raw_mic_pkts);

    let mut sps_list: Vec<Vec<u8>> = Vec::new();
    let mut pps_list: Vec<Vec<u8>> = Vec::new();
    let mut vps_list: Vec<Vec<u8>> = Vec::new();
    let mut converted_video: Vec<(Vec<u8>, u32, bool)> = Vec::with_capacity(video_pkts.len());

    let video_timescale = 90_000_u32;
    let _default_frame_ticks = (video_timescale / fps.max(1)).max(1);
    let mut prev_vid_ticks: u64 = 0;

    for (idx, vp) in video_pkts.iter().enumerate() {
        let next_end_rel_100ns = if idx + 1 < video_pkts.len() {
            (video_pkts[idx + 1].pts_100ns - v_start).max(0) as u64
        } else {
            ((vp.pts_100ns - v_start).max(0) + vp.duration_100ns.max(50_000)) as u64
        };
        let target_ticks =
            ((next_end_rel_100ns as u128 * video_timescale as u128) / 10_000_000) as u64;
        let ticks = target_ticks
            .saturating_sub(prev_vid_ticks)
            .max(1)
            .min(video_timescale as u64 * 2) as u32;
        prev_vid_ticks += ticks as u64;

        let mut mp4_sample = Vec::with_capacity(vp.data.len() + 16);
        let mut is_sync = vp.is_idr_keyframe;

        if codec == VideoCodec::Av1 {
            mp4_sample.extend_from_slice(&vp.data);
        } else {
            let nals = split_annex_b_nals(&vp.data);
            if nals.is_empty() {
                mp4_sample.extend_from_slice(&(vp.data.len() as u32).to_be_bytes());
                mp4_sample.extend_from_slice(&vp.data);
            } else {
                for nal in nals {
                    if nal.is_empty() {
                        continue;
                    }
                    match codec {
                        VideoCodec::H264 => {
                            let nal_type = nal[0] & 0x1F;
                            if nal_type == 7 && sps_list.is_empty() {
                                sps_list.push(nal.to_vec());
                            } else if nal_type == 8 && pps_list.is_empty() {
                                pps_list.push(nal.to_vec());
                            } else {
                                if nal_type == 5 {
                                    is_sync = true;
                                }
                                mp4_sample.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                                mp4_sample.extend_from_slice(nal);
                            }
                        }
                        VideoCodec::Hevc => {
                            let nal_type = (nal[0] >> 1) & 0x3F;
                            if nal_type == 32 && vps_list.is_empty() {
                                vps_list.push(nal.to_vec());
                            } else if nal_type == 33 && sps_list.is_empty() {
                                sps_list.push(nal.to_vec());
                            } else if nal_type == 34 && pps_list.is_empty() {
                                pps_list.push(nal.to_vec());
                            } else {
                                if (19..=21).contains(&nal_type) {
                                    is_sync = true;
                                }
                                mp4_sample.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                                mp4_sample.extend_from_slice(nal);
                            }
                        }
                        VideoCodec::Av1 => {}
                    }
                }
            }
        }

        if !mp4_sample.is_empty() {
            converted_video.push((mp4_sample, ticks, is_sync));
        }
    }

    if converted_video.is_empty() {
        return Err("Video packets produced zero MP4 samples".to_owned());
    }

    let tmp_path = path.with_extension("mp4.tmp");
    let mut f = BufWriter::new(File::create(&tmp_path).map_err(|e| format!("create tmp mp4: {e}"))?);

    let mut ftyp = Vec::new();
    ftyp.extend_from_slice(b"isom");
    ftyp.extend_from_slice(&512_u32.to_be_bytes());
    ftyp.extend_from_slice(b"isomiso2avc1mp41");
    write_box(&mut f, *b"ftyp", &ftyp).map_err(|e| format!("write ftyp: {e}"))?;

    let mdat_pos = f.stream_position().map_err(|e| e.to_string())?;
    f.write_all(&0_u32.to_be_bytes()).map_err(|e| e.to_string())?;
    f.write_all(b"mdat").map_err(|e| e.to_string())?;

    let mut vid_samples = Vec::with_capacity(converted_video.len());
    let mut vid_total_ticks: u64 = 0;
    for (data, ticks, is_sync) in &converted_video {
        let off = f.stream_position().map_err(|e| e.to_string())?;
        f.write_all(data).map_err(|e| e.to_string())?;
        vid_samples.push(MuxedSample {
            offset: off,
            size: data.len() as u32,
            duration_ticks: *ticks,
            is_sync: *is_sync,
        });
        vid_total_ticks += *ticks as u64;
    }

    let audio_timescale = 48_000_u32;
    let (game_samples, game_total_ticks, game_stsd) = write_encoded_audio_pcm(&mut f, &mixed_pcm)?;
    let (mic_samples, mic_total_ticks, mic_stsd) = if separate_audio_tracks && !raw_mic_pkts.is_empty() {
        write_encoded_audio_samples(&mut f, &raw_mic_pkts, v_start, total_frames)?
    } else {
        (Vec::new(), 0, Vec::new())
    };

    let mdat_end = f.stream_position().map_err(|e| e.to_string())?;
    let mdat_size = (mdat_end - mdat_pos) as u32;
    f.flush().map_err(|e| e.to_string())?;
    {
        let inner = f.get_mut();
        inner.seek(SeekFrom::Start(mdat_pos)).map_err(|e| e.to_string())?;
        inner.write_all(&mdat_size.to_be_bytes()).map_err(|e| e.to_string())?;
        inner.seek(SeekFrom::Start(mdat_end)).map_err(|e| e.to_string())?;
    }

    let movie_timescale = 1000_u32;
    let movie_dur_ms = ((vid_total_ticks * movie_timescale as u64) / video_timescale as u64).max(1) as u32;

    let mut moov_body = Vec::new();
    let next_track_id = 2
        + if game_samples.is_empty() { 0 } else { 1 }
        + if mic_samples.is_empty() { 0 } else { 1 };
    moov_body.extend_from_slice(&build_mvhd(movie_timescale, movie_dur_ms, next_track_id));

    let v_stsd = build_video_stsd(codec, width, height, &vps_list, &sps_list, &pps_list);
    moov_body.extend_from_slice(&build_video_trak(
        1,
        movie_dur_ms,
        video_timescale,
        vid_total_ticks as u32,
        width,
        height,
        &v_stsd,
        &vid_samples,
    ));

    let mut tid = 2_u32;
    if !game_samples.is_empty() {
        let dur_ms = ((game_total_ticks * movie_timescale as u64) / audio_timescale as u64).max(1) as u32;
        moov_body.extend_from_slice(&build_audio_trak(
            tid,
            dur_ms,
            audio_timescale,
            game_total_ticks as u32,
            &game_stsd,
            &game_samples,
        ));
        tid += 1;
    }
    if !mic_samples.is_empty() {
        let dur_ms = ((mic_total_ticks * movie_timescale as u64) / audio_timescale as u64).max(1) as u32;
        moov_body.extend_from_slice(&build_audio_trak(
            tid,
            dur_ms,
            audio_timescale,
            mic_total_ticks as u32,
            &mic_stsd,
            &mic_samples,
        ));
    }

    write_box(&mut f, *b"moov", &moov_body).map_err(|e| format!("write moov: {e}"))?;
    f.flush().map_err(|e| e.to_string())?;
    drop(f);

    if path.exists() {
        let _ = fs::remove_file(path);
    }
    fs::rename(&tmp_path, path).map_err(|e| format!("rename mp4: {e}"))?;
    Ok(())
}

fn video_timeline_span(video_pkts: &[&EncodedPacket]) -> (i64, usize) {
    let v_start = video_pkts.first().map(|p| p.pts_100ns).unwrap_or(0);
    let v_end = video_pkts
        .last()
        .map(|p| p.pts_100ns + p.duration_100ns)
        .unwrap_or(v_start);
    let span_100ns = (v_end - v_start).max(100_000);
    let total_frames = (((span_100ns as i128 * 48_000) / 10_000_000) as usize)
        .clamp(480, 48_000 * 360);
    (v_start, total_frames)
}

fn place_track_on_timeline(pkts: &[&EncodedPacket], v_start: i64, total_frames: usize) -> Vec<u8> {
    let mut buf = vec![0_u8; total_frames * 4];
    let mut cursor_frame = 0_usize;
    let mut initialized = false;
    for p in pkts {
        if p.pts_100ns + p.duration_100ns <= v_start {
            continue;
        }
        let raw_frames = p.data.len() / 4;
        if raw_frames == 0 {
            continue;
        }
        let skip_frames = if p.pts_100ns < v_start {
            ((((v_start - p.pts_100ns) as i128) * 48_000) / 10_000_000) as usize
        } else {
            0
        };
        if skip_frames >= raw_frames {
            continue;
        }
        let pkt_frames = raw_frames - skip_frames;
        let rel_100ns = (p.pts_100ns - v_start).max(0);
        let target_frame = ((rel_100ns as i128 * 48_000) / 10_000_000) as usize;
        if !initialized || (target_frame as isize - cursor_frame as isize).abs() > 2400 {
            cursor_frame = target_frame;
            initialized = true;
        }
        if cursor_frame >= total_frames {
            break;
        }
        let copy_frames = pkt_frames.min(total_frames - cursor_frame);
        let src_start = skip_frames * 4;
        let dst_start = cursor_frame * 4;
        let copy_bytes = copy_frames * 4;
        buf[dst_start..dst_start + copy_bytes].copy_from_slice(&p.data[src_start..src_start + copy_bytes]);
        cursor_frame += copy_frames;
    }
    buf
}

fn mix_game_and_mic_pcm(
    video_pkts: &[&EncodedPacket],
    game_pkts: &[&EncodedPacket],
    mic_pkts: &[&EncodedPacket],
) -> Vec<u8> {
    if game_pkts.is_empty() && mic_pkts.is_empty() {
        return Vec::new();
    }
    let (v_start, total_frames) = video_timeline_span(video_pkts);
    let game_buf = place_track_on_timeline(game_pkts, v_start, total_frames);
    if mic_pkts.is_empty() {
        return game_buf;
    }
    let mic_buf = place_track_on_timeline(mic_pkts, v_start, total_frames);
    if game_pkts.is_empty() {
        return mic_buf;
    }
    let len = total_frames * 4;
    let mut mixed = vec![0_u8; len];
    for pos in (0..len).step_by(2) {
        let gs = i16::from_le_bytes([game_buf[pos], game_buf[pos + 1]]) as i32;
        let ms = i16::from_le_bytes([mic_buf[pos], mic_buf[pos + 1]]) as i32;
        let summed = (gs + ms).clamp(i16::MIN as i32, i16::MAX as i32) as i16;
        let b = summed.to_le_bytes();
        mixed[pos] = b[0];
        mixed[pos + 1] = b[1];
    }
    mixed
}

fn write_encoded_audio_samples<W: Write + Seek>(
    f: &mut W,
    pkts: &[&EncodedPacket],
    v_start: i64,
    total_frames: usize,
) -> Result<(Vec<MuxedSample>, u64, Vec<u8>), String> {
    if pkts.is_empty() || pkts.iter().map(|p| p.data.len()).sum::<usize>() < 4 {
        return Ok((Vec::new(), 0, Vec::new()));
    }
    let pcm_all = place_track_on_timeline(pkts, v_start, total_frames);
    write_encoded_audio_pcm(f, &pcm_all)
}

fn write_encoded_audio_pcm<W: Write + Seek>(
    f: &mut W,
    pcm_all: &[u8],
) -> Result<(Vec<MuxedSample>, u64, Vec<u8>), String> {
    if pcm_all.len() < 4 {
        return Ok((Vec::new(), 0, Vec::new()));
    }

    if let Some((aac_frames, asc)) = encode_pcm_to_aac(pcm_all) {
        if !aac_frames.is_empty() {
            let mut out = Vec::with_capacity(aac_frames.len());
            let mut total_ticks = 0_u64;
            for frame in aac_frames {
                let off = f.stream_position().map_err(|e| e.to_string())?;
                f.write_all(&frame).map_err(|e| e.to_string())?;
                out.push(MuxedSample {
                    offset: off,
                    size: frame.len() as u32,
                    duration_ticks: 1024,
                    is_sync: true,
                });
                total_ticks += 1024;
            }
            return Ok((out, total_ticks, build_aac_stsd(&asc)));
        }
    }

    let mut out = Vec::new();
    let mut total_frames = 0_u64;
    for chunk in pcm_all.chunks(4096) {
        let frames = (chunk.len() / 4) as u32;
        if frames == 0 {
            continue;
        }
        let byte_len = (frames as usize) * 4;
        let off = f.stream_position().map_err(|e| e.to_string())?;
        f.write_all(&chunk[..byte_len]).map_err(|e| e.to_string())?;
        out.push(MuxedSample {
            offset: off,
            size: byte_len as u32,
            duration_ticks: frames,
            is_sync: true,
        });
        total_frames += frames as u64;
    }
    Ok((out, total_frames, build_pcm_sowt_stsd()))
}

fn encode_pcm_to_aac(pcm_48k_stereo: &[u8]) -> Option<(Vec<Vec<u8>>, Vec<u8>)> {
    if pcm_48k_stereo.len() < 4096 {
        return None;
    }
    unsafe {
        let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        MFStartup(MF_VERSION, MFSTARTUP_NOSOCKET).ok()?;

        let out_info = MFT_REGISTER_TYPE_INFO {
            guidMajorType: MF_MEDIATYPE_AUDIO,
            guidSubtype: MFAudioFormat_AAC,
        };
        let mut activates: *mut Option<IMFActivate> = std::ptr::null_mut();
        let mut count = 0_u32;
        MFTEnumEx(
            MFT_CATEGORY_AUDIO_ENCODER,
            MFT_ENUM_FLAG_SYNCMFT | MFT_ENUM_FLAG_LOCALMFT | MFT_ENUM_FLAG_SORTANDFILTER,
            None,
            Some(&out_info),
            &mut activates,
            &mut count,
        )
        .ok()?;

        if count == 0 || activates.is_null() {
            let _ = MFShutdown();
            return None;
        }

        let mut mft_opt: Option<IMFTransform> = None;
        for i in 0..count as usize {
            if mft_opt.is_none() {
                if let Some(act) = &*activates.add(i) {
                    mft_opt = act.ActivateObject::<IMFTransform>().ok();
                }
            }
            std::ptr::drop_in_place(activates.add(i));
        }
        CoTaskMemFree(Some(activates.cast()));

        let Some(mft) = mft_opt else {
            let _ = MFShutdown();
            return None;
        };

        let res = (|| -> Option<(Vec<Vec<u8>>, Vec<u8>)> {
            let in_mt = MFCreateMediaType().ok()?;
            in_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_AUDIO).ok()?;
            in_mt.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_PCM).ok()?;
            in_mt.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).ok()?;
            in_mt.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48_000).ok()?;
            in_mt.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2).ok()?;
            in_mt.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 48_000 * 4).ok()?;
            mft.SetInputType(0, &in_mt, 0).ok()?;

            let out_mt = MFCreateMediaType().ok()?;
            out_mt.SetGUID(&MF_MT_MAJOR_TYPE, &MF_MEDIATYPE_AUDIO).ok()?;
            out_mt.SetGUID(&MF_MT_SUBTYPE, &MFAudioFormat_AAC).ok()?;
            out_mt.SetUINT32(&MF_MT_AUDIO_BITS_PER_SAMPLE, 16).ok()?;
            out_mt.SetUINT32(&MF_MT_AUDIO_SAMPLES_PER_SECOND, 48_000).ok()?;
            out_mt.SetUINT32(&MF_MT_AUDIO_NUM_CHANNELS, 2).ok()?;
            out_mt.SetUINT32(&MF_MT_AUDIO_AVG_BYTES_PER_SECOND, 24_000).ok()?;
            out_mt.SetUINT32(&MF_MT_AAC_PAYLOAD_TYPE, 0).ok()?;
            mft.SetOutputType(0, &out_mt, 0).ok()?;

            let mut asc = vec![0x11_u8, 0x90_u8];
            if let Ok(cur_out) = mft.GetOutputCurrentType(0) {
                let mut ptr: *mut u8 = std::ptr::null_mut();
                let mut len = 0_u32;
                if cur_out.GetAllocatedBlob(&MF_MT_USER_DATA, &mut ptr, &mut len).is_ok()
                    && !ptr.is_null()
                {
                    let slice = std::slice::from_raw_parts(ptr, len as usize);
                    if slice.len() > 12 {
                        asc = slice[12..].to_vec();
                    }
                    CoTaskMemFree(Some(ptr.cast()));
                }
            }

            let _ = mft.ProcessMessage(MFT_MESSAGE_NOTIFY_BEGIN_STREAMING, 0);
            let _ = mft.ProcessMessage(MFT_MESSAGE_NOTIFY_START_OF_STREAM, 0);

            let mut aac_frames = Vec::new();
            let chunk_bytes = 4096_usize;
            let mut pts_100ns = 0_i64;
            let frame_dur = (1024_i64 * 10_000_000) / 48_000;

            for chunk in pcm_48k_stereo.chunks(chunk_bytes) {
                let Ok(buf) = MFCreateMemoryBuffer(chunk.len() as u32) else { break };
                let mut ptr: *mut u8 = std::ptr::null_mut();
                if buf.Lock(&mut ptr, None, None).is_ok() && !ptr.is_null() {
                    std::ptr::copy_nonoverlapping(chunk.as_ptr(), ptr, chunk.len());
                    let _ = buf.Unlock();
                    let _ = buf.SetCurrentLength(chunk.len() as u32);
                }
                if let Ok(sample) = MFCreateSample() {
                    let _ = sample.AddBuffer(&buf);
                    let _ = sample.SetSampleTime(pts_100ns);
                    let _ = sample.SetSampleDuration(frame_dur);
                    let _ = mft.ProcessInput(0, &sample, 0);
                    pts_100ns += frame_dur;
                }
                drain_aac_outputs(&mft, &mut aac_frames);
            }

            let _ = mft.ProcessMessage(MFT_MESSAGE_COMMAND_DRAIN, 0);
            drain_aac_outputs(&mft, &mut aac_frames);

            Some((aac_frames, asc))
        })();

        let _ = MFShutdown();
        res
    }
}

unsafe fn drain_aac_outputs(mft: &IMFTransform, out: &mut Vec<Vec<u8>>) {
    let Ok(info) = mft.GetOutputStreamInfo(0) else { return };
    let mft_alloc = (info.dwFlags & 0x0000_0100) != 0;
    loop {
        let sample_opt = if mft_alloc {
            None
        } else {
            let Ok(s) = MFCreateSample() else { break };
            let Ok(b) = MFCreateMemoryBuffer(info.cbSize.max(8192)) else { break };
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
        if let Some(sample) = ManuallyDrop::into_inner(out_buf.pSample) {
            if res.is_ok() {
                if let Ok(buf) = sample.ConvertToContiguousBuffer() {
                    let mut ptr: *mut u8 = std::ptr::null_mut();
                    let mut len = 0_u32;
                    if buf.Lock(&mut ptr, None, Some(&mut len)).is_ok() && !ptr.is_null() && len > 0
                    {
                        out.push(std::slice::from_raw_parts(ptr, len as usize).to_vec());
                        let _ = buf.Unlock();
                    }
                }
            }
        }
        let _ = ManuallyDrop::into_inner(out_buf.pEvents);
        match res {
            Ok(()) => continue,
            Err(e) if e.code() == MF_E_TRANSFORM_NEED_MORE_INPUT => break,
            Err(_) => break,
        }
    }
}

fn mux_packets_to_mkv(
    path: &Path,
    packets: &[EncodedPacket],
    width: u32,
    height: u32,
    fps: u32,
    codec: VideoCodec,
    separate_audio_tracks: bool,
) -> Result<(), String> {
    let mut out = Vec::with_capacity(256 * 1024);

    let mut ebml = Vec::new();
    ebml_uint(&mut ebml, 0x4286, 1);
    ebml_uint(&mut ebml, 0x42F7, 1);
    ebml_uint(&mut ebml, 0x42F2, 4);
    ebml_uint(&mut ebml, 0x42F3, 8);
    ebml_str(&mut ebml, 0x4282, b"matroska");
    ebml_uint(&mut ebml, 0x4287, 4);
    ebml_uint(&mut ebml, 0x4285, 2);
    ebml_master(&mut out, 0x1A45DFA3, &ebml);

    let mut seg = Vec::new();

    let mut info = Vec::new();
    ebml_uint(&mut info, 0x2AD7B1, 1_000_000);
    ebml_str(&mut info, 0x4D80, b"Rewind Lossless Remuxer");
    ebml_str(&mut info, 0x5741, b"Rewind");
    let max_pts_ms = packets
        .iter()
        .map(|p| (p.pts_100ns + p.duration_100ns) / 10_000)
        .max()
        .unwrap_or(1000)
        .max(1) as f64;
    ebml_f64(&mut info, 0x4489, max_pts_ms);
    ebml_master(&mut seg, 0x1549A966, &info);

    let mut tracks = Vec::new();
    let codec_id = match codec {
        VideoCodec::Hevc => b"V_MPEGH/ISO/HEVC".as_slice(),
        VideoCodec::Av1 => b"V_AV1".as_slice(),
        VideoCodec::H264 => b"V_MPEG4/ISO/AVC".as_slice(),
    };
    let mut v_entry = Vec::new();
    ebml_uint(&mut v_entry, 0xD7, 1);
    ebml_uint(&mut v_entry, 0x73C5, 1);
    ebml_uint(&mut v_entry, 0x83, 1);
    ebml_str(&mut v_entry, 0x86, codec_id);
    ebml_uint(&mut v_entry, 0x23E383, (1_000_000_000 / fps.max(1)) as u64);
    let mut v_pix = Vec::new();
    ebml_uint(&mut v_pix, 0xB0, width as u64);
    ebml_uint(&mut v_pix, 0xBA, height as u64);
    ebml_master(&mut v_entry, 0xE0, &v_pix);
    ebml_master(&mut tracks, 0xAE, &v_entry);

    let has_game_audio = if separate_audio_tracks {
        packets.iter().any(|p| p.track == PacketTrack::GameAudio)
    } else {
        packets
            .iter()
            .any(|p| p.track == PacketTrack::GameAudio || p.track == PacketTrack::MicAudio)
    };
    let has_mic_audio = separate_audio_tracks && packets.iter().any(|p| p.track == PacketTrack::MicAudio);

    if has_game_audio {
        tracks.extend_from_slice(&build_mkv_audio_track(2));
    }
    if has_mic_audio {
        tracks.extend_from_slice(&build_mkv_audio_track(3));
    }
    ebml_master(&mut seg, 0x1654AE6B, &tracks);

    let mut cluster = Vec::new();
    ebml_uint(&mut cluster, 0xE7, 0);
    for p in packets {
        let track_num: u8 = match p.track {
            PacketTrack::Video => 1,
            PacketTrack::GameAudio => 2,
            PacketTrack::MicAudio if separate_audio_tracks => 3,
            PacketTrack::MicAudio => 2,
        };
        let ts_ms = ((p.pts_100ns / 10_000).clamp(0, i16::MAX as i64)) as i16;
        let flags: u8 = if p.is_idr_keyframe || p.track != PacketTrack::Video {
            0x80
        } else {
            0x00
        };
        let mut blk = Vec::with_capacity(4 + p.data.len());
        blk.push(0x80 | track_num);
        blk.extend_from_slice(&ts_ms.to_be_bytes());
        blk.push(flags);
        blk.extend_from_slice(&p.data);
        ebml_bin(&mut cluster, 0xA3, &blk);
    }
    ebml_master(&mut seg, 0x1F43B675, &cluster);
    ebml_master(&mut out, 0x18538067, &seg);

    fs::write(path, out).map_err(|e| format!("write mkv: {e}"))
}

fn build_mkv_audio_track(num: u64) -> Vec<u8> {
    let mut a_entry = Vec::new();
    ebml_uint(&mut a_entry, 0xD7, num);
    ebml_uint(&mut a_entry, 0x73C5, num);
    ebml_uint(&mut a_entry, 0x83, 2);
    ebml_str(&mut a_entry, 0x86, b"A_PCM/INT/LIT");
    let mut a_set = Vec::new();
    ebml_f64(&mut a_set, 0xB5, 48_000.0);
    ebml_uint(&mut a_set, 0x9F, 2);
    ebml_uint(&mut a_set, 0x6264, 16);
    ebml_master(&mut a_entry, 0xE1, &a_set);
    let mut wrap = Vec::new();
    ebml_master(&mut wrap, 0xAE, &a_entry);
    wrap
}

fn ebml_id(buf: &mut Vec<u8>, id: u32) {
    let bytes = id.to_be_bytes();
    let start = bytes.iter().position(|&b| b != 0).unwrap_or(3);
    buf.extend_from_slice(&bytes[start..]);
}

fn ebml_vint_size(buf: &mut Vec<u8>, len: usize) {
    if len < 127 {
        buf.push(0x80 | len as u8);
    } else if len < 16383 {
        let v = (0x4000_u16 | len as u16).to_be_bytes();
        buf.extend_from_slice(&v);
    } else if len < 2_097_151 {
        let v = (0x20_0000_u32 | len as u32).to_be_bytes();
        buf.extend_from_slice(&v[1..]);
    } else {
        let v = (0x1000_0000_u32 | len as u32).to_be_bytes();
        buf.extend_from_slice(&v);
    }
}

fn ebml_master(buf: &mut Vec<u8>, id: u32, body: &[u8]) {
    ebml_id(buf, id);
    ebml_vint_size(buf, body.len());
    buf.extend_from_slice(body);
}

fn ebml_bin(buf: &mut Vec<u8>, id: u32, body: &[u8]) {
    ebml_master(buf, id, body);
}

fn ebml_str(buf: &mut Vec<u8>, id: u32, s: &[u8]) {
    ebml_master(buf, id, s);
}

fn ebml_uint(buf: &mut Vec<u8>, id: u32, val: u64) {
    let bytes = val.to_be_bytes();
    let first = bytes.iter().position(|&b| b != 0).unwrap_or(7);
    ebml_master(buf, id, &bytes[first..]);
}

fn ebml_f64(buf: &mut Vec<u8>, id: u32, val: f64) {
    ebml_master(buf, id, &val.to_be_bytes());
}

fn split_annex_b_nals(data: &[u8]) -> Vec<&[u8]> {
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
    if starts.is_empty() {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(starts.len());
    for idx in 0..starts.len() {
        let (pos, sc_len) = starts[idx];
        let payload_start = pos + sc_len;
        let payload_end = if idx + 1 < starts.len() {
            starts[idx + 1].0
        } else {
            data.len()
        };
        if payload_start < payload_end {
            out.push(&data[payload_start..payload_end]);
        }
    }
    out
}

fn write_box<W: Write>(w: &mut W, tag: [u8; 4], payload: &[u8]) -> std::io::Result<()> {
    let sz = (8 + payload.len()) as u32;
    w.write_all(&sz.to_be_bytes())?;
    w.write_all(&tag)?;
    w.write_all(payload)
}

fn box_bytes(tag: [u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(8 + payload.len());
    out.extend_from_slice(&((8 + payload.len()) as u32).to_be_bytes());
    out.extend_from_slice(&tag);
    out.extend_from_slice(payload);
    out
}

fn build_mvhd(timescale: u32, duration: u32, next_track_id: u32) -> Vec<u8> {
    let mut b = vec![0_u8; 100];
    b[12..16].copy_from_slice(&timescale.to_be_bytes());
    b[16..20].copy_from_slice(&duration.to_be_bytes());
    b[20..24].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
    b[24..26].copy_from_slice(&0x0100_u16.to_be_bytes());
    b[36..40].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
    b[52..56].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
    b[68..72].copy_from_slice(&0x4000_0000_u32.to_be_bytes());
    b[96..100].copy_from_slice(&next_track_id.to_be_bytes());
    box_bytes(*b"mvhd", &b)
}

fn build_tkhd(track_id: u32, duration: u32, width: u32, height: u32, is_audio: bool) -> Vec<u8> {
    let mut b = vec![0_u8; 84];
    b[3] = 0x03;
    b[12..16].copy_from_slice(&track_id.to_be_bytes());
    b[20..24].copy_from_slice(&duration.to_be_bytes());
    if is_audio {
        b[36..38].copy_from_slice(&0x0100_u16.to_be_bytes());
    }
    b[40..44].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
    b[56..60].copy_from_slice(&0x0001_0000_u32.to_be_bytes());
    b[72..76].copy_from_slice(&0x4000_0000_u32.to_be_bytes());
    b[76..80].copy_from_slice(&(width << 16).to_be_bytes());
    b[80..84].copy_from_slice(&(height << 16).to_be_bytes());
    box_bytes(*b"tkhd", &b)
}

fn build_mdhd(timescale: u32, duration: u32) -> Vec<u8> {
    let mut b = vec![0_u8; 24];
    b[12..16].copy_from_slice(&timescale.to_be_bytes());
    b[16..20].copy_from_slice(&duration.to_be_bytes());
    b[20..22].copy_from_slice(&0x55C4_u16.to_be_bytes());
    box_bytes(*b"mdhd", &b)
}

fn build_hdlr(handler_type: [u8; 4], name: &[u8]) -> Vec<u8> {
    let mut b = vec![0_u8; 24];
    b[8..12].copy_from_slice(&handler_type);
    b.extend_from_slice(name);
    b.push(0);
    box_bytes(*b"hdlr", &b)
}

fn build_dinf() -> Vec<u8> {
    let url = box_bytes(*b"url ", &[0, 0, 0, 1]);
    let mut dref = vec![0, 0, 0, 0, 0, 0, 0, 1];
    dref.extend_from_slice(&url);
    box_bytes(*b"dinf", &box_bytes(*b"dref", &dref))
}

fn build_video_stsd(
    codec: VideoCodec,
    width: u32,
    height: u32,
    vps: &[Vec<u8>],
    sps: &[Vec<u8>],
    pps: &[Vec<u8>],
) -> Vec<u8> {
    let (sample_tag, cfg_box) = match codec {
        VideoCodec::H264 => (*b"avc1", build_avcc_box(sps, pps)),
        VideoCodec::Hevc => (*b"hvc1", build_hvcc_box(vps, sps, pps)),
        VideoCodec::Av1 => (*b"av01", box_bytes(*b"av1C", &[0x81, 0x04, 0x0C, 0x00])),
    };

    let mut entry = vec![0_u8; 78];
    entry[6..8].copy_from_slice(&1_u16.to_be_bytes());
    entry[24..26].copy_from_slice(&(width as u16).to_be_bytes());
    entry[26..28].copy_from_slice(&(height as u16).to_be_bytes());
    entry[28..32].copy_from_slice(&0x0048_0000_u32.to_be_bytes());
    entry[32..36].copy_from_slice(&0x0048_0000_u32.to_be_bytes());
    entry[40..42].copy_from_slice(&1_u16.to_be_bytes());
    entry[74..76].copy_from_slice(&24_u16.to_be_bytes());
    entry[76..78].copy_from_slice(&(-1_i16).to_be_bytes());
    entry.extend_from_slice(&cfg_box);

    let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
    stsd.extend_from_slice(&box_bytes(sample_tag, &entry));
    box_bytes(*b"stsd", &stsd)
}

fn build_avcc_box(sps: &[Vec<u8>], pps: &[Vec<u8>]) -> Vec<u8> {
    let default_sps = vec![0x67, 0x64, 0x00, 0x2A, 0xAC, 0xD9, 0x40, 0x78, 0x02, 0x27, 0xE5, 0xC0, 0x44, 0x00, 0x00, 0x03, 0x00, 0x04, 0x00, 0x00, 0x03, 0x00, 0xF0, 0x3C, 0x60, 0xC6, 0x58];
    let default_pps = vec![0x68, 0xEB, 0xE3, 0xCB, 0x22, 0xC0];
    let sps_ref = if sps.is_empty() { &default_sps } else { &sps[0] };
    let pps_ref = if pps.is_empty() { &default_pps } else { &pps[0] };

    let mut b = Vec::new();
    b.push(1);
    b.push(*sps_ref.get(1).unwrap_or(&0x64));
    b.push(*sps_ref.get(2).unwrap_or(&0x00));
    b.push(*sps_ref.get(3).unwrap_or(&0x2A));
    b.push(0xFF);
    b.push(0xE1);
    b.extend_from_slice(&(sps_ref.len() as u16).to_be_bytes());
    b.extend_from_slice(sps_ref);
    b.push(1);
    b.extend_from_slice(&(pps_ref.len() as u16).to_be_bytes());
    b.extend_from_slice(pps_ref);
    box_bytes(*b"avcC", &b)
}

fn build_hvcc_box(vps: &[Vec<u8>], sps: &[Vec<u8>], pps: &[Vec<u8>]) -> Vec<u8> {
    let mut b = vec![0_u8; 23];
    b[0] = 1;
    b[1] = 0x01;
    b[2] = 0x60;
    b[12] = 0x99;
    b[13] = 0xF0;
    b[14] = 0x00;
    b[15] = 0xFC;
    b[16] = 0xFD;
    b[17] = 0xF8;
    b[18] = 0xF8;
    b[21] = 0xFF;
    let mut arrays: Vec<(u8, &[u8])> = Vec::new();
    if let Some(v) = vps.first() {
        arrays.push((32, v.as_slice()));
    }
    if let Some(s) = sps.first() {
        arrays.push((33, s.as_slice()));
    }
    if let Some(p) = pps.first() {
        arrays.push((34, p.as_slice()));
    }
    b[22] = arrays.len() as u8;
    for (nal_type, nal) in arrays {
        b.push(0x80 | (nal_type & 0x3F));
        b.extend_from_slice(&1_u16.to_be_bytes());
        b.extend_from_slice(&(nal.len() as u16).to_be_bytes());
        b.extend_from_slice(nal);
    }
    box_bytes(*b"hvcC", &b)
}

fn build_pcm_sowt_stsd() -> Vec<u8> {
    let mut entry = vec![0_u8; 28];
    entry[6..8].copy_from_slice(&1_u16.to_be_bytes());
    entry[16..18].copy_from_slice(&2_u16.to_be_bytes());
    entry[18..20].copy_from_slice(&16_u16.to_be_bytes());
    entry[24..26].copy_from_slice(&48_000_u16.to_be_bytes());
    let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
    stsd.extend_from_slice(&box_bytes(*b"sowt", &entry));
    box_bytes(*b"stsd", &stsd)
}

fn build_aac_stsd(asc: &[u8]) -> Vec<u8> {
    let asc_slice = if asc.is_empty() { &[0x11, 0x90] } else { asc };
    let mut dec_spec = vec![0x05_u8, asc_slice.len() as u8];
    dec_spec.extend_from_slice(asc_slice);

    let mut dec_cfg_body = Vec::new();
    dec_cfg_body.push(0x40);
    dec_cfg_body.push(0x15);
    dec_cfg_body.extend_from_slice(&[0x00, 0x00, 0x00]);
    dec_cfg_body.extend_from_slice(&192_000_u32.to_be_bytes());
    dec_cfg_body.extend_from_slice(&192_000_u32.to_be_bytes());
    dec_cfg_body.extend_from_slice(&dec_spec);
    let mut dec_cfg = vec![0x04_u8, dec_cfg_body.len() as u8];
    dec_cfg.extend_from_slice(&dec_cfg_body);

    let sl_cfg = [0x06_u8, 0x01, 0x02];

    let mut es_body = Vec::new();
    es_body.extend_from_slice(&[0x00, 0x01, 0x00]);
    es_body.extend_from_slice(&dec_cfg);
    es_body.extend_from_slice(&sl_cfg);

    let mut esds = vec![0_u8, 0, 0, 0, 0x03, es_body.len() as u8];
    esds.extend_from_slice(&es_body);
    let esds_box = box_bytes(*b"esds", &esds);

    let mut entry = vec![0_u8; 28];
    entry[6..8].copy_from_slice(&1_u16.to_be_bytes());
    entry[16..18].copy_from_slice(&2_u16.to_be_bytes());
    entry[18..20].copy_from_slice(&16_u16.to_be_bytes());
    entry[24..26].copy_from_slice(&48_000_u16.to_be_bytes());
    entry.extend_from_slice(&esds_box);

    let mut stsd = vec![0, 0, 0, 0, 0, 0, 0, 1];
    stsd.extend_from_slice(&box_bytes(*b"mp4a", &entry));
    box_bytes(*b"stsd", &stsd)
}

fn build_video_trak(
    track_id: u32,
    movie_dur: u32,
    timescale: u32,
    media_dur: u32,
    width: u32,
    height: u32,
    stsd: &[u8],
    samples: &[MuxedSample],
) -> Vec<u8> {
    let tkhd = build_tkhd(track_id, movie_dur, width, height, false);
    let mdhd = build_mdhd(timescale, media_dur);
    let hdlr = build_hdlr(*b"vide", b"Rewind Video");
    let vmhd = box_bytes(*b"vmhd", &[0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0]);
    let dinf = build_dinf();
    let stbl = build_stbl(stsd, samples, true);
    let mut minf = Vec::new();
    minf.extend_from_slice(&vmhd);
    minf.extend_from_slice(&dinf);
    minf.extend_from_slice(&stbl);
    let mut mdia = Vec::new();
    mdia.extend_from_slice(&mdhd);
    mdia.extend_from_slice(&hdlr);
    mdia.extend_from_slice(&box_bytes(*b"minf", &minf));
    let mut trak = Vec::new();
    trak.extend_from_slice(&tkhd);
    trak.extend_from_slice(&box_bytes(*b"mdia", &mdia));
    box_bytes(*b"trak", &trak)
}

fn build_audio_trak(
    track_id: u32,
    movie_dur: u32,
    timescale: u32,
    media_dur: u32,
    stsd: &[u8],
    samples: &[MuxedSample],
) -> Vec<u8> {
    let tkhd = build_tkhd(track_id, movie_dur, 0, 0, true);
    let mdhd = build_mdhd(timescale, media_dur);
    let hdlr = build_hdlr(*b"soun", b"Rewind Audio");
    let smhd = box_bytes(*b"smhd", &[0, 0, 0, 0, 0, 0, 0, 0]);
    let dinf = build_dinf();
    let stbl = build_stbl(stsd, samples, false);
    let mut minf = Vec::new();
    minf.extend_from_slice(&smhd);
    minf.extend_from_slice(&dinf);
    minf.extend_from_slice(&stbl);
    let mut mdia = Vec::new();
    mdia.extend_from_slice(&mdhd);
    mdia.extend_from_slice(&hdlr);
    mdia.extend_from_slice(&box_bytes(*b"minf", &minf));
    let mut trak = Vec::new();
    trak.extend_from_slice(&tkhd);
    trak.extend_from_slice(&box_bytes(*b"mdia", &mdia));
    box_bytes(*b"trak", &trak)
}

fn build_stbl(stsd: &[u8], samples: &[MuxedSample], emit_stss: bool) -> Vec<u8> {
    let mut stts_runs: Vec<(u32, u32)> = Vec::new();
    for s in samples {
        if let Some(last) = stts_runs.last_mut() {
            if last.1 == s.duration_ticks {
                last.0 += 1;
                continue;
            }
        }
        stts_runs.push((1, s.duration_ticks));
    }
    let mut stts = vec![0, 0, 0, 0];
    stts.extend_from_slice(&(stts_runs.len() as u32).to_be_bytes());
    for (cnt, dur) in stts_runs {
        stts.extend_from_slice(&cnt.to_be_bytes());
        stts.extend_from_slice(&dur.to_be_bytes());
    }

    let mut stsc = vec![0, 0, 0, 0];
    stsc.extend_from_slice(&1_u32.to_be_bytes());
    stsc.extend_from_slice(&1_u32.to_be_bytes());
    stsc.extend_from_slice(&1_u32.to_be_bytes());
    stsc.extend_from_slice(&1_u32.to_be_bytes());

    let mut stsz = vec![0, 0, 0, 0, 0, 0, 0, 0];
    stsz.extend_from_slice(&(samples.len() as u32).to_be_bytes());
    for s in samples {
        stsz.extend_from_slice(&s.size.to_be_bytes());
    }

    let use_co64 = samples.iter().any(|s| s.offset > u32::MAX as u64);
    let chunk_box = if use_co64 {
        let mut co64 = vec![0, 0, 0, 0];
        co64.extend_from_slice(&(samples.len() as u32).to_be_bytes());
        for s in samples {
            co64.extend_from_slice(&s.offset.to_be_bytes());
        }
        box_bytes(*b"co64", &co64)
    } else {
        let mut stco = vec![0, 0, 0, 0];
        stco.extend_from_slice(&(samples.len() as u32).to_be_bytes());
        for s in samples {
            stco.extend_from_slice(&(s.offset as u32).to_be_bytes());
        }
        box_bytes(*b"stco", &stco)
    };

    let mut body = Vec::new();
    body.extend_from_slice(stsd);
    body.extend_from_slice(&box_bytes(*b"stts", &stts));
    if emit_stss {
        let syncs: Vec<u32> = samples
            .iter()
            .enumerate()
            .filter_map(|(i, s)| if s.is_sync { Some((i + 1) as u32) } else { None })
            .collect();
        if !syncs.is_empty() {
            let mut stss = vec![0, 0, 0, 0];
            stss.extend_from_slice(&(syncs.len() as u32).to_be_bytes());
            for idx in syncs {
                stss.extend_from_slice(&idx.to_be_bytes());
            }
            body.extend_from_slice(&box_bytes(*b"stss", &stss));
        }
    }
    body.extend_from_slice(&box_bytes(*b"stsc", &stsc));
    body.extend_from_slice(&box_bytes(*b"stsz", &stsz));
    body.extend_from_slice(&chunk_box);
    box_bytes(*b"stbl", &body)
}


struct ParsedTrack {
    is_video: bool,
    width: u32,
    height: u32,
    timescale: u32,
    stsd_box: Vec<u8>,
    codec_label: String,
    samples: Vec<MuxedSample>,
}

fn iter_boxes(data: &[u8]) -> Vec<([u8; 4], &[u8])> {
    let mut out = Vec::new();
    let mut off = 0;
    while off + 8 <= data.len() {
        let sz = u32::from_be_bytes(data[off..off + 4].try_into().unwrap()) as usize;
        let tag: [u8; 4] = data[off + 4..off + 8].try_into().unwrap();
        let (hdr, total) = if sz == 1 && off + 16 <= data.len() {
            let big = u64::from_be_bytes(data[off + 8..off + 16].try_into().unwrap()) as usize;
            (16, big)
        } else if sz == 0 {
            (8, data.len() - off)
        } else {
            (8, sz)
        };
        if total < hdr || off + total > data.len() {
            break;
        }
        out.push((tag, &data[off + hdr..off + total]));
        off += total;
    }
    out
}

fn find_moov_bytes(file_data: &[u8]) -> Option<&[u8]> {
    for (tag, body) in iter_boxes(file_data) {
        if &tag == b"moov" {
            return Some(body);
        }
    }
    None
}

fn parse_mp4_tracks(file_data: &[u8]) -> Result<(u32, u32, Vec<ParsedTrack>), String> {
    let moov = find_moov_bytes(file_data).ok_or_else(|| "Missing moov atom".to_owned())?;
    let mut movie_timescale = 1000_u32;
    let mut movie_duration = 0_u32;
    let mut tracks = Vec::new();

    for (tag, body) in iter_boxes(moov) {
        if &tag == b"mvhd" && body.len() >= 20 {
            movie_timescale = u32::from_be_bytes(body[12..16].try_into().unwrap()).max(1);
            movie_duration = u32::from_be_bytes(body[16..20].try_into().unwrap());
        } else if &tag == b"trak" {
            if let Some(t) = parse_single_trak(body) {
                tracks.push(t);
            }
        }
    }

    if tracks.is_empty() {
        return Err("No valid tracks in MP4".to_owned());
    }
    Ok((movie_timescale, movie_duration, tracks))
}

fn parse_single_trak(trak_body: &[u8]) -> Option<ParsedTrack> {
    let mut width = 0_u32;
    let mut height = 0_u32;
    let mut timescale = 90_000_u32;
    let mut is_video = false;
    let mut stsd_box = Vec::new();
    let mut codec_label = String::new();
    let mut stts_entries: Vec<(u32, u32)> = Vec::new();
    let mut stsc_entries: Vec<(u32, u32, u32)> = Vec::new();
    let mut stsz_sizes: Vec<u32> = Vec::new();
    let mut chunk_offsets: Vec<u64> = Vec::new();
    let mut sync_set: Vec<u32> = Vec::new();

    for (tag, body) in iter_boxes(trak_body) {
        if &tag == b"tkhd" && body.len() >= 84 {
            width = u32::from_be_bytes(body[76..80].try_into().unwrap()) >> 16;
            height = u32::from_be_bytes(body[80..84].try_into().unwrap()) >> 16;
        } else if &tag == b"mdia" {
            for (mtag, mbody) in iter_boxes(body) {
                if &mtag == b"mdhd" && mbody.len() >= 20 {
                    timescale = u32::from_be_bytes(mbody[12..16].try_into().unwrap()).max(1);
                } else if &mtag == b"hdlr" && mbody.len() >= 12 {
                    is_video = &mbody[8..12] == b"vide";
                } else if &mtag == b"minf" {
                    for (nt, nb) in iter_boxes(mbody) {
                        if &nt == b"stbl" {
                            for (st, sb) in iter_boxes(nb) {
                                match &st {
                                    b"stsd" => {
                                        stsd_box = box_bytes(*b"stsd", sb);
                                        if sb.len() >= 16 {
                                            let fourcc = &sb[12..16];
                                            codec_label = match fourcc {
                                                b"hvc1" | b"hev1" => "HEVC".to_owned(),
                                                b"avc1" | b"avc3" => "H.264".to_owned(),
                                                b"av01" => "AV1".to_owned(),
                                                _ => String::from_utf8_lossy(fourcc).into_owned(),
                                            };
                                            if is_video && (width == 0 || height == 0) && sb.len() >= 44 {
                                                width = u16::from_be_bytes(sb[40..42].try_into().unwrap()) as u32;
                                                height = u16::from_be_bytes(sb[42..44].try_into().unwrap()) as u32;
                                            }
                                        }
                                    }
                                    b"stts" if sb.len() >= 8 => {
                                        let cnt = u32::from_be_bytes(sb[4..8].try_into().unwrap()) as usize;
                                        for i in 0..cnt {
                                            let p = 8 + i * 8;
                                            if p + 8 <= sb.len() {
                                                stts_entries.push((
                                                    u32::from_be_bytes(sb[p..p + 4].try_into().unwrap()),
                                                    u32::from_be_bytes(sb[p + 4..p + 8].try_into().unwrap()),
                                                ));
                                            }
                                        }
                                    }
                                    b"stsc" if sb.len() >= 8 => {
                                        let cnt = u32::from_be_bytes(sb[4..8].try_into().unwrap()) as usize;
                                        for i in 0..cnt {
                                            let p = 8 + i * 12;
                                            if p + 12 <= sb.len() {
                                                stsc_entries.push((
                                                    u32::from_be_bytes(sb[p..p + 4].try_into().unwrap()),
                                                    u32::from_be_bytes(sb[p + 4..p + 8].try_into().unwrap()),
                                                    u32::from_be_bytes(sb[p + 8..p + 12].try_into().unwrap()),
                                                ));
                                            }
                                        }
                                    }
                                    b"stsz" if sb.len() >= 12 => {
                                        let const_sz = u32::from_be_bytes(sb[4..8].try_into().unwrap());
                                        let cnt = u32::from_be_bytes(sb[8..12].try_into().unwrap()) as usize;
                                        if const_sz > 0 {
                                            stsz_sizes = vec![const_sz; cnt];
                                        } else {
                                            for i in 0..cnt {
                                                let p = 12 + i * 4;
                                                if p + 4 <= sb.len() {
                                                    stsz_sizes.push(u32::from_be_bytes(sb[p..p + 4].try_into().unwrap()));
                                                }
                                            }
                                        }
                                    }
                                    b"stco" if sb.len() >= 8 => {
                                        let cnt = u32::from_be_bytes(sb[4..8].try_into().unwrap()) as usize;
                                        for i in 0..cnt {
                                            let p = 8 + i * 4;
                                            if p + 4 <= sb.len() {
                                                chunk_offsets.push(u32::from_be_bytes(sb[p..p + 4].try_into().unwrap()) as u64);
                                            }
                                        }
                                    }
                                    b"co64" if sb.len() >= 8 => {
                                        let cnt = u32::from_be_bytes(sb[4..8].try_into().unwrap()) as usize;
                                        for i in 0..cnt {
                                            let p = 8 + i * 8;
                                            if p + 8 <= sb.len() {
                                                chunk_offsets.push(u64::from_be_bytes(sb[p..p + 8].try_into().unwrap()));
                                            }
                                        }
                                    }
                                    b"stss" if sb.len() >= 8 => {
                                        let cnt = u32::from_be_bytes(sb[4..8].try_into().unwrap()) as usize;
                                        for i in 0..cnt {
                                            let p = 8 + i * 4;
                                            if p + 4 <= sb.len() {
                                                sync_set.push(u32::from_be_bytes(sb[p..p + 4].try_into().unwrap()));
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if stsz_sizes.is_empty() || chunk_offsets.is_empty() {
        return None;
    }

    let mut durs = Vec::with_capacity(stsz_sizes.len());
    for (cnt, dur) in stts_entries {
        for _ in 0..cnt {
            if durs.len() >= stsz_sizes.len() {
                break;
            }
            durs.push(dur.max(1));
        }
    }
    while durs.len() < stsz_sizes.len() {
        durs.push(1);
    }

    let mut samples = Vec::with_capacity(stsz_sizes.len());
    let mut sample_idx = 0_usize;
    let total_chunks = chunk_offsets.len();
    for c_idx in 0..total_chunks {
        let chunk_num = (c_idx + 1) as u32;
        let samples_in_chunk = stsc_entries
            .iter()
            .rev()
            .find(|(first_chunk, _, _)| *first_chunk <= chunk_num)
            .map(|(_, spc, _)| *spc)
            .unwrap_or(1);

        let mut off = chunk_offsets[c_idx];
        for _ in 0..samples_in_chunk {
            if sample_idx >= stsz_sizes.len() {
                break;
            }
            let sz = stsz_sizes[sample_idx];
            let dur = durs[sample_idx];
            let is_sync = sync_set.is_empty() || sync_set.binary_search(&((sample_idx + 1) as u32)).is_ok();
            samples.push(MuxedSample {
                offset: off,
                size: sz,
                duration_ticks: dur,
                is_sync,
            });
            off += sz as u64;
            sample_idx += 1;
        }
    }

    Some(ParsedTrack {
        is_video,
        width,
        height,
        timescale,
        stsd_box,
        codec_label,
        samples,
    })
}

pub fn probe_mp4_metadata(path: &Path) -> Result<Mp4Metadata, String> {
    let data = fs::read(path).map_err(|e| format!("read mp4: {e}"))?;
    let (movie_ts, movie_dur, tracks) = parse_mp4_tracks(&data)?;
    let vid = tracks
        .iter()
        .find(|t| t.is_video)
        .ok_or_else(|| "No video track in MP4".to_owned())?;

    let vid_ticks: u64 = vid.samples.iter().map(|s| s.duration_ticks as u64).sum();
    let duration_secs = if vid_ticks > 0 {
        vid_ticks as f32 / vid.timescale as f32
    } else {
        movie_dur as f32 / movie_ts as f32
    };
    let fps = if duration_secs > 0.05 {
        (vid.samples.len() as f32 / duration_secs).round().clamp(1.0, 240.0) as u32
    } else {
        60
    };

    let mut keyframe_times_secs = Vec::new();
    let mut acc_ticks = 0_u64;
    for s in &vid.samples {
        if s.is_sync {
            keyframe_times_secs.push(acc_ticks as f32 / vid.timescale as f32);
        }
        acc_ticks += s.duration_ticks as u64;
    }

    Ok(Mp4Metadata {
        duration_secs,
        width: vid.width,
        height: vid.height,
        fps,
        codec: vid.codec_label.clone(),
        keyframe_times_secs,
        track_count: tracks.len(),
    })
}

pub fn trim_mp4_lossless(
    src_path: &Path,
    dst_path: &Path,
    start_secs: f32,
    end_secs: f32,
    vertical_9_16: bool,
) -> Result<Mp4Metadata, String> {
    let src_bytes = fs::read(src_path).map_err(|e| format!("read source mp4: {e}"))?;
    let (_, _, tracks) = parse_mp4_tracks(&src_bytes)?;
    let vid_idx = tracks
        .iter()
        .position(|t| t.is_video)
        .ok_or_else(|| "Source MP4 missing video track".to_owned())?;

    let vid = &tracks[vid_idx];
    let target_start_ticks = (start_secs.max(0.0) * vid.timescale as f32) as u64;
    let target_end_ticks = (end_secs.max(start_secs + 0.25) * vid.timescale as f32) as u64;

    let mut acc = 0_u64;
    let mut start_sample_idx = 0_usize;
    let mut actual_start_ticks = 0_u64;
    let mut end_sample_idx = vid.samples.len();

    for (i, s) in vid.samples.iter().enumerate() {
        if s.is_sync && acc <= target_start_ticks {
            start_sample_idx = i;
            actual_start_ticks = acc;
        }
        acc += s.duration_ticks as u64;
        if acc >= target_end_ticks && i >= start_sample_idx {
            end_sample_idx = i + 1;
            break;
        }
    }
    if start_sample_idx >= end_sample_idx {
        end_sample_idx = (start_sample_idx + 1).min(vid.samples.len());
    }

    let actual_start_secs = actual_start_ticks as f32 / vid.timescale as f32;
    let vid_slice_ticks: u64 = vid.samples[start_sample_idx..end_sample_idx]
        .iter()
        .map(|s| s.duration_ticks as u64)
        .sum();
    let actual_end_secs = actual_start_secs + (vid_slice_ticks as f32 / vid.timescale as f32);

    if let Some(parent) = dst_path.parent() {
        fs::create_dir_all(parent).map_err(|e| format!("create_dir_all: {e}"))?;
    }
    let tmp_dst = dst_path.with_extension("mp4.trimtmp");
    let mut f = BufWriter::new(File::create(&tmp_dst).map_err(|e| format!("create trimmed mp4: {e}"))?);

    let mut ftyp = Vec::new();
    ftyp.extend_from_slice(b"isom");
    ftyp.extend_from_slice(&512_u32.to_be_bytes());
    ftyp.extend_from_slice(b"isomiso2avc1mp41");
    write_box(&mut f, *b"ftyp", &ftyp).map_err(|e| e.to_string())?;

    let mdat_pos = f.stream_position().map_err(|e| e.to_string())?;
    f.write_all(&0_u32.to_be_bytes()).map_err(|e| e.to_string())?;
    f.write_all(b"mdat").map_err(|e| e.to_string())?;

    let mut out_tracks_samples: Vec<(usize, Vec<MuxedSample>, u64)> = Vec::new();
    for (t_idx, tr) in tracks.iter().enumerate() {
        let (s_from, s_to) = if t_idx == vid_idx {
            (start_sample_idx, end_sample_idx)
        } else {
            let t_start = (actual_start_secs * tr.timescale as f32) as u64;
            let t_end = (actual_end_secs * tr.timescale as f32) as u64;
            let mut a = 0_u64;
            let mut from_i = 0_usize;
            let mut to_i = tr.samples.len();
            let mut found_from = false;
            for (i, s) in tr.samples.iter().enumerate() {
                if !found_from && a >= t_start {
                    from_i = i;
                    found_from = true;
                }
                a += s.duration_ticks as u64;
                if a >= t_end {
                    to_i = i + 1;
                    break;
                }
            }
            (from_i, to_i.max(from_i))
        };

        let mut copied = Vec::new();
        let mut total_ticks = 0_u64;
        for s in &tr.samples[s_from..s_to] {
            let start = s.offset as usize;
            let end = start + s.size as usize;
            if end <= src_bytes.len() {
                let new_off = f.stream_position().map_err(|e| e.to_string())?;
                f.write_all(&src_bytes[start..end]).map_err(|e| e.to_string())?;
                copied.push(MuxedSample {
                    offset: new_off,
                    size: s.size,
                    duration_ticks: s.duration_ticks,
                    is_sync: s.is_sync,
                });
                total_ticks += s.duration_ticks as u64;
            }
        }
        out_tracks_samples.push((t_idx, copied, total_ticks));
    }

    let mdat_end = f.stream_position().map_err(|e| e.to_string())?;
    let mdat_size = (mdat_end - mdat_pos) as u32;
    f.flush().map_err(|e| e.to_string())?;
    {
        let inner = f.get_mut();
        inner.seek(SeekFrom::Start(mdat_pos)).map_err(|e| e.to_string())?;
        inner.write_all(&mdat_size.to_be_bytes()).map_err(|e| e.to_string())?;
        inner.seek(SeekFrom::Start(mdat_end)).map_err(|e| e.to_string())?;
    }

    let movie_timescale = 1000_u32;
    let (_, _, vid_ticks_out) = &out_tracks_samples[vid_idx];
    let movie_dur_ms = ((*vid_ticks_out * movie_timescale as u64) / vid.timescale as u64).max(1) as u32;

    let mut moov_body = Vec::new();
    moov_body.extend_from_slice(&build_mvhd(movie_timescale, movie_dur_ms, (tracks.len() + 1) as u32));

    let (out_w, out_h) = if vertical_9_16 {
        let w = ((vid.height * 9) / 16) & !1;
        (w.max(360), vid.height.max(640))
    } else {
        (vid.width, vid.height)
    };

    for (t_idx, samples_out, ticks_out) in out_tracks_samples {
        if samples_out.is_empty() {
            continue;
        }
        let tr = &tracks[t_idx];
        let dur_ms = ((ticks_out * movie_timescale as u64) / tr.timescale as u64).max(1) as u32;
        if tr.is_video {
            moov_body.extend_from_slice(&build_video_trak(
                (t_idx + 1) as u32,
                dur_ms,
                tr.timescale,
                ticks_out as u32,
                out_w,
                out_h,
                &tr.stsd_box,
                &samples_out,
            ));
        } else {
            moov_body.extend_from_slice(&build_audio_trak(
                (t_idx + 1) as u32,
                dur_ms,
                tr.timescale,
                ticks_out as u32,
                &tr.stsd_box,
                &samples_out,
            ));
        }
    }

    write_box(&mut f, *b"moov", &moov_body).map_err(|e| e.to_string())?;
    f.flush().map_err(|e| e.to_string())?;
    drop(f);

    if dst_path.exists() {
        let _ = fs::remove_file(dst_path);
    }
    fs::rename(&tmp_dst, dst_path).map_err(|e| format!("rename trimmed mp4: {e}"))?;
    probe_mp4_metadata(dst_path)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn synthetic_h264_annexb_packets(secs: i64, fps: u32) -> Vec<EncodedPacket> {
        let sps_pps_idr: Vec<u8> = vec![
            0, 0, 0, 1, 0x67, 0x64, 0x00, 0x2A, 0xAC, 0xD9, 0x40, 0x78,
            0, 0, 0, 1, 0x68, 0xEB, 0xE3, 0xCB,
            0, 0, 0, 1, 0x65, 0x88, 0x84, 0x21, 0xA0,
        ];
        let p_frame: Vec<u8> = vec![0, 0, 0, 1, 0x41, 0x9A, 0x24, 0x6C];
        let mut pkts = Vec::new();
        let total = secs * (fps as i64);
        let dur = 10_000_000 / (fps as i64);

        for i in 0..total {
            let is_idr = i % (fps as i64 * 2) == 0;
            pkts.push(EncodedPacket {
                track: PacketTrack::Video,
                pts_100ns: i * dur,
                dts_100ns: i * dur,
                duration_100ns: dur,
                is_idr_keyframe: is_idr,
                data: if is_idr { sps_pps_idr.clone() } else { p_frame.clone() },
            });
            if i % 3 == 0 {
                pkts.push(EncodedPacket {
                    track: PacketTrack::GameAudio,
                    pts_100ns: i * dur,
                    dts_100ns: i * dur,
                    duration_100ns: dur * 3,
                    is_idr_keyframe: false,
                    data: vec![0_u8; 4096],
                });
            }
            if i % 3 == 0 {
                pkts.push(EncodedPacket {
                    track: PacketTrack::MicAudio,
                    pts_100ns: i * dur,
                    dts_100ns: i * dur,
                    duration_100ns: dur * 3,
                    is_idr_keyframe: false,
                    data: vec![0_u8; 4096],
                });
            }
        }
        pkts
    }

    #[test]
    fn muxes_probes_and_losslessly_trims_mp4_at_idr_boundary() {
        let dir = std::env::temp_dir().join("rewind_remux_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("source.mp4");
        let dst = dir.join("trimmed.mp4");

        let pkts = synthetic_h264_annexb_packets(6, 30);
        mux_packets_to_mp4(&src, &pkts, 1920, 1080, 30, VideoCodec::H264, true).unwrap();

        let meta = probe_mp4_metadata(&src).unwrap();
        assert_eq!(meta.width, 1920);
        assert_eq!(meta.height, 1080);
        assert_eq!(meta.track_count, 3);
        assert!((meta.duration_secs - 6.0).abs() < 0.25);
        assert_eq!(meta.keyframe_times_secs.len(), 3);

        let pcm = crate::player::probe_audio_stream_pcm(&src).unwrap();
        assert!(!pcm.is_empty());

        let trimmed_meta = trim_mp4_lossless(&src, &dst, 2.5, 5.5, true).unwrap();
        assert_eq!(trimmed_meta.width, 606);
        assert_eq!(trimmed_meta.height, 1080);
        assert_eq!(trimmed_meta.track_count, 3);
        assert!(trimmed_meta.duration_secs >= 3.0 && trimmed_meta.duration_secs <= 4.0);

        let _ = fs::remove_dir_all(&dir);
    }

    static GPU_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn captures_muxes_and_decodes_real_video_and_aac_audio_when_gpu_available() {
        let _gpu_guard = GPU_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let cfg = crate::config::AppConfig::default();
        let hw = crate::capture::detect_hardware();
        let Some(mut pipe) = crate::capture::GpuCapturePipeline::open(&cfg, &hw) else {
            return;
        };

        let mut pkts = Vec::new();
        let mut start_qpc = None;
        for step in 0..24_i64 {
            let mut frame_pkts = pipe.capture_and_encode_frame();
            if start_qpc.is_none() {
                if let Some(first) = frame_pkts.first() {
                    start_qpc = Some(first.pts_100ns);
                }
            }
            pkts.append(&mut frame_pkts);
            if let Some(q0) = start_qpc {
                let pts = q0 + step * 333_333;
                pkts.push(EncodedPacket {
                    track: PacketTrack::GameAudio,
                    pts_100ns: pts,
                    dts_100ns: pts,
                    duration_100ns: 333_333,
                    is_idr_keyframe: false,
                    data: vec![0_u8; 6400],
                });
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        }

        if pkts.iter().all(|p| p.track != PacketTrack::Video || !p.is_idr_keyframe) {
            return;
        }

        let dir = std::env::temp_dir().join("rewind_real_dec_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mp4 = dir.join("real_av.mp4");
        let (w, h) = cfg.resolution.dimensions();

        mux_packets_to_mp4(&mp4, &pkts, w, h, cfg.fps.as_u32(), cfg.codec, false).unwrap();

        let (has_audio, has_video, has_pcm) = crate::player::probe_first_av_sync(&mp4).unwrap();
        assert!(has_audio);
        assert!(has_video);
        assert!(has_pcm);

        let dec = crate::player::VideoDecoder::open(&mp4, 2.0).unwrap();
        dec.set_audio_gain(0.0);
        let mut seq = 0_u64;
        let mut got_img = None;
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            if let Some(img) = dec.take_latest_frame(&mut seq) {
                got_img = Some(img);
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let img = got_img.expect("expected decoded native-resolution frame");
        assert_eq!(img.size, [w as usize, h as usize]);
        assert!(dec.preload_progress().0);
        drop(dec);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn mixes_mic_audio_into_primary_mp4_track_when_separate_tracks_disabled() {
        let dir = std::env::temp_dir().join("rewind_mic_mix_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let src = dir.join("mic_mix.mp4");

        let mut pkts = synthetic_h264_annexb_packets(2, 30);
        let mut frame_sample = 0_usize;
        for pkt in &mut pkts {
            if pkt.track == PacketTrack::MicAudio {
                let mut pcm = Vec::with_capacity(4096);
                for _ in 0..1024 {
                    let phase = (frame_sample as f32) * std::f32::consts::TAU * 440.0 / 48_000.0;
                    let s = (phase.sin() * 14_000.0) as i16;
                    let b = s.to_le_bytes();
                    pcm.extend_from_slice(&b);
                    pcm.extend_from_slice(&b);
                    frame_sample += 1;
                }
                pkt.data = pcm;
            }
        }

        mux_packets_to_mp4(&src, &pkts, 1920, 1080, 30, VideoCodec::H264, false).unwrap();

        let meta = probe_mp4_metadata(&src).unwrap();
        assert_eq!(meta.track_count, 2);

        let pcm = crate::player::probe_audio_stream_pcm(&src).unwrap();
        let peak = pcm
            .chunks_exact(2)
            .map(|c| (i16::from_le_bytes([c[0], c[1]]) as i32).abs())
            .max()
            .unwrap_or(0);
        assert!(peak > 1000, "Expected non-zero mic audio amplitude, got peak {peak}");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn captures_120fps_exact_timing_when_gpu_available() {
        let _gpu_guard = GPU_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut cfg = crate::config::AppConfig::default();
        cfg.fps = crate::config::FrameRate::Fps120;
        cfg.cqp_qp = 22;
        let hw = crate::capture::detect_hardware();
        let Some(mut pipe) = crate::capture::GpuCapturePipeline::open(&cfg, &hw) else {
            return;
        };

        let timer = crate::win_ffi::HighResTimer::new();
        let origin = crate::win_ffi::qpc_now_100ns();
        let mut pkts = Vec::new();
        for step in 0..120_i64 {
            let mut frame_pkts = pipe.capture_and_encode_frame();
            pkts.append(&mut frame_pkts);
            let target = origin + (((step + 1) as i128 * 10_000_000) / 120) as i64;
            let rem = target - crate::win_ffi::qpc_now_100ns();
            if rem > 1_000 {
                if let Some(t) = &timer {
                    t.wait_100ns(rem);
                } else {
                    std::thread::sleep(std::time::Duration::from_micros((rem / 10) as u64));
                }
            }
        }
        drop(pipe);

        let vid_cnt = pkts.iter().filter(|p| p.track == PacketTrack::Video).count();
        if vid_cnt == 0 {
            return;
        }
        assert!(vid_cnt >= 100, "too few 120fps packets: {vid_cnt}");

        let dir = std::env::temp_dir().join("rewind_120fps_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mp4 = dir.join("test_120fps.mp4");
        let (w, h) = cfg.resolution.dimensions();
        mux_packets_to_mp4(&mp4, &pkts, w, h, 120, cfg.codec, false).unwrap();

        let meta = probe_mp4_metadata(&mp4).unwrap();
        let vid_pkts: Vec<&EncodedPacket> = pkts
            .iter()
            .filter(|p| p.track == PacketTrack::Video)
            .collect();
        let deltas: Vec<(usize, i64, i64)> = vid_pkts
            .windows(2)
            .enumerate()
            .filter_map(|(i, w)| {
                let d = w[1].pts_100ns - w[0].pts_100ns;
                if (d - 83_333).abs() > 2 {
                    Some((i, d, w[0].duration_100ns))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(
            meta.fps,
            120,
            "cnt={} dur={:.4}s weird_deltas={:?} last_dur={}",
            vid_pkts.len(),
            meta.duration_secs,
            deltas,
            vid_pkts.last().map(|p| p.duration_100ns).unwrap_or(-1)
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn captures_1440p_without_speedup_or_av_drift_when_gpu_available() {
        let _gpu_guard = GPU_TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let mut cfg = crate::config::AppConfig::default();
        cfg.resolution = crate::config::Resolution::P1440;
        cfg.fps = crate::config::FrameRate::Fps120;
        cfg.codec = crate::config::VideoCodec::Hevc;
        cfg.cqp_qp = 22;
        let hw = crate::capture::detect_hardware();
        let Some(mut pipe) = crate::capture::GpuCapturePipeline::open(&cfg, &hw) else {
            return;
        };

        let timer = crate::win_ffi::HighResTimer::new();
        let origin = crate::win_ffi::qpc_now_100ns();
        let deadline = origin + 20_000_000;
        let mut pkts = Vec::new();
        let mut step = 0_i64;
        while crate::win_ffi::qpc_now_100ns() < deadline {
            let mut frame_pkts = pipe.capture_and_encode_frame();
            pkts.append(&mut frame_pkts);
            step += 1;
            let target = origin + ((step as i128 * 10_000_000) / 120) as i64;
            let rem = target - crate::win_ffi::qpc_now_100ns();
            if rem > 1_000 {
                if let Some(t) = &timer {
                    t.wait_100ns(rem);
                } else {
                    std::thread::sleep(std::time::Duration::from_micros((rem / 10) as u64));
                }
            }
        }
        let end_qpc = crate::win_ffi::qpc_now_100ns();
        drop(pipe);

        let vid_pkts: Vec<&EncodedPacket> = pkts
            .iter()
            .filter(|p| p.track == PacketTrack::Video)
            .collect();
        if vid_pkts.len() < 10 {
            return;
        }
        let av_drift_s = (end_qpc - vid_pkts.last().unwrap().pts_100ns).abs() as f64 / 10_000_000.0;
        assert!(
            av_drift_s < 0.25,
            "1440p video PTS drifted from wall-clock QPC by {av_drift_s:.3}s"
        );

        let dir = std::env::temp_dir().join("rewind_1440p_realtime_test");
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let mp4 = dir.join("test_1440p.mp4");
        let (w, h) = cfg.resolution.dimensions();
        mux_packets_to_mp4(&mp4, &pkts, w, h, 120, pipe_codec_or(cfg.codec, &hw), false).unwrap();

        let meta = probe_mp4_metadata(&mp4).unwrap();
        assert!(
            meta.duration_secs >= 1.65 && meta.duration_secs <= 2.40,
            "expected ~2.0s real-time 1440p duration, got {:.3}s (samples={})",
            meta.duration_secs,
            vid_pkts.len()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    fn pipe_codec_or(req: VideoCodec, hw: &crate::capture::HardwareInfo) -> VideoCodec {
        if hw.is_codec_supported(req) {
            req
        } else {
            VideoCodec::H264
        }
    }
}
