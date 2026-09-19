//! In-process bridge halves. A handle owns cancellation, status and teardown.
use std::{
    io,
    os::unix::{
        fs::PermissionsExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use crate::{
    egress, ingress,
    worker::{EgressSpec, HalfStatus, IngressSpec, Phase},
};

type State = Arc<(Mutex<HalfStatus>, Condvar)>;
fn update(state: &State, change: impl FnOnce(&mut HalfStatus)) {
    change(&mut state.0.lock().expect("bridge status"));
    state.1.notify_all();
}

struct Listener {
    socket: UnixListener,
    path: PathBuf,
}
impl Listener {
    fn bind(path: PathBuf) -> io::Result<Self> {
        let socket = UnixListener::bind(&path)?;
        let listener = Self { socket, path };
        std::fs::set_permissions(&listener.path, std::fs::Permissions::from_mode(0o600))?;
        listener.socket.set_nonblocking(true)?;
        Ok(listener)
    }
    fn accept(&self, stop: &AtomicBool) -> io::Result<UnixStream> {
        while !stop.load(Ordering::Acquire) {
            match self.socket.accept() {
                Ok((stream, _)) => {
                    stream.set_nonblocking(false)?;
                    return Ok(stream);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => thread::sleep(Duration::from_millis(10)),
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(io::ErrorKind::Interrupted, "bridge stopped"))
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

pub struct Task {
    stop: Arc<AtomicBool>,
    cancelled: Arc<AtomicBool>,
    state: State,
    thread: Option<JoinHandle<()>>,
}
impl Task {
    fn spawn(run: impl FnOnce(Arc<AtomicBool>, State) -> Result<serde_json::Value, String> + Send + 'static) -> io::Result<Self> {
        let state = Arc::new((
            Mutex::new(HalfStatus {
                phase: Some(Phase::Starting),
                ..HalfStatus::default()
            }),
            Condvar::new(),
        ));
        let stop = Arc::new(AtomicBool::new(false));
        let cancelled = Arc::new(AtomicBool::new(false));
        let thread = {
            let state = state.clone();
            let stop = stop.clone();
            let cancelled = cancelled.clone();
            thread::Builder::new().name("porthole-bridge".into()).spawn(move || {
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| run(stop.clone(), state.clone())))
                    .unwrap_or_else(|_| Err("bridge task panicked".into()));
                stop.store(true, Ordering::Release);
                update(&state, |status| {
                    status.phase = Some(Phase::Ended);
                    match result {
                        Ok(report) => {
                            status.report = Some(report);
                            status.exit_code = Some(0);
                        }
                        Err(_) if cancelled.load(Ordering::Acquire) => status.exit_code = Some(0),
                        Err(error) => {
                            status.failure = Some(error);
                            status.exit_code = Some(1);
                        }
                    }
                });
            })?
        };
        Ok(Self {
            stop,
            cancelled,
            state,
            thread: Some(thread),
        })
    }

    pub fn egress(spec: EgressSpec) -> io::Result<Self> {
        let media = Listener::bind(spec.media_socket.clone())?;
        let control = Listener::bind(spec.control_socket.clone())?;
        Self::spawn(move |stop, state| {
            update(&state, |s| s.phase = Some(Phase::Listening));
            let media_stream = media.accept(&stop).map_err(|e| e.to_string())?;
            let control_stream = control.accept(&stop).map_err(|e| e.to_string())?;
            let mut config = egress::EgressConfig {
                chroma_policy: spec.chroma,
                token: spec.link_token,
                input_socket: spec.input_socket,
                ..Default::default()
            };
            if let Some(bitrate) = spec.bitrate_bps {
                config.bitrate_bps = bitrate;
            }
            let report = egress::run(
                egress::Source::Named {
                    service: spec.source_service,
                    token: spec.source_token,
                },
                media_stream,
                control_stream,
                config,
                stop,
                Box::new(move |decision| {
                    update(&state, |s| {
                        s.phase = Some(Phase::Running);
                        s.decision = Some(decision.clone());
                    })
                }),
            )
            .map_err(|e| e.to_string())?;
            serde_json::to_value(report).map_err(|e| e.to_string())
        })
    }

    pub fn ingress(spec: IngressSpec) -> io::Result<Self> {
        Self::spawn(move |stop, state| {
            let media = UnixStream::connect(&spec.media_socket).map_err(|e| e.to_string())?;
            let control = UnixStream::connect(&spec.control_socket).map_err(|e| e.to_string())?;
            let publication = (spec.service.clone(), spec.viewer_token.clone());
            let cpu_socket = spec.cpu_socket.as_ref().map(|p| p.to_string_lossy().into_owned());
            let input_socket = spec.input_socket.as_ref().map(|p| p.to_string_lossy().into_owned());
            let report = ingress::run(
                ingress::Publish::Named {
                    service: spec.service,
                    token: spec.viewer_token,
                },
                media,
                control,
                ingress::IngressConfig {
                    chroma_policy: spec.chroma,
                    token: spec.link_token,
                    cpu_socket: spec.cpu_socket,
                    input_socket: spec.input_socket,
                    ..Default::default()
                },
                stop,
                Box::new(move |_| {
                    update(&state, |s| {
                        s.phase = Some(Phase::Running);
                        s.publication = Some(publication);
                        s.cpu_socket = cpu_socket;
                        s.input_socket = input_socket;
                    })
                }),
            )
            .map_err(|e| e.to_string())?;
            serde_json::to_value(report).map_err(|e| e.to_string())
        })
    }

    pub fn status(&self) -> HalfStatus {
        self.state.0.lock().expect("bridge status").clone()
    }
    pub fn wait_for(&self, ready: impl Fn(&HalfStatus) -> bool, timeout: Duration) -> HalfStatus {
        let deadline = Instant::now() + timeout;
        let mut status = self.state.0.lock().expect("bridge status");
        while !ready(&status) && status.phase != Some(Phase::Ended) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break;
            }
            status = self.state.1.wait_timeout(status, remaining).expect("bridge status").0;
        }
        status.clone()
    }
    pub fn stop(&mut self) -> HalfStatus {
        self.cancelled.store(true, Ordering::Release);
        self.stop.store(true, Ordering::Release);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
        self.status()
    }
}
impl Drop for Task {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ingress_stop_interrupts_a_partial_hello_and_failed_hello_ends_the_task() {
        use std::io::Write;
        for malformed in [false, true] {
            let root = tempfile::tempdir_in("/tmp").unwrap();
            let media = UnixListener::bind(root.path().join("m")).unwrap();
            let control = UnixListener::bind(root.path().join("c")).unwrap();
            let mut task = Task::ingress(IngressSpec {
                media_socket: root.path().join("m"),
                control_socket: root.path().join("c"),
                service: "unused-before-hello".into(),
                viewer_token: Some("viewer".into()),
                link_token: "link".into(),
                chroma: Default::default(),
                cpu_socket: None,
                input_socket: None,
            })
            .unwrap();
            let _media = media.accept().unwrap().0;
            let mut control = control.accept().unwrap().0;
            if malformed {
                crate::wire::Message::new(crate::wire::Kind::Frame, 0, vec![])
                    .write_to(&mut control)
                    .unwrap();
                let status = task.wait_for(|_| false, Duration::from_secs(5));
                assert_eq!(status.phase, Some(Phase::Ended));
                assert!(status.failure.as_deref().unwrap().contains("hello"));
            } else {
                control.write_all(&[1]).unwrap();
            }
            let (done, finished) = std::sync::mpsc::channel();
            std::thread::spawn(move || {
                done.send(task.stop()).unwrap();
            });
            assert_eq!(finished.recv_timeout(Duration::from_secs(5)).unwrap().phase, Some(Phase::Ended));
        }
    }
    #[test]
    fn stop_before_peer_arrives_removes_owned_sockets_and_can_restart() {
        let root = tempfile::tempdir_in("/tmp").unwrap();
        for partial_peer in [false, true] {
            let spec = EgressSpec {
                source_service: "unused".into(),
                source_token: None,
                media_socket: root.path().join("m"),
                control_socket: root.path().join("c"),
                link_token: "test".into(),
                chroma: Default::default(),
                bitrate_bps: None,
                input_socket: None,
            };
            let mut task = Task::egress(spec.clone()).unwrap();
            assert_eq!(
                task.wait_for(|s| s.phase == Some(Phase::Listening), Duration::from_secs(1)).phase,
                Some(Phase::Listening)
            );
            let _peer = partial_peer.then(|| UnixStream::connect(&spec.media_socket).unwrap());
            assert_eq!(task.stop().phase, Some(Phase::Ended));
            assert!(!spec.media_socket.exists());
            assert!(!spec.control_socket.exists());
            assert_eq!(task.stop().exit_code, Some(0));
        }
    }
}
