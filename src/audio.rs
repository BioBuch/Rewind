use std::collections::VecDeque;
use std::f32::consts::TAU;
use std::sync::OnceLock;

use windows::core::PCWSTR;
use windows::Win32::Media::Audio::{
    eCapture, eCommunications, eMultimedia, eRender, waveInGetDevCapsW, waveInGetNumDevs,
    waveOutClose, waveOutGetDevCapsW, waveOutGetNumDevs, waveOutOpen, waveOutPrepareHeader,
    waveOutReset, waveOutUnprepareHeader, waveOutWrite, IAudioCaptureClient, IAudioClient,
    IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, PlaySoundW, AUDCLNT_BUFFERFLAGS_SILENT,
    AUDCLNT_SHAREMODE_SHARED, AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM, AUDCLNT_STREAMFLAGS_LOOPBACK,
    AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY, CALLBACK_NULL, HWAVEOUT, SND_ASYNC, SND_MEMORY,
    SND_NODEFAULT, WAVEFORMATEX, WAVEHDR, WAVEINCAPSW, WAVEOUTCAPSW, WAVE_FORMAT_PCM, WHDR_DONE,
};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoTaskMemFree, CoUninitialize, CLSCTX_ALL,
    COINIT_MULTITHREADED,
};

#[link(name = "winmm")]
extern "system" {
    fn waveInMessage(hwi: usize, umsg: u32, dw1: usize, dw2: usize) -> u32;
    fn waveOutMessage(hwo: usize, umsg: u32, dw1: usize, dw2: usize) -> u32;
}
const DRV_QUERYFUNCTIONINSTANCEID: u32 = 0x0811;
const DRV_QUERYFUNCTIONINSTANCEIDSIZE: u32 = 0x0812;

use crate::ring_buffer::{EncodedPacket, PacketTrack};
use crate::win_ffi::{from_wide_slice, qpc_now_100ns};

pub fn enumerate_output_devices() -> Vec<String> {
    let mut out = vec!["Default System Output".to_owned()];
    unsafe {
        let count = waveOutGetNumDevs();
        for idx in 0..count {
            let mut caps = WAVEOUTCAPSW::default();
            if waveOutGetDevCapsW(
                idx as usize,
                &mut caps,
                std::mem::size_of::<WAVEOUTCAPSW>() as u32,
            ) == 0
            {
                let pname = caps.szPname;
                let name = from_wide_slice(&pname).trim().to_owned();
                if !name.is_empty() && !out.contains(&name) {
                    out.push(name);
                }
            }
        }
    }
    out
}

pub fn enumerate_input_devices() -> Vec<String> {
    let mut out = vec!["Default Input Device".to_owned()];
    unsafe {
        let count = waveInGetNumDevs();
        for idx in 0..count {
            let mut caps = WAVEINCAPSW::default();
            if waveInGetDevCapsW(
                idx as usize,
                &mut caps,
                std::mem::size_of::<WAVEINCAPSW>() as u32,
            ) == 0
            {
                let pname = caps.szPname;
                let name = from_wide_slice(&pname).trim().to_owned();
                if !name.is_empty() && !out.contains(&name) {
                    out.push(name);
                }
            }
        }
    }
    out
}

fn pcm_48k_format(channels: u16) -> WAVEFORMATEX {
    let ch = channels.max(1);
    let block_align = ch * 2;
    WAVEFORMATEX {
        wFormatTag: WAVE_FORMAT_PCM as u16,
        nChannels: ch,
        nSamplesPerSec: 48_000,
        nAvgBytesPerSec: 48_000 * (block_align as u32),
        nBlockAlign: block_align,
        wBitsPerSample: 16,
        cbSize: 0,
    }
}

