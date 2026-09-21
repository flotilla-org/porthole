//! Stop blocking link I/O and join owned threads on every return path.
use std::{
    io,
    net::Shutdown,
    os::unix::net::UnixStream,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

pub(crate) struct RunScope {
    stop: Arc<AtomicBool>,
    sockets: Vec<UnixStream>,
    hooks: Vec<Box<dyn FnOnce()>>,
    threads: Vec<JoinHandle<()>>,
}
impl RunScope {
    pub(crate) fn new(stop: Arc<AtomicBool>, media: &UnixStream, control: &UnixStream) -> io::Result<Self> {
        let sockets = vec![media.try_clone()?, control.try_clone()?];
        let wake = vec![media.try_clone()?, control.try_clone()?];
        let flag = stop.clone();
        let watcher = thread::Builder::new().name("bridge-link-stop".into()).spawn(move || {
            while !flag.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(10));
            }
            for socket in wake {
                let _ = socket.shutdown(Shutdown::Both);
            }
        })?;
        Ok(Self {
            stop,
            sockets,
            hooks: Vec::new(),
            threads: vec![watcher],
        })
    }
    pub(crate) fn on_stop(&mut self, hook: impl FnOnce() + 'static) {
        self.hooks.push(Box::new(hook));
    }
    pub(crate) fn spawn(&mut self, name: &str, run: impl FnOnce() + Send + 'static) -> io::Result<()> {
        self.threads.push(thread::Builder::new().name(name.into()).spawn(run)?);
        Ok(())
    }
    pub(crate) fn finish(&mut self) {
        self.stop.store(true, Ordering::Release);
        for socket in &self.sockets {
            let _ = socket.shutdown(Shutdown::Both);
        }
        for hook in self.hooks.drain(..) {
            hook();
        }
        for thread in self.threads.drain(..) {
            let _ = thread.join();
        }
    }
}
impl Drop for RunScope {
    fn drop(&mut self) {
        self.finish();
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Read, sync::mpsc};

    use super::*;
    #[test]
    fn dropping_scope_wakes_blocked_reads_and_joins_them() {
        let (media, _media_peer) = UnixStream::pair().unwrap();
        let (control, _control_peer) = UnixStream::pair().unwrap();
        let stop = Arc::new(AtomicBool::new(false));
        let mut scope = RunScope::new(stop.clone(), &media, &control).unwrap();
        let (done, finished) = mpsc::channel();
        let mut reader = media.try_clone().unwrap();
        scope
            .spawn("blocked-reader", move || {
                let _ = reader.read(&mut [0]);
                done.send(()).unwrap();
            })
            .unwrap();
        drop(scope);
        assert!(stop.load(Ordering::Acquire));
        finished.try_recv().unwrap();
    }
}
