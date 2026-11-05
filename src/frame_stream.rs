use serde::Serialize;
use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc, Mutex,
};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::sync::watch;

const LEASE_MS: u64 = 2_000;
const MAX_PLANE_BYTES: usize = 2 * 1024 * 1024;

pub type Nv12Receiver = watch::Receiver<Option<Arc<Vec<u8>>>>;

#[derive(Default)]
struct PublishState {
    next_copy_ms: u64,
    sequence: u64,
}

pub struct Nv12Stream {
    started: Instant,
    active: AtomicBool,
    client_busy: AtomicBool,
    requested_until: AtomicU64,
    interval_ms: AtomicU64,
    publish: Mutex<PublishState>,
    sender: watch::Sender<Option<Arc<Vec<u8>>>>,
}

impl Default for Nv12Stream {
    fn default() -> Self {
        let (sender, _) = watch::channel(None);
        Self {
            started: Instant::now(),
            active: AtomicBool::new(false),
            client_busy: AtomicBool::new(false),
            requested_until: AtomicU64::new(0),
            interval_ms: AtomicU64::new(100),
            publish: Mutex::new(PublishState::default()),
            sender,
        }
    }
}

pub struct StreamLease {
    stream: Arc<Nv12Stream>,
}

impl StreamLease {
    pub fn renew(&self) {
        self.renew_at(self.stream.elapsed_ms());
    }

    fn renew_at(&self, now: u64) {
        if self.stream.active.load(Ordering::Acquire) {
            self.stream
                .requested_until
                .store(now.saturating_add(LEASE_MS), Ordering::Release);
        }
    }
}

impl Drop for StreamLease {
    fn drop(&mut self) {
        let mut publish = self.stream.publish.lock().unwrap();
        self.stream.active.store(false, Ordering::Release);
        self.stream.requested_until.store(0, Ordering::Release);
        self.stream.sender.send_replace(None);
        publish.next_copy_ms = 0;
        // Release admission last so this lease cannot clear its successor's packet.
        self.stream.client_busy.store(false, Ordering::Release);
    }
}

impl Nv12Stream {
    fn elapsed_ms(&self) -> u64 {
        self.started.elapsed().as_millis().min(u64::MAX as u128) as u64
    }

    pub fn subscribe(
        self: &Arc<Self>,
        fps: u32,
    ) -> Result<(Nv12Receiver, StreamLease), &'static str> {
        if !(1..=15).contains(&fps) {
            return Err("NV12 source rate must be between 1 and 15 fps.");
        }
        if self
            .client_busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err("An NV12 stream client is already connected.");
        }
        let mut publish = self.publish.lock().unwrap();
        publish.next_copy_ms = 0;
        self.sender.send_replace(None);
        self.interval_ms
            .store(1_000_u64.div_ceil(u64::from(fps)), Ordering::Release);
        self.active.store(true, Ordering::Release);
        let lease = StreamLease {
            stream: Arc::clone(self),
        };
        lease.renew();
        Ok((self.sender.subscribe(), lease))
    }

    pub fn clear(&self) {
        let mut publish = self.publish.lock().unwrap();
        self.active.store(false, Ordering::Release);
        self.requested_until.store(0, Ordering::Release);
        self.sender.send_replace(None);
        publish.next_copy_ms = 0;
    }

    pub fn publish(&self, width: u32, height: u32, stride: u32, y: &[u8], uv: &[u8]) {
        self.publish_at(width, height, stride, y, uv, self.elapsed_ms());
    }

    fn publish_at(&self, width: u32, height: u32, stride: u32, y: &[u8], uv: &[u8], now: u64) {
        if !self.active.load(Ordering::Acquire)
            || now > self.requested_until.load(Ordering::Acquire)
            || self.sender.receiver_count() == 0
        {
            return;
        }
        let Some((width, height, stride)) = plane_layout(width, height, stride, y.len(), uv.len())
        else {
            return;
        };
        let Ok(mut publish) = self.publish.try_lock() else {
            return;
        };
        if !self.active.load(Ordering::Acquire)
            || now > self.requested_until.load(Ordering::Acquire)
            || now < publish.next_copy_ms
        {
            return;
        }
        let Some(sequence) = publish.sequence.checked_add(1) else {
            return;
        };
        #[derive(Serialize)]
        struct Header {
            version: u8,
            format: &'static str,
            width: usize,
            height: usize,
            stride_y: usize,
            stride_uv: usize,
            y_bytes: usize,
            uv_bytes: usize,
            captured_unix_ms: u64,
            sequence: u64,
            color_matrix: &'static str,
            color_range: &'static str,
        }
        let captured_unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| duration.as_millis().try_into().ok())
            .unwrap_or_default();
        let Ok(header) = serde_json::to_vec(&Header {
            version: 1,
            format: "nv12",
            width,
            height,
            stride_y: width,
            stride_uv: width,
            y_bytes: width * height,
            uv_bytes: width * height / 2,
            captured_unix_ms,
            sequence,
            color_matrix: "bt601",
            color_range: "limited",
        }) else {
            return;
        };
        let mut packet = Vec::with_capacity(4 + header.len() + width * height * 3 / 2);
        packet.extend_from_slice(&(header.len() as u32).to_be_bytes());
        packet.extend_from_slice(&header);
        for plane in [
            y.chunks(stride).take(height),
            uv.chunks(stride).take(height / 2),
        ] {
            for row in plane {
                packet.extend_from_slice(&row[..width]);
            }
        }
        publish.sequence = sequence;
        publish.next_copy_ms = now.saturating_add(self.interval_ms.load(Ordering::Acquire));
        self.sender.send_replace(Some(Arc::new(packet)));
    }
}