unsafe fn resolve_wavein_endpoint_id(device_hint: &str) -> Option<Vec<u16>> {
    let hint = device_hint.trim().to_ascii_lowercase();
    let count = waveInGetNumDevs();
    let mut best_idx: Option<u32> = None;
    let mut mic_fallback_idx: Option<u32> = None;

    for idx in 0..count {
        let mut caps = WAVEINCAPSW::default();
        if waveInGetDevCapsW(
            idx as usize,
            &mut caps,
            std::mem::size_of::<WAVEINCAPSW>() as u32,
        ) == 0
        {
            let pname = caps.szPname;
            let name = from_wide_slice(&pname).trim().to_ascii_lowercase();
            if name.is_empty() {
                continue;
            }
            if mic_fallback_idx.is_none() && name.contains("microphone") {
                mic_fallback_idx = Some(idx);
            }
            if !hint.is_empty() && hint != "default input device" {
                if name == hint || name.contains(&hint) || hint.contains(&name) {
                    best_idx = Some(idx);
                    break;
                }
            }
        }
    }

    let chosen = best_idx.or(mic_fallback_idx)?;
    let mut cb_bytes: u32 = 0;
    if waveInMessage(
        chosen as usize,
        DRV_QUERYFUNCTIONINSTANCEIDSIZE,
        (&mut cb_bytes as *mut u32) as usize,
        0,
    ) != 0
        || cb_bytes < 2
    {
        return None;
    }
    let wchar_len = ((cb_bytes as usize) + 1) / 2;
    let mut buf = vec![0_u16; wchar_len.max(1)];
    if waveInMessage(
        chosen as usize,
        DRV_QUERYFUNCTIONINSTANCEID,
        buf.as_mut_ptr() as usize,
        buf.len() * 2,
    ) != 0
    {
        return None;
    }
    if *buf.last().unwrap_or(&1) != 0 {
        buf.push(0);
    }
    Some(buf)
}

unsafe fn resolve_waveout_endpoint_id(device_hint: &str) -> Option<Vec<u16>> {
    let hint = device_hint.trim().to_ascii_lowercase();
    if hint.is_empty() || hint == "default system output" {
        return None;
    }
    let count = waveOutGetNumDevs();
    let mut best_idx: Option<u32> = None;
    for idx in 0..count {
        let mut caps = WAVEOUTCAPSW::default();
        if waveOutGetDevCapsW(
            idx as usize,
            &mut caps,
            std::mem::size_of::<WAVEOUTCAPSW>() as u32,
        ) == 0
        {
            let pname = caps.szPname;
            let name = from_wide_slice(&pname).trim().to_ascii_lowercase();
            if !name.is_empty() && (name == hint || name.contains(&hint) || hint.contains(&name)) {
                best_idx = Some(idx);
                break;
            }
        }
    }
    let chosen = best_idx?;
    let mut cb_bytes: u32 = 0;
    if waveOutMessage(
        chosen as usize,
        DRV_QUERYFUNCTIONINSTANCEIDSIZE,
        (&mut cb_bytes as *mut u32) as usize,
        0,
    ) != 0
        || cb_bytes < 2
    {
        return None;
    }
    let wchar_len = (cb_bytes as usize).div_ceil(2);
    let mut buf = vec![0_u16; wchar_len.max(1)];
    if waveOutMessage(
        chosen as usize,
        DRV_QUERYFUNCTIONINSTANCEID,
        buf.as_mut_ptr() as usize,
        buf.len() * 2,
    ) != 0
    {
        return None;
    }
    if *buf.last().unwrap_or(&1) != 0 {
        buf.push(0);
    }
    Some(buf)
}

pub struct WasapiTrackStream {
    audio_client: IAudioClient,
    capture_client: IAudioCaptureClient,
    track: PacketTrack,
    channels: u16,
    last_packet_qpc: i64,
}

impl WasapiTrackStream {
    pub fn open_system_loopback_fallback() -> Option<Self> {
        Self::open_loopback("Default System Output")
    }

