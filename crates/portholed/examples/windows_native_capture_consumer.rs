//! A separate-process consumer of a Windows native capture session (#186).
//!
//! It opens the session's Local Endpoint with the attach token, describes the
//! producer, creates a D3D11 device on the producer's adapter, attaches, and
//! reads frames back to verify them. It installs replacement pools after a
//! resize and attaches again after an epoch change (device loss). It logs one
//! line per change: size, pool generation, epoch, whether the frame is one
//! uniform colour (the capture fixture paints solid red, green or blue), and
//! every attach refusal. It exits when the session refuses a new attach (for
//! example after the captured window closed) or after `--seconds`.
//!
//! ```text
//! $env:PORTHOLE_ATTACH_TOKEN = '<attach_token>'
//! windows_native_capture_consumer --session-id <id> --endpoint <local_endpoint> [--seconds 60] [--log consumer.log]
//! ```
//!
//! The token comes from the environment so it stays out of process listings.

#[cfg(not(windows))]
fn main() {}

#[cfg(windows)]
fn main() -> std::process::ExitCode {
    consumer::main()
}

#[cfg(windows)]
mod consumer {
    use std::{
        fs::File,
        io::Write,
        sync::Mutex,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use jackstay::{
        acquisition::arena::AcquireOutcome,
        native::windows::{AdapterSelection, D3d11Device, SharedFenceHandle, SharedTextureHandle, setup::D3d11SetupClient},
    };
    use porthole::native_capture::{OpenError, open};
    use porthole_protocol::capture_sessions::{
        NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT, NativeCaptureInfo, WindowsNativePublication,
    };

    struct Log(Mutex<Option<File>>, Instant);

    impl Log {
        fn line(&self, text: &str) {
            let wall = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |time| time.as_secs_f64());
            let line = format!("[{wall:.3} +{:>8.3}s] {text}", self.1.elapsed().as_secs_f64());
            println!("{line}");
            if let Some(file) = self.0.lock().unwrap().as_mut() {
                let _ = writeln!(file, "{line}");
                let _ = file.flush();
            }
        }
    }

    /// A frame within this long of a new pool generation may show the window
    /// mid-repaint (newly exposed area not yet painted); it is logged as
    /// transitional rather than counted as a failure.
    const REPAINT_GRACE: Duration = Duration::from_millis(250);

    #[derive(Default)]
    struct Totals {
        attaches: u32,
        frames: u64,
        verified: u64,
        transitional: u64,
        not_uniform: u64,
        sizes: Vec<(u32, u32)>,
        epochs: Vec<u64>,
    }