fn plane_layout(
    width: u32,
    height: u32,
    stride: u32,
    y_len: usize,
    uv_len: usize,
) -> Option<(usize, usize, usize)> {
    if width == 0
        || height == 0
        || width > 640
        || height > 360
        || !width.is_multiple_of(2)
        || !height.is_multiple_of(2)
        || stride < width
        || stride > 4_096
    {
        return None;
    }
    let (width, height, stride) = (width as usize, height as usize, stride as usize);
    let y_required = (height - 1).checked_mul(stride)?.checked_add(width)?;
    let uv_required = (height / 2 - 1).checked_mul(stride)?.checked_add(width)?;
    if y_required > y_len
        || uv_required > uv_len
        || y_required.checked_add(uv_required)? > MAX_PLANE_BYTES
    {
        return None;
    }
    Some((width, height, stride))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn packet(receiver: &Nv12Receiver) -> (serde_json::Value, Vec<u8>) {
        let packet = receiver.borrow().as_ref().unwrap().clone();
        let length = u32::from_be_bytes(packet[..4].try_into().unwrap()) as usize;
        (
            serde_json::from_slice(&packet[4..4 + length]).unwrap(),
            packet[4 + length..].to_vec(),
        )
    }

    #[test]
    fn packs_strided_planes_with_explicit_metadata_and_rejects_invalid_buffers() {
        let stream = Arc::new(Nv12Stream::default());
        let (receiver, _lease) = stream.subscribe(10).unwrap();
        let y = [1, 2, 90, 3, 4];
        let uv = [5, 6];
        stream.publish_at(2, 2, 3, &y, &uv, 1);
        let (header, pixels) = packet(&receiver);
        assert_eq!(pixels, [1, 2, 3, 4, 5, 6]);
        for (field, value) in [
            ("version", 1),
            ("width", 2),
            ("height", 2),
            ("stride_y", 2),
            ("stride_uv", 2),
            ("y_bytes", 4),
            ("uv_bytes", 2),
            ("sequence", 1),
        ] {
            assert_eq!(header[field], value);
        }
        assert_eq!(header["format"], "nv12");
        assert_eq!(header["color_matrix"], "bt601");
        assert_eq!(header["color_range"], "limited");
        assert!(header["captured_unix_ms"].as_u64().unwrap() > 0);
        for (w, h, s, y_len, uv_len) in [
            (0, 2, 2, 4, 2),
            (3, 2, 3, 6, 3),
            (2, 3, 2, 6, 2),
            (642, 2, 642, 1284, 642),
            (2, 362, 2, 724, 362),
            (2, 2, 1, 4, 2),
            (2, 2, 4097, 4, 2),
            (2, 2, 3, 4, 2),
            (2, 2, 3, 5, 1),
            (640, 360, 4096, usize::MAX, usize::MAX),
        ] {
            assert!(plane_layout(w, h, s, y_len, uv_len).is_none());
        }
        assert_eq!(
            plane_layout(640, 360, 640, 230400, 115200),
            Some((640, 360, 640))
        );
    }

    #[test]
    fn copies_only_on_demand_with_global_throttle_and_expiring_lease() {
        let stream = Arc::new(Nv12Stream::default());
        stream.publish_at(2, 2, 2, &[1; 4], &[2; 2], 0);
        assert!(stream.sender.borrow().is_none());
        let (receiver, lease) = stream.subscribe(10).unwrap();
        lease.renew_at(0);
        stream.publish_at(2, 2, 2, &[1; 4], &[2; 2], 1);
        stream.publish_at(2, 2, 2, &[3; 4], &[4; 2], 100);
        assert_eq!(packet(&receiver).0["sequence"], 1);
        stream.publish_at(2, 2, 2, &[3; 4], &[4; 2], 101);
        assert_eq!(packet(&receiver).0["sequence"], 2);
        stream.publish_at(2, 2, 2, &[5; 4], &[6; 2], 2001);
        assert_eq!(packet(&receiver).0["sequence"], 2);
        lease.renew_at(2002);
        stream.publish_at(2, 2, 2, &[5; 4], &[6; 2], 2003);
        assert_eq!(packet(&receiver).0["sequence"], 3);
        let held = stream.publish.lock().unwrap();
        stream.publish_at(2, 2, 2, &[7; 4], &[8; 2], 2200);
        assert_eq!(packet(&receiver).0["sequence"], 3);
        drop(held);
    }

    #[test]
    fn one_client_clear_and_drop_prevent_stale_or_cross_session_output() {
        let stream = Arc::new(Nv12Stream::default());
        assert!(stream.subscribe(0).is_err());
        assert!(stream.subscribe(16).is_err());
        let (receiver, lease) = stream.subscribe(15).unwrap();
        assert!(stream.subscribe(15).is_err());
        stream.publish_at(2, 2, 2, &[1; 4], &[2; 2], 1);
        assert_eq!(packet(&receiver).0["sequence"], 1);
        stream.clear();
        lease.renew_at(10);
        stream.publish_at(2, 2, 2, &[1; 4], &[2; 2], 11);
        assert!(receiver.borrow().is_none());
        assert!(stream.subscribe(15).is_err());
        drop(lease);
        let (next, next_lease) = stream.subscribe(15).unwrap();
        stream.publish_at(2, 2, 2, &[3; 4], &[4; 2], 1);
        assert_eq!(packet(&next).0["sequence"], 2);
        drop(next);
        drop(receiver);
        stream.publish_at(2, 2, 2, &[5; 4], &[6; 2], 100);
        assert_eq!(stream.publish.lock().unwrap().sequence, 2);
        drop(next_lease);
        assert!(stream.sender.borrow().is_none());
        assert!(!stream.client_busy.load(Ordering::Acquire));
    }
}