    pub fn open_loopback(device_hint: &str) -> Option<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let trimmed = device_hint.trim();
            let dev: Option<IMMDevice> =
                if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("Default System Output") {
                    resolve_waveout_endpoint_id(trimmed)
                        .and_then(|id| enumerator.GetDevice(PCWSTR(id.as_ptr())).ok())
                } else {
                    None
                };
            let dev = dev.or_else(|| enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia).ok())?;
            let client: IAudioClient = dev.Activate(CLSCTX_ALL, None).ok()?;
            let fmt = pcm_48k_format(2);
            let flags = AUDCLNT_STREAMFLAGS_LOOPBACK
                | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
            client
                .Initialize(AUDCLNT_SHAREMODE_SHARED, flags, 2_000_000, 0, &fmt, None)
                .ok()?;
            let capture_client: IAudioCaptureClient = client.GetService().ok()?;
            client.Start().ok()?;

            Some(Self {
                audio_client: client,
                capture_client,
                track: PacketTrack::GameAudio,
                channels: 2,
                last_packet_qpc: 0,
            })
        }
    }

    pub fn open_microphone(device_hint: &str) -> Option<Self> {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).ok()?;
            let trimmed = device_hint.trim();
            let dev: Option<IMMDevice> = if !trimmed.is_empty() && !trimmed.eq_ignore_ascii_case("Default Input Device") {
                resolve_wavein_endpoint_id(trimmed)
                    .and_then(|id| enumerator.GetDevice(PCWSTR(id.as_ptr())).ok())
            } else {
                None
            };
            let dev = dev
                .or_else(|| enumerator.GetDefaultAudioEndpoint(eCapture, eMultimedia).ok())
                .or_else(|| enumerator.GetDefaultAudioEndpoint(eCapture, eCommunications).ok())
                .or_else(|| {
                    resolve_wavein_endpoint_id("Microphone")
                        .and_then(|id| enumerator.GetDevice(PCWSTR(id.as_ptr())).ok())
                })?;

            let mut client: IAudioClient = dev.Activate(CLSCTX_ALL, None).ok().or_else(|| {
                let fallback = resolve_wavein_endpoint_id("Microphone")
                    .and_then(|id| enumerator.GetDevice(PCWSTR(id.as_ptr())).ok())?;
                fallback.Activate(CLSCTX_ALL, None).ok()
            })?;

            let mut native_ch = 2_u16;
            if let Ok(mix_fmt) = client.GetMixFormat() {
                if !mix_fmt.is_null() {
                    native_ch = (*mix_fmt).nChannels.max(1);
                    CoTaskMemFree(Some(mix_fmt.cast()));
                }
            }

            let flags = AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY;
            let first_ch = if native_ch == 1 { 1 } else { 2 };
            let second_ch = if first_ch == 1 { 2 } else { 1 };

            let mut chosen_ch = first_ch;
            if client
                .Initialize(
                    AUDCLNT_SHAREMODE_SHARED,
                    flags,
                    2_000_000,
                    0,
                    &pcm_48k_format(first_ch),
                    None,
                )
                .is_err()
            {
                client = dev.Activate(CLSCTX_ALL, None).ok()?;
                client
                    .Initialize(
                        AUDCLNT_SHAREMODE_SHARED,
                        flags,
                        2_000_000,
                        0,
                        &pcm_48k_format(second_ch),
                        None,
                    )
                    .ok()?;
                chosen_ch = second_ch;
            }

            let capture_client: IAudioCaptureClient = client.GetService().ok()?;
            client.Start().ok()?;

            Some(Self {
                audio_client: client,
                capture_client,
                track: PacketTrack::MicAudio,
                channels: chosen_ch,
                last_packet_qpc: 0,
            })
        }
    }

    pub fn drain_packets(&mut self, volume_pct: u32, muted: bool) -> Vec<EncodedPacket> {
        let mut raw_chunks: Vec<(Vec<u8>, i64)> = Vec::new();
        let gain = if muted { 0.0 } else { (volume_pct.min(100) as f32) / 100.0 };
        let ch = self.channels.max(1) as usize;

        unsafe {
            loop {
                let Ok(pkt_frames) = self.capture_client.GetNextPacketSize() else {
                    break;
                };
                if pkt_frames == 0 {
                    break;
                }
                let mut data_ptr: *mut u8 = std::ptr::null_mut();
                let mut num_frames = 0_u32;
                let mut flags = 0_u32;
                if self
                    .capture_client
                    .GetBuffer(&mut data_ptr, &mut num_frames, &mut flags, None, None)
                    .is_err()
                {
                    break;
                }

                let frames = num_frames as usize;
                let pcm = if frames == 0 {
                    Vec::new()
                } else if (flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0 || muted || data_ptr.is_null() {
                    vec![0_u8; frames * 4]
                } else if ch == 1 {
                    let src_bytes = std::slice::from_raw_parts(data_ptr, frames * 2);
                    let mut out = Vec::with_capacity(frames * 4);
                    for s_bytes in src_bytes.chunks_exact(2) {
                        let raw = i16::from_le_bytes([s_bytes[0], s_bytes[1]]);
                        let scaled = ((raw as f32) * gain).clamp(-32768.0, 32767.0) as i16;
                        let b = scaled.to_le_bytes();
                        out.extend_from_slice(&b);
                        out.extend_from_slice(&b);
                    }
                    out
                } else {
                    let src_bytes = std::slice::from_raw_parts(data_ptr, frames * ch * 2);
                    let mut out = Vec::with_capacity(frames * 4);
                    let mut peak_l = 0_i32;
                    let mut peak_r = 0_i32;
                    for frame in src_bytes.chunks_exact(ch * 2) {
                        let l_raw = i16::from_le_bytes([frame[0], frame[1]]);
                        let r_raw = i16::from_le_bytes([frame[2], frame[3]]);
                        let l = ((l_raw as f32) * gain).clamp(-32768.0, 32767.0) as i16;
                        let r = ((r_raw as f32) * gain).clamp(-32768.0, 32767.0) as i16;
                        peak_l = peak_l.max((l as i32).abs());
                        peak_r = peak_r.max((r as i32).abs());
                        out.extend_from_slice(&l.to_le_bytes());
                        out.extend_from_slice(&r.to_le_bytes());
                    }
                    if self.track == PacketTrack::MicAudio {
                        if peak_l > 64 && peak_r == 0 {
                            for stereo in out.chunks_exact_mut(4) {
                                stereo[2] = stereo[0];
                                stereo[3] = stereo[1];
                            }
                        } else if peak_r > 64 && peak_l == 0 {
                            for stereo in out.chunks_exact_mut(4) {
                                stereo[0] = stereo[2];
                                stereo[1] = stereo[3];
                            }
                        }
                    }
                    out
                };

                let _ = self.capture_client.ReleaseBuffer(num_frames);

                if !pcm.is_empty() {
                    let dur_100ns = (((num_frames as i64) * 10_000_000) / 48_000).max(10_000);
                    raw_chunks.push((pcm, dur_100ns));
                }
            }
        }

        let mut packets = Vec::with_capacity(raw_chunks.len());
        if !raw_chunks.is_empty() {
            let total_dur_100ns: i64 = raw_chunks.iter().map(|(_, d)| *d).sum();
            let now = qpc_now_100ns();
            let ideal_start = (now - total_dur_100ns).max(0);
            if self.last_packet_qpc == 0
                || (ideal_start - self.last_packet_qpc).abs() > 400_000
                || self.last_packet_qpc > now
            {
                self.last_packet_qpc = ideal_start;
            }
            for (pcm, dur_100ns) in raw_chunks {
                let pts = self.last_packet_qpc;
                packets.push(EncodedPacket {
                    track: self.track,
                    pts_100ns: pts,
                    dts_100ns: pts,
                    duration_100ns: dur_100ns,
                    is_idr_keyframe: false,
                    data: pcm,
                });
                self.last_packet_qpc += dur_100ns;
            }
        }

        packets
    }
}

