//! CPU capture publication and its retained teardown owner. Porthole supplies
//! authorized bytes; Jackstay owns admission, mappings and acquired storage.

use std::{
    collections::BTreeMap,
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use jackstay::acquisition::arena::{ArenaConfig, ArenaProducer, FrameDescriptor, PublishOutcome, ReconfigurationStatus};

use super::CaptureRegistryError;

const MEMORY_BUDGET: u64 = 512 * 1024 * 1024;
const DRAIN_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_CONNECTIONS: usize = 4;
pub(super) type Producer = Arc<Mutex<ArenaProducer>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Format {
    width: u32,
    height: u32,
    stride: u32,
    pixel_format: u32,
    color_space: u32,
    clock_domain: u32,
    bytes: usize,
}

impl Format {
    fn from_frame(frame: &FrameDescriptor, bytes: &[u8]) -> Result<Self, CaptureRegistryError> {
        let min_stride = frame.width.checked_mul(4).ok_or_else(|| failure("CPU width overflow"))?;
        let expected = (frame.stride as usize)
            .checked_mul(frame.height as usize)
            .ok_or_else(|| failure("CPU frame size overflow"))?;
        if frame.width == 0 || frame.height == 0 || frame.stride < min_stride || expected != bytes.len() {
            return Err(failure("CPU frame dimensions, stride and payload disagree"));
        }
        Ok(Self {
            width: frame.width,
            height: frame.height,
            stride: frame.stride,
            pixel_format: frame.pixel_format,
            color_space: frame.color_space,
            clock_domain: frame.clock_domain,
            bytes: bytes.len(),
        })
    }
}

pub(super) struct CpuSession {
    state: Arc<Mutex<Runtime>>,
}

#[derive(Default)]
struct Runtime {
    memory_budget: u64,
    producer: Option<Producer>,
    installed: Option<Format>,
    pending: Option<Format>,
    pause: Option<(u64, u64)>,
    error: Option<String>,
    cleanup_error: Option<String>,
    closing: Option<Instant>,
    drained: bool,
    published: u64,
    dropped: u64,
    next_connection: u64,
    connections: BTreeMap<u64, UnixStream>,
}

pub(super) struct Snapshot {
    pub status: &'static str,
    pub message: String,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub pixel_format: jackstay::model::PixelFormat,
}

impl CpuSession {
    pub fn new() -> Result<Self, CaptureRegistryError> {
        Self::with_budget(MEMORY_BUDGET)
    }

    fn with_budget(memory_budget: u64) -> Result<Self, CaptureRegistryError> {
        let state = Arc::new(Mutex::new(Runtime {
            memory_budget,
            ..Runtime::default()
        }));
        let worker = state.clone();
        // This owner outlives a dropped registry/session until actual retirement.
        // It owns no registry reference and never joins a capture callback queue.
        std::thread::Builder::new()
            .name("cpu-capture-retirement".to_owned())
            .spawn(move || {
                loop {
                    std::thread::sleep(Duration::from_millis(100));
                    let mut state = worker.lock().expect("CPU capture runtime poisoned");
                    state.maintain();
                    if state.drained {
                        break;
                    }
                }
            })
            .map_err(failure)?;
        Ok(Self { state })
    }

    pub fn publish(&self, descriptor: FrameDescriptor, bytes: &[u8]) -> Result<PublishOutcome, CaptureRegistryError> {
        let mut state = self.state.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        let result = state.publish(descriptor, bytes);
        if let Err(error) = &result {
            state.error = Some(error.to_string());
            if let Some(producer) = &state.producer {
                producer.lock().expect("CPU producer poisoned").stop();
            }
        }
        result
    }

    pub fn stop(&self) {
        self.state.lock().expect("CPU capture runtime poisoned").stop();
    }

    pub fn fail(&self, message: String) {
        let mut state = self.state.lock().expect("CPU capture runtime poisoned");
        state.error.get_or_insert(message);
        state.stop();
    }

    pub fn snapshot(&self) -> Snapshot {
        self.state.lock().expect("CPU capture runtime poisoned").snapshot()
    }

    pub fn open_connection(&self, stream: &UnixStream) -> Result<(Producer, Connection), CaptureRegistryError> {
        let mut state = self.state.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        if state.closing.is_some() || state.error.is_some() {
            return Err(failure("CPU capture is closed"));
        }
        if state.connections.len() >= MAX_CONNECTIONS {
            return Err(failure("CPU setup connection limit reached"));
        }
        let producer = state
            .producer
            .as_ref()
            .cloned()
            .ok_or_else(|| failure("CPU capture has no first frame"))?;
        let stream = stream.try_clone().map_err(failure)?;
        let id = state
            .next_connection
            .checked_add(1)
            .ok_or_else(|| failure("CPU setup connection identities exhausted"))?;
        state.next_connection = id;
        state.connections.insert(id, stream);
        Ok((
            producer,
            Connection {
                state: self.state.clone(),
                id,
            },
        ))
    }
}

impl Drop for CpuSession {
    fn drop(&mut self) {
        self.stop();
    }
}

pub(super) struct Connection {
    state: Arc<Mutex<Runtime>>,
    id: u64,
}

impl Drop for Connection {
    fn drop(&mut self) {
        if let Ok(mut state) = self.state.lock() {
            state.connections.remove(&self.id);
        }
    }
}

impl Runtime {
    fn stop(&mut self) {
        if self.closing.is_none() {
            self.closing = Some(Instant::now());
        }
        if let Some(producer) = &self.producer {
            producer.lock().expect("CPU producer poisoned").stop();
        }
        for stream in self.connections.values() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }

    fn publish(&mut self, descriptor: FrameDescriptor, bytes: &[u8]) -> Result<PublishOutcome, CaptureRegistryError> {
        if self.closing.is_some() || self.error.is_some() {
            return Err(failure("CPU capture publication is closed"));
        }
        let format = Format::from_frame(&descriptor, bytes)?;
        if self.producer.is_none() {
            let producer = ArenaProducer::new(ArenaConfig {
                resource_capacity: 8,
                retained_history: 2,
                producer_reserve: 1,
                payload_capacity: bytes.len(),
                memory_budget: self.memory_budget,
                max_incarnations: MAX_CONNECTIONS as u32,
                drain_timeout: DRAIN_TIMEOUT,
            })
            .map_err(failure)?;
            self.producer = Some(Arc::new(Mutex::new(producer)));
            self.installed = Some(format);
        }
        self.maintain();
        if let Some(error) = &self.error {
            return Err(failure(error));
        }
        let producer = self.producer.as_ref().expect("CPU producer initialized").clone();
        let mut producer = producer.lock().map_err(|_| CaptureRegistryError::Poisoned)?;
        if self.pending.is_none() && self.installed != Some(format) {
            match producer.reconfigure_cpu(format.bytes).map_err(failure)? {
                ReconfigurationStatus::Ready { .. } => {
                    self.installed = Some(format);
                    self.pause = None;
                }
                ReconfigurationStatus::PausedCapacity { requested, available } => {
                    self.pending = Some(format);
                    self.pause = Some((requested, available));
                }
            }
        }
        let result = producer.publish(descriptor, bytes).map_err(failure)?;
        match result {
            PublishOutcome::Published { .. } => self.published = self.published.saturating_add(1),
            PublishOutcome::Dropped => self.dropped = self.dropped.saturating_add(1),
        }
        Ok(result)
    }

    fn maintain(&mut self) {
        let Some(producer) = self.producer.as_ref().cloned() else {
            self.drained = self.closing.is_some();
            return;
        };
        let mut guard = producer.lock().expect("CPU producer poisoned");
        self.cleanup_error = match guard.poll_cleanup() {
            Err(error) => Some(error.to_string()),
            Ok(_) => {
                let failures = guard.cleanup_failures();
                (!failures.is_empty()).then(|| {
                    failures
                        .into_iter()
                        .map(|failure| format!("{:?}: {}", failure.incarnation, failure.reason))
                        .collect::<Vec<_>>()
                        .join("; ")
                })
            }
        };
        if self.closing.is_none() && self.error.is_none() && self.pending.is_some() {
            match guard.advance_reconfiguration() {
                Ok(ReconfigurationStatus::Ready { .. }) => {
                    self.installed = self.pending.take();
                    self.pause = None;
                }
                Ok(ReconfigurationStatus::PausedCapacity { requested, available }) => self.pause = Some((requested, available)),
                Err(error) => {
                    self.error = Some(error.to_string());
                    guard.stop();
                }
            }
        }
        if let Some(started) = self.closing {
            let ready = match guard.poll_shutdown_ready() {
                Ok(ready) => ready,
                Err(error) => {
                    self.cleanup_error = Some(error.to_string());
                    false
                }
            };
            drop(guard);
            if ready && Arc::strong_count(&producer) == 2 && self.connections.is_empty() {
                drop(self.producer.take());
                drop(producer);
                self.drained = true;
                self.cleanup_error = None;
            } else if started.elapsed() >= DRAIN_TIMEOUT && self.cleanup_error.is_none() {
                self.cleanup_error = Some("CPU shutdown needs recovery: mappings or setup owners have not retired".to_owned());
            }
        }
    }

    fn snapshot(&self) -> Snapshot {
        let (status, detail) = if self.drained && self.error.is_some() {
            (
                "failed",
                format!("{}; CPU resources retired", self.error.as_deref().expect("checked")),
            )
        } else if self.drained {
            ("closed", "CPU resources retired".to_owned())
        } else if let Some(error) = &self.cleanup_error {
            ("recovery_required", error.clone())
        } else if let Some(error) = &self.error {
            ("failed", error.clone())
        } else if self.closing.is_some() {
            ("draining", "waiting for CPU mappings and claims".to_owned())
        } else if let Some((requested, available)) = self.pause {
            ("paused", format!("replacement needs {requested} bytes; {available} available"))
        } else if self.installed.is_some() {
            ("ready", String::new())
        } else {
            ("starting", String::new())
        };
        Snapshot {
            status,
            message: format!(
                "{detail}; CPU budget={} bytes, published={}, dropped={}",
                self.memory_budget, self.published, self.dropped
            ),
            width: self.installed.map_or(0, |format| format.width),
            height: self.installed.map_or(0, |format| format.height),
            stride: self.installed.map_or(0, |format| format.stride),
            pixel_format: self.installed.map_or(jackstay::model::PixelFormat::Unknown, |format| {
                use jackstay::model::PixelFormat;
                match format.pixel_format {
                    value if value == PixelFormat::Bgra8Unorm as u32 => PixelFormat::Bgra8Unorm,
                    value if value == PixelFormat::Rgba8Unorm as u32 => PixelFormat::Rgba8Unorm,
                    _ => PixelFormat::Unknown,
                }
            }),
        }
    }
}

fn failure(error: impl std::fmt::Display) -> CaptureRegistryError {
    CaptureRegistryError::Capture(error.to_string())
}

impl std::fmt::Debug for CpuSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CpuSession")
            .field("status", &self.snapshot().status)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use jackstay::acquisition::arena::{AcquireOutcome, ArenaConsumer};

    use super::*;

    fn descriptor(len: usize) -> FrameDescriptor {
        FrameDescriptor {
            width: 1,
            height: (len / 4) as u32,
            stride: 4,
            pixel_format: jackstay::model::PixelFormat::Bgra8Unorm as u32,
            ..FrameDescriptor::default()
        }
    }

    fn wait_status(session: &CpuSession, expected: &str) {
        let deadline = Instant::now() + Duration::from_secs(2);
        while session.snapshot().status != expected {
            assert!(Instant::now() < deadline, "expected {expected}, got {}", session.snapshot().message);
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn idle_maintenance_resumes_a_paused_resize_after_old_lease_retirement() {
        let session = CpuSession::with_budget(768 * 1024).unwrap();
        session.publish(descriptor(32 * 1024), &vec![7; 32 * 1024]).unwrap();
        let (stream, _peer) = UnixStream::pair().unwrap();
        let (producer, connection) = session.open_connection(&stream).unwrap();
        let mut consumer = ArenaConsumer::from_grant(producer.lock().unwrap().attach(1).unwrap()).unwrap();
        let AcquireOutcome::Frame(old) = consumer.acquire_latest(0).unwrap() else {
            panic!("missing old frame")
        };
        assert_eq!(
            session.publish(descriptor(64 * 1024), &vec![8; 64 * 1024]).unwrap(),
            PublishOutcome::Dropped
        );
        assert_eq!(session.snapshot().status, "paused");
        assert_eq!(session.snapshot().height, 8192, "pending dimensions were advertised as installed");
        consumer.relinquish_configuration();
        assert_eq!(old.bytes(), vec![7; 32 * 1024]);
        drop(old);
        wait_status(&session, "ready");
        assert_eq!(session.snapshot().height, 16384);
        let offer = producer
            .lock()
            .unwrap()
            .configuration_offer(consumer.incarnation())
            .unwrap()
            .unwrap();
        consumer.install_configuration(offer).unwrap();
        assert!(matches!(
            session.publish(descriptor(64 * 1024), &vec![8; 64 * 1024]).unwrap(),
            PublishOutcome::Published { .. }
        ));
        drop(consumer);
        drop(producer);
        drop(connection);
        session.stop();
        wait_status(&session, "closed");
    }

    #[test]
    fn closing_retains_resources_until_frames_mappings_and_setup_owners_retire() {
        let session = CpuSession::new().unwrap();
        session.publish(descriptor(4), b"held").unwrap();
        let (stream, _peer) = UnixStream::pair().unwrap();
        let (producer, connection) = session.open_connection(&stream).unwrap();
        let consumer = ArenaConsumer::from_grant(producer.lock().unwrap().attach(1).unwrap()).unwrap();
        let AcquireOutcome::Frame(held) = consumer.acquire_latest(0).unwrap() else {
            panic!("missing frame")
        };
        session.stop();
        assert!(matches!(consumer.acquire_latest(0).unwrap(), AcquireOutcome::Closed));
        drop(consumer);
        session.state.lock().unwrap().maintain();
        assert_eq!(session.snapshot().status, "draining");
        assert_eq!(held.bytes(), b"held");
        drop(held);
        session.state.lock().unwrap().maintain();
        assert_eq!(session.snapshot().status, "draining", "setup owner was still alive");
        drop(producer);
        drop(connection);
        wait_status(&session, "closed");
    }
}
