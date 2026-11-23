use std::collections::VecDeque;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PacketTrack {
    Video,
    GameAudio,
    MicAudio,
}

#[derive(Debug, Clone)]
pub struct EncodedPacket {
    pub track: PacketTrack,
    pub pts_100ns: i64,
    pub dts_100ns: i64,
    pub duration_100ns: i64,
    pub is_idr_keyframe: bool,
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, Copy)]
struct PacketMeta {
    track: PacketTrack,
    pts_100ns: i64,
    dts_100ns: i64,
    duration_100ns: i64,
    is_idr_keyframe: bool,
    offset: usize,
    len: usize,
}

fn commit_pages(buf: &mut [u8]) {
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

fn meta_intersects_range(meta: &PacketMeta, seg_start: usize, seg_end: usize) -> bool {
    if seg_start >= seg_end {
        return false;
    }
    if meta.len == 0 {
        meta.offset >= seg_start && meta.offset < seg_end
    } else {
        meta.offset < seg_end && seg_start < meta.offset + meta.len
    }
}

pub struct CompressedRingBuffer {
    slab: Vec<u8>,
    packets: VecDeque<PacketMeta>,
    max_duration_100ns: i64,
    max_bytes_cap: usize,
    used_bytes: usize,
    write_head: usize,
    latest_pts_100ns: i64,
}

impl CompressedRingBuffer {
    pub fn new(max_duration_secs: u32, max_bytes_cap: usize) -> Self {
        Self {
            slab: Vec::new(),
            packets: VecDeque::with_capacity(8192),
            max_duration_100ns: (max_duration_secs.max(1) as i64) * 10_000_000,
            max_bytes_cap,
            used_bytes: 0,
            write_head: 0,
            latest_pts_100ns: i64::MIN,
        }
    }

    pub fn set_max_duration_secs(&mut self, secs: u32) {
        self.max_duration_100ns = (secs.max(1) as i64) * 10_000_000;
        self.evict_by_time();
    }

    pub fn set_budget(&mut self, max_duration_secs: u32, max_bytes_cap: usize) {
        self.max_duration_100ns = (max_duration_secs.max(1) as i64) * 10_000_000;
        if self.max_bytes_cap != max_bytes_cap {
            self.max_bytes_cap = max_bytes_cap;
            if !self.slab.is_empty() {
                self.preallocate();
            }
        } else {
            self.evict_by_time();
        }
    }

    pub fn preallocate(&mut self) {
        if self.max_bytes_cap == 0 {
            self.clear();
            return;
        }
        if self.slab.len() == self.max_bytes_cap {
            return;
        }
        self.packets.clear();
        self.used_bytes = 0;
        self.write_head = 0;
        self.latest_pts_100ns = i64::MIN;
        let mut new_slab = Vec::new();
        if new_slab.try_reserve_exact(self.max_bytes_cap).is_ok() {
            new_slab.resize(self.max_bytes_cap, 0);
            commit_pages(&mut new_slab);
            self.slab = new_slab;
        } else {
            self.slab = vec![0_u8; self.max_bytes_cap];
            commit_pages(&mut self.slab);
        }
    }

    fn evict_front(&mut self) {
        if let Some(removed) = self.packets.pop_front() {
            self.used_bytes = self.used_bytes.saturating_sub(removed.len);
        }
    }

    fn evict_by_time(&mut self) {
        if self.latest_pts_100ns == i64::MIN {
            return;
        }
        let cutoff = self.latest_pts_100ns.saturating_sub(self.max_duration_100ns);
        while let Some(front) = self.packets.front() {
            if front.pts_100ns < cutoff {
                self.evict_front();
            } else {
                break;
            }
        }
    }

    pub fn push(&mut self, packet: EncodedPacket) {
        let pkt_len = packet.data.len();
        if self.max_bytes_cap == 0 || pkt_len > self.max_bytes_cap {
            return;
        }
        if self.slab.len() != self.max_bytes_cap {
            self.preallocate();
        }
        let cap = self.slab.len();
        if cap == 0 || pkt_len > cap {
            return;
        }

        let offset = if pkt_len <= cap.saturating_sub(self.write_head) {
            let start = self.write_head;
            let end = start + pkt_len;
            while let Some(front) = self.packets.front() {
                if meta_intersects_range(front, start, end) {
                    self.evict_front();
                } else {
                    break;
                }
            }
            self.write_head = end;
            start
        } else {
            let old_wh = self.write_head;
            while let Some(front) = self.packets.front() {
                if meta_intersects_range(front, old_wh, cap)
                    || meta_intersects_range(front, 0, pkt_len)
                {
                    self.evict_front();
                } else {
                    break;
                }
            }
            self.write_head = pkt_len;
            0
        };

        if pkt_len > 0 {
            self.slab[offset..offset + pkt_len].copy_from_slice(&packet.data);
        }

        if self.latest_pts_100ns == i64::MIN || packet.pts_100ns > self.latest_pts_100ns {
            self.latest_pts_100ns = packet.pts_100ns;
        }
        self.used_bytes += pkt_len;
        self.packets.push_back(PacketMeta {
            track: packet.track,
            pts_100ns: packet.pts_100ns,
            dts_100ns: packet.dts_100ns,
            duration_100ns: packet.duration_100ns,
            is_idr_keyframe: packet.is_idr_keyframe,
            offset,
            len: pkt_len,
        });

        self.evict_by_time();
    }

    pub fn clear(&mut self) {
        self.packets.clear();
        self.used_bytes = 0;
        self.write_head = 0;
        self.latest_pts_100ns = i64::MIN;
        self.slab = Vec::new();
        self.slab.shrink_to_fit();
    }

    pub fn len(&self) -> usize {
        self.packets.len()
    }

    pub fn is_empty(&self) -> bool {
        self.packets.is_empty()
    }

    pub fn total_bytes(&self) -> usize {
        self.used_bytes
    }

    pub fn snapshot_from_nearest_idr(&self, window_secs: u32) -> Vec<EncodedPacket> {
        let Some(newest_pts) = self.packets.back().map(|p| p.pts_100ns) else {
            return Vec::new();
        };
        let window_100ns = (window_secs.max(1) as i64) * 10_000_000;
        let cutoff = newest_pts.saturating_sub(window_100ns);
        let anchor = self
            .packets
            .iter()
            .position(|p| p.pts_100ns >= cutoff)
            .unwrap_or(0);
        let start_idx = (0..=anchor)
            .rev()
            .find(|&i| {
                self.packets[i].track == PacketTrack::Video && self.packets[i].is_idr_keyframe
            })
            .or_else(|| {
                self.packets
                    .iter()
                    .position(|p| p.track == PacketTrack::Video && p.is_idr_keyframe)
            })
            .unwrap_or(anchor);

        let start_pts = self.packets[start_idx].pts_100ns;
        let mut out = Vec::with_capacity(self.packets.len().saturating_sub(start_idx) + 16);
        let push_meta = |meta: &PacketMeta, slab: &[u8], dst: &mut Vec<EncodedPacket>| {
            let end = meta.offset + meta.len;
            let data = if meta.len > 0 && end <= slab.len() {
                slab[meta.offset..end].to_vec()
            } else {
                Vec::new()
            };
            dst.push(EncodedPacket {
                track: meta.track,
                pts_100ns: meta.pts_100ns,
                dts_100ns: meta.dts_100ns,
                duration_100ns: meta.duration_100ns,
                is_idr_keyframe: meta.is_idr_keyframe,
                data,
            });
        };
        push_meta(&self.packets[start_idx], &self.slab, &mut out);
        for meta in self.packets.iter().take(start_idx) {
            if meta.track != PacketTrack::Video
                && meta.pts_100ns.saturating_add(meta.duration_100ns) > start_pts
            {
                push_meta(meta, &self.slab, &mut out);
            }
        }
        for meta in self.packets.iter().skip(start_idx + 1) {
            push_meta(meta, &self.slab, &mut out);
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mk_vid(pts_sec: i64, idr: bool, bytes: usize) -> EncodedPacket {
        EncodedPacket {
            track: PacketTrack::Video,
            pts_100ns: pts_sec * 10_000_000,
            dts_100ns: pts_sec * 10_000_000,
            duration_100ns: 166_666,
            is_idr_keyframe: idr,
            data: vec![if idr { 0x65 } else { 0x41 }; bytes],
        }
    }

    #[test]
    fn snapshot_snaps_back_to_nearest_preceding_idr() {
        let mut rb = CompressedRingBuffer::new(30, 64 * 1024 * 1024);
        rb.preallocate();
        for s in 0..=10 {
            rb.push(EncodedPacket {
                track: PacketTrack::GameAudio,
                pts_100ns: s * 10_000_000,
                dts_100ns: s * 10_000_000,
                duration_100ns: 200_000,
                is_idr_keyframe: true,
                data: vec![0xAA; 256],
            });
            rb.push(mk_vid(s, s % 3 == 0, 1024));
        }

        let snap = rb.snapshot_from_nearest_idr(5);
        assert!(!snap.is_empty());
        assert_eq!(snap[0].track, PacketTrack::Video);
        assert!(snap[0].is_idr_keyframe);
        assert!(snap[0].pts_100ns <= 5 * 10_000_000);
        let last_vid = snap.iter().rev().find(|p| p.track == PacketTrack::Video).unwrap();
        assert_eq!(last_vid.pts_100ns, 10 * 10_000_000);
    }

    #[test]
    fn clear_releases_all_memory() {
        let mut rb = CompressedRingBuffer::new(30, 16 * 1024 * 1024);
        rb.preallocate();
        rb.push(mk_vid(0, true, 4096));
        assert!(!rb.is_empty());
        assert_eq!(rb.total_bytes(), 4096);
        rb.clear();
        assert!(rb.is_empty());
        assert_eq!(rb.total_bytes(), 0);
        assert_eq!(rb.slab.capacity(), 0);
    }

    #[test]
    fn circular_slab_wraps_and_evicts_without_corruption() {
        let mut rb = CompressedRingBuffer::new(120, 4096);
        rb.preallocate();
        assert_eq!(rb.slab.len(), 4096);

        for i in 0_i64..20 {
            let tag = ((i % 200) + 1) as u8;
            rb.push(EncodedPacket {
                track: PacketTrack::Video,
                pts_100ns: i * 1_000_000,
                dts_100ns: i * 1_000_000,
                duration_100ns: 1_000_000,
                is_idr_keyframe: i % 2 == 0,
                data: vec![tag; 900],
            });
        }

        assert!(rb.total_bytes() <= 4096);
        let snap = rb.snapshot_from_nearest_idr(10);
        assert!(!snap.is_empty());
        assert!(snap[0].is_idr_keyframe);
        for pkt in &snap {
            let expected_tag = (((pkt.pts_100ns / 1_000_000) % 200) + 1) as u8;
            assert_eq!(pkt.data.len(), 900);
            assert!(pkt.data.iter().all(|&b| b == expected_tag));
        }
    }
}