impl Drop for WasapiTrackStream {
    fn drop(&mut self) {
        unsafe {
            let _ = self.audio_client.Stop();
            CoUninitialize();
        }
    }
}

pub fn play_clip_chime() {
    static CHIME_WAV: OnceLock<Vec<u8>> = OnceLock::new();
    let wav = CHIME_WAV.get_or_init(build_clip_chime_wav);
    unsafe {
        let _ = PlaySoundW(
            PCWSTR(wav.as_ptr().cast()),
            None,
            SND_MEMORY | SND_ASYNC | SND_NODEFAULT,
        );
    }
}

fn build_clip_chime_wav() -> Vec<u8> {
    let sample_rate = 48_000_usize;
    let total_samples = (sample_rate * 230) / 1000;
    let data_bytes = (total_samples * 4) as u32;
    let mut buf = Vec::with_capacity(44 + data_bytes as usize);

    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(36 + data_bytes).to_le_bytes());
    buf.extend_from_slice(b"WAVEfmt ");
    buf.extend_from_slice(&16_u32.to_le_bytes());
    buf.extend_from_slice(&1_u16.to_le_bytes());
    buf.extend_from_slice(&2_u16.to_le_bytes());
    buf.extend_from_slice(&(sample_rate as u32).to_le_bytes());
    buf.extend_from_slice(&((sample_rate * 4) as u32).to_le_bytes());
    buf.extend_from_slice(&4_u16.to_le_bytes());
    buf.extend_from_slice(&16_u16.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_bytes.to_le_bytes());

    let split = (sample_rate * 85) / 1000;
    for i in 0..total_samples {
        let t = i as f32 / sample_rate as f32;
        let note1_env = {
            let env_a = (t / 0.008).min(1.0);
            let env_d = (-t * 16.0).exp();
            env_a * env_d
        };
        let note2 = if i >= split {
            let t2 = (i - split) as f32 / sample_rate as f32;
            let env_a = (t2 / 0.008).min(1.0);
            let env_d = (-t2 * 14.0).exp();
            ((TAU * 880.0 * t2).sin() * 0.72 + (TAU * 1318.5 * t2).sin() * 0.28) * env_a * env_d
        } else {
            0.0
        };
        let note1 = ((TAU * 587.33 * t).sin() * 0.75 + (TAU * 1174.66 * t).sin() * 0.25) * note1_env;
        let mixed = ((note1 * 0.45 + note2 * 0.65) * 0.34).clamp(-0.95, 0.95);
        let s = (mixed * 32767.0) as i16;
        let bytes = s.to_le_bytes();
        buf.extend_from_slice(&bytes);
        buf.extend_from_slice(&bytes);
    }
    buf
}