    /// The frame's colour name if every pixel is one of the fixture's colours.
    fn uniform(pixels: &[u8]) -> Option<&'static str> {
        // BGRA as read back; the fixture paints red, green (0xa0) and blue.
        [("red", [0, 0, 255, 255]), ("green", [0, 160, 0, 255]), ("blue", [255, 0, 0, 255])]
            .into_iter()
            .find(|(_, bgra)| pixels.chunks_exact(4).all(|pixel| pixel == bgra))
            .map(|(name, _)| name)
    }

    /// One attachment: read frames until the publication closes, the
    /// producer leaves, or the deadline passes.
    fn attach_once(
        log: &Log,
        native: &NativeCaptureInfo,
        session_id: &str,
        deadline: Instant,
        totals: &mut Totals,
    ) -> Result<(), OpenError> {
        let opened = open(session_id, native)?;
        totals.attaches += 1;
        totals.epochs.push(opened.epoch);
        log.line(&format!("attached: publication {:?}, epoch {}", opened.publication, opened.epoch));
        if opened.publication != WindowsNativePublication::D3d11 {
            log.line("CPU publication: this consumer verifies D3D11 frames only");
            return Ok(());
        }
        // SAFETY: `open` verified the endpoint's server (this user, this
        // logon session) before any byte; this process is the sole recipient.
        let mut client = unsafe { D3d11SetupClient::from_stream(opened.stream) };
        let producer = client
            .describe()
            .map_err(|error| OpenError::Protocol(format!("describe: {error}")))?;
        log.line(&format!(
            "producer adapter {} {:?} (software {})",
            producer.adapter.luid, producer.adapter.description, producer.adapter.software
        ));
        let device = D3d11Device::new(AdapterSelection::Luid(producer.adapter.luid))
            .map_err(|error| OpenError::Protocol(format!("device: {error}")))?;
        let mut consumer = client
            .attach(1, &device)
            .map_err(|error| OpenError::Protocol(format!("attach: {error}")))?;
        let mut after = 0;
        let mut last = String::new();
        let mut generation = None;
        let mut generation_since = Instant::now();
        while Instant::now() < deadline {
            let outcome = consumer
                .acquire_latest(after)
                .map_err(|error| OpenError::Protocol(format!("acquire: {error}")))?;
            match outcome {
                AcquireOutcome::Frame(frame) => {
                    after = frame.cursor();
                    let descriptor = *frame.descriptor();
                    let native = frame
                        .native_resources::<SharedTextureHandle, SharedFenceHandle>()
                        .map_err(|error| OpenError::Protocol(format!("native resources: {error}")))?;
                    let texture = device
                        .open_texture(native.surface)
                        .map_err(|error| OpenError::Protocol(format!("open texture: {error}")))?;
                    let ready = device
                        .open_fence(native.sync_handle)
                        .map_err(|error| OpenError::Protocol(format!("open fence: {error}")))?;
                    if ready.is_abandoned() {
                        log.line("frame abandoned: the producer's device went away");
                        return Ok(());
                    }
                    let pixels = device
                        .read_pixels(&texture, &[(&ready, descriptor.fence_value)], Duration::from_secs(2))
                        .map_err(|error| OpenError::Protocol(format!("read back: {error}")))?;
                    drop(frame);
                    totals.frames += 1;
                    if generation != Some(descriptor.config_generation) {
                        generation = Some(descriptor.config_generation);
                        generation_since = Instant::now();
                    }
                    let color = uniform(&pixels);
                    let transitional = color.is_none() && generation_since.elapsed() < REPAINT_GRACE;
                    if color.is_some() {
                        totals.verified += 1;
                    } else if transitional {
                        totals.transitional += 1;
                    } else {
                        totals.not_uniform += 1;
                    }
                    let size = (descriptor.width, descriptor.height);
                    if totals.sizes.last() != Some(&size) {
                        totals.sizes.push(size);
                    }
                    let summary = format!(
                        "frame {}x{} generation {} epoch {}: {}",
                        descriptor.width,
                        descriptor.height,
                        descriptor.config_generation,
                        opened.epoch,
                        match (color, transitional) {
                            (Some(color), _) => format!("uniform {color}, verified"),
                            (None, true) => "not uniform, transitional (repaint after a new pool generation)".to_owned(),
                            (None, false) => "NOT uniform".to_owned(),
                        }
                    );
                    if summary != last {
                        log.line(&summary);
                        last = summary;
                    }
                }
                AcquireOutcome::Reconfiguration => {
                    let installed = client
                        .install_configuration(&mut consumer)
                        .map_err(|error| OpenError::Protocol(format!("install configuration: {error}")))?;
                    log.line(&format!("installed a replacement pool: {}", installed.is_some()));
                }
                AcquireOutcome::Closed => {
                    log.line("publication closed");
                    return Ok(());
                }
                _ => {
                    if !client.is_alive() {
                        log.line("producer closed the setup connection");
                        return Ok(());
                    }
                    std::thread::sleep(Duration::from_millis(15));
                }
            }
        }
        Ok(())
    }

    pub fn main() -> std::process::ExitCode {
        let mut session_id = None;
        let mut endpoint = None;
        let mut seconds = 60;
        let mut path = None;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut value = || args.next().expect("option value");
            match arg.as_str() {
                "--session-id" => session_id = Some(value()),
                "--endpoint" => endpoint = Some(value()),
                "--seconds" => seconds = value().parse().expect("--seconds N"),
                "--log" => path = Some(value()),
                other => panic!("unknown argument {other}"),
            }
        }
        let session_id = session_id.expect("--session-id");
        let native = NativeCaptureInfo {
            transport_kind: NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT,
            endpoint: endpoint.expect("--endpoint"),
            attach_token: std::env::var("PORTHOLE_ATTACH_TOKEN").expect("PORTHOLE_ATTACH_TOKEN"),
        };
        let log = Log(Mutex::new(path.map(|path| File::create(path).expect("log file"))), Instant::now());
        log.line(&format!("consumer pid {} for session {session_id}", std::process::id()));
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let mut totals = Totals::default();
        let mut refused = None;
        while Instant::now() < deadline {
            match attach_once(&log, &native, &session_id, deadline, &mut totals) {
                Ok(()) => std::thread::sleep(Duration::from_millis(100)),
                Err(OpenError::Rejected(message)) => {
                    log.line(&format!("attach refused: {message}"));
                    refused = Some(message);
                    break;
                }
                Err(OpenError::Endpoint(jackstay::local::Error::Io(error))) if error.kind() == std::io::ErrorKind::NotFound => {
                    log.line("attach endpoint is gone: the session was removed");
                    refused = Some("endpoint removed".to_owned());
                    break;
                }
                Err(error) => {
                    log.line(&format!("attach failed: {error}"));
                    std::thread::sleep(Duration::from_millis(250));
                }
            }
        }
        log.line(&format!(
            "summary: attaches {}, epochs {:?}, frames {}, verified {}, transitional {}, not uniform {}, sizes {:?}, final refusal {:?}",
            totals.attaches, totals.epochs, totals.frames, totals.verified, totals.transitional, totals.not_uniform, totals.sizes, refused
        ));
        if totals.verified > 0 && totals.not_uniform == 0 {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::FAILURE
        }
    }
}