struct QueuedWaveChunk {
    _data: Box<[u8]>,
    hdr: Box<WAVEHDR>,
}

pub struct PcmWaveOutPlayer {
    hwo: HWAVEOUT,
    queue: VecDeque<QueuedWaveChunk>,
}

impl PcmWaveOutPlayer {
    pub fn open() -> Option<Self> {
        let fmt = pcm_48k_format(2);
        let mut hwo = HWAVEOUT::default();
        let res = unsafe {
            waveOutOpen(
                Some(&mut hwo),
                u32::MAX,
                &fmt,
                0,
                0,
                CALLBACK_NULL,
            )
        };
        if res != 0 {
            return None;
        }
        Some(Self {
            hwo,
            queue: VecDeque::new(),
        })
    }

    pub fn push_pcm(&mut self, pcm: Vec<u8>) {
        if pcm.is_empty() {
            return;
        }
        self.reclaim_finished();
        let mut data = pcm.into_boxed_slice();
        let mut hdr = Box::new(WAVEHDR {
            lpData: windows::core::PSTR(data.as_mut_ptr()),
            dwBufferLength: data.len() as u32,
            ..Default::default()
        });
        unsafe {
            let sz = std::mem::size_of::<WAVEHDR>() as u32;
            if waveOutPrepareHeader(self.hwo, hdr.as_mut(), sz) == 0 {
                let _ = waveOutWrite(self.hwo, hdr.as_mut(), sz);
                self.queue.push_back(QueuedWaveChunk { _data: data, hdr });
            }
        }
    }

    pub fn reset(&mut self) {
        unsafe {
            let _ = waveOutReset(self.hwo);
            let sz = std::mem::size_of::<WAVEHDR>() as u32;
            while let Some(mut chunk) = self.queue.pop_front() {
                let _ = waveOutUnprepareHeader(self.hwo, chunk.hdr.as_mut(), sz);
            }
        }
    }

    fn reclaim_finished(&mut self) {
        let sz = std::mem::size_of::<WAVEHDR>() as u32;
        while let Some(front) = self.queue.front() {
            if (front.hdr.dwFlags & WHDR_DONE) == 0 {
                break;
            }
            if let Some(mut chunk) = self.queue.pop_front() {
                unsafe {
                    let _ = waveOutUnprepareHeader(self.hwo, chunk.hdr.as_mut(), sz);
                }
            }
        }
    }
}

impl Drop for PcmWaveOutPlayer {
    fn drop(&mut self) {
        self.reset();
        unsafe {
            let _ = waveOutClose(self.hwo);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enumerates_real_audio_endpoints() {
        let outputs = enumerate_output_devices();
        let inputs = enumerate_input_devices();
        assert!(!outputs.is_empty());
        assert!(!inputs.is_empty());
        assert_eq!(outputs[0], "Default System Output");
        assert_eq!(inputs[0], "Default Input Device");
    }

    #[test]
    fn clip_chime_wav_is_valid_riff() {
        let wav = build_clip_chime_wav();
        assert!(wav.len() > 44);
        assert_eq!(&wav[0..4], b"RIFF");
        assert_eq!(&wav[8..12], b"WAVE");
    }

    #[test]
    fn opens_configured_microphone_and_drains_stereo_pcm() {
        let inputs = enumerate_input_devices();
        let physical = inputs
            .iter()
            .find(|n| n.contains("TONOR") || n.contains("Microphone"))
            .cloned();
        let hint = physical
            .clone()
            .unwrap_or_else(|| "Default Input Device".to_owned());
        let stream = WasapiTrackStream::open_microphone(&hint);
        if physical.is_some() {
            assert!(
                stream.is_some(),
                "Expected physical microphone '{hint}' to open cleanly via WASAPI"
            );
        }
        if let Some(mut s) = stream {
            std::thread::sleep(std::time::Duration::from_millis(70));
            let pkts = s.drain_packets(100, false);
            assert!(!pkts.is_empty());
            for p in &pkts {
                assert_eq!(p.track, PacketTrack::MicAudio);
                assert_eq!(p.data.len() % 4, 0);
            }
        }
    }
}
