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
//! With `--frames-dir` (#189) it also records every frame it reads as one JSON
//! line in `frames.jsonl` there: wall time, QPC time, the frame's QPC
//! timestamp, cursor, sequence, fence value, slot, pool, generation, epoch and
//! the result. Every frame that is not one uniform fixture colour is logged
//! (never deduplicated) and summarised: the most common colours and their
//! fractions, the bounding box of the pixels that differ from the dominant
//! colour, row and column runs, and a kind:
//!
//! - `split-horizontal` / `split-vertical`: exactly two fixture colours split by
//!   one straight boundary, as a torn or partial copy of two consecutive
//!   repaints would look;
//! - `mixed-fixture-colours`: only fixture colours, not one clean split;
//! - `small-overlay`: one fixture colour with a blob of other pixels at most
//!   64x64 (a cursor-sized overlay);
//! - `edge-band`: one fixture colour with other pixels spanning a full edge (a
//!   crop that includes window frame, or an unpainted strip);
//! - `foreign-dominant`: the dominant colour is not a fixture colour (for
//!   example an unpainted black or white surface);
//! - `other`.
//!
//! The first `--max-saved` (default 200) such frames are saved as PNGs there.
//!
//! ```text
//! $env:PORTHOLE_ATTACH_TOKEN = '<attach_token>'
//! windows_native_capture_consumer --session-id <id> --endpoint <local_endpoint> [--seconds 60] [--log consumer.log]
//!                                 [--frames-dir frames] [--max-saved 200]
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
        collections::HashMap,
        fs::File,
        io::{BufWriter, Write},
        path::{Path, PathBuf},
        sync::Mutex,
        time::{Duration, Instant, SystemTime, UNIX_EPOCH},
    };

    use jackstay::{
        acquisition::arena::{AcquireOutcome, FrameDescriptor},
        native::windows::{AdapterSelection, D3d11Device, SharedFenceHandle, SharedTextureHandle, setup::D3d11SetupClient},
    };
    use porthole::native_capture::{OpenError, open};
    use porthole_protocol::capture_sessions::{
        NATIVE_ATTACH_TRANSPORT_WINDOWS_LOCAL_ENDPOINT, NativeCaptureInfo, WindowsNativePublication,
    };
    use serde_json::{Value, json};
    use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

    fn wall_seconds() -> f64 {
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0.0, |time| time.as_secs_f64())
    }

    /// QPC now in nanoseconds: the clock of a frame's `timestamp_ns` (WGC's
    /// `SystemRelativeTime`) and of the capture fixture's event log.
    fn qpc_ns() -> u64 {
        static FREQUENCY: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
        let frequency = *FREQUENCY.get_or_init(|| {
            let mut frequency = 0i64;
            // SAFETY: the out pointer is a live local.
            let _ = unsafe { QueryPerformanceFrequency(&mut frequency) };
            frequency
        });
        let mut counter = 0i64;
        // SAFETY: the out pointer is a live local.
        let _ = unsafe { QueryPerformanceCounter(&mut counter) };
        if counter <= 0 || frequency <= 0 {
            return 0;
        }
        (counter as u128 * 1_000_000_000 / frequency as u128) as u64
    }

    struct Log(Mutex<Option<File>>, Instant);

    impl Log {
        fn line(&self, text: &str) {
            let line = format!("[{:.3} +{:>8.3}s] {text}", wall_seconds(), self.1.elapsed().as_secs_f64());
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
        kinds: Vec<(String, u64)>,
    }

    impl Totals {
        fn count_kind(&mut self, kind: &str) {
            match self.kinds.iter_mut().find(|(name, _)| name == kind) {
                Some((_, count)) => *count += 1,
                None => self.kinds.push((kind.to_owned(), 1)),
            }
        }
    }

    /// `--frames-dir`: one JSON line per frame, and PNGs of non-uniform ones.
    struct Forensics {
        dir: PathBuf,
        frames: BufWriter<File>,
        max_saved: u32,
        saved: u32,
        non_uniform: u64,
    }

    /// BGRA as read back; the fixture paints red, green (0xa0) and blue.
    const FIXTURE_COLOURS: [(&str, [u8; 4]); 3] = [("red", [0, 0, 255, 255]), ("green", [0, 160, 0, 255]), ("blue", [255, 0, 0, 255])];

    /// The frame's colour name if every pixel is one of the fixture's colours.
    fn uniform(pixels: &[u8]) -> Option<&'static str> {
        FIXTURE_COLOURS
            .into_iter()
            .find(|(_, bgra)| pixels.chunks_exact(4).all(|pixel| pixel == bgra))
            .map(|(name, _)| name)
    }

    fn is_fixture(bgra: [u8; 4]) -> bool {
        FIXTURE_COLOURS.iter().any(|(_, colour)| *colour == bgra)
    }

    fn colour_name(bgra: [u8; 4]) -> String {
        match FIXTURE_COLOURS.iter().find(|(_, colour)| *colour == bgra) {
            Some((name, _)) => (*name).to_owned(),
            None => match bgra {
                [0, 0, 0, 255] => "black".to_owned(),
                [255, 255, 255, 255] => "white".to_owned(),
                [b, g, r, a] => format!("rgba({r},{g},{b},{a})"),
            },
        }
    }

    type Runs = Vec<([u8; 4], usize, usize)>;

    /// Runs of identical one-colour lines (rows or columns): `None` if some
    /// line has more than one colour.
    fn line_runs(lines: impl Iterator<Item = Option<[u8; 4]>>) -> Option<Runs> {
        let mut runs: Runs = Vec::new();
        for (index, line) in lines.enumerate() {
            let colour = line?;
            match runs.last_mut() {
                Some((last, _, end)) if *last == colour => *end = index + 1,
                _ => runs.push((colour, index, index + 1)),
            }
        }
        Some(runs)
    }

    fn uniform_line<'a>(mut pixels: impl Iterator<Item = &'a [u8]>) -> Option<[u8; 4]> {
        let first: [u8; 4] = pixels.next()?.try_into().ok()?;
        pixels.all(|pixel| pixel == first).then_some(first)
    }

    /// The kind and a short summary of a frame that is not one fixture colour
    /// (see the module docs).
    fn analyse(pixels: &[u8], width: u32, height: u32) -> (&'static str, Value) {
        let (width, height) = (width as usize, height as usize);
        let total = (width * height).max(1);
        let mut histogram: HashMap<[u8; 4], usize> = HashMap::new();
        for pixel in pixels.chunks_exact(4) {
            *histogram.entry(pixel.try_into().expect("4 bytes")).or_default() += 1;
        }
        let mut colours: Vec<([u8; 4], usize)> = histogram.into_iter().collect();
        colours.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
        let dominant = colours.first().map_or([0; 4], |(colour, _)| *colour);
        let fixture_only = colours.iter().all(|(colour, _)| is_fixture(*colour));
        let (mut left, mut top, mut right, mut bottom, mut others) = (usize::MAX, usize::MAX, 0, 0, 0usize);
        for (index, pixel) in pixels.chunks_exact(4).enumerate() {
            if pixel != dominant {
                let (x, y) = (index % width, index / width);
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
                others += 1;
            }
        }
        let row = |y: usize| &pixels[y * width * 4..(y + 1) * width * 4];
        let row_runs = line_runs((0..height).map(|y| uniform_line(row(y).chunks_exact(4))));
        let column_runs = line_runs((0..width).map(|x| uniform_line((0..height).map(|y| &row(y)[x * 4..x * 4 + 4]))));
        let two_runs = |runs: &Option<Runs>| runs.as_ref().is_some_and(|runs| runs.len() == 2);
        let (box_width, box_height) = (right.saturating_sub(left), bottom.saturating_sub(top));
        let touches_edge = left == 0 || top == 0 || right == width || bottom == height;
        let kind = if fixture_only && colours.len() == 2 && two_runs(&row_runs) {
            "split-horizontal"
        } else if fixture_only && colours.len() == 2 && two_runs(&column_runs) {
            "split-vertical"
        } else if fixture_only {
            "mixed-fixture-colours"
        } else if !is_fixture(dominant) {
            "foreign-dominant"
        } else if box_width <= 64 && box_height <= 64 {
            // Checked before `edge-band`: a cursor-sized blob is an overlay
            // even where it touches an edge or corner.
            "small-overlay"
        } else if touches_edge && (box_width == width || box_height == height) {
            "edge-band"
        } else {
            "other"
        };
        let fraction = |count: usize| (count as f64 / total as f64 * 1e4).round() / 1e4;
        let describe = |runs: Option<Runs>| {
            runs.map(|runs| {
                let shown: Vec<Value> = runs
                    .iter()
                    .take(8)
                    .map(|(colour, start, end)| json!([colour_name(*colour), start, end]))
                    .collect();
                json!({ "count": runs.len(), "first": shown })
            })
        };
        let summary = json!({
            "distinct_colours": colours.len(),
            "top_colours": colours.iter().take(4).map(|(colour, count)| json!([colour_name(*colour), fraction(*count)])).collect::<Vec<_>>(),
            "dominant": colour_name(dominant),
            "fixture_colours_only": fixture_only,
            "other_pixels": others,
            "other_bbox": (others > 0).then(|| json!({ "x": left, "y": top, "width": box_width, "height": box_height })),
            "row_runs": describe(row_runs),
            "column_runs": describe(column_runs),
        });
        (kind, summary)
    }

    fn save_png(path: &Path, pixels: &[u8], width: u32, height: u32) -> Result<(), String> {
        let rgba: Vec<u8> = pixels
            .chunks_exact(4)
            .flat_map(|pixel| [pixel[2], pixel[1], pixel[0], pixel[3]])
            .collect();
        image::save_buffer(path, &rgba, width, height, image::ExtendedColorType::Rgba8).map_err(|error| error.to_string())
    }

    /// Classify one read-back frame, count it, and record it.
    #[allow(clippy::too_many_arguments)]
    fn check_frame(
        log: &Log,
        forensics: &mut Option<Forensics>,
        totals: &mut Totals,
        descriptor: &FrameDescriptor,
        epoch: u64,
        pixels: &[u8],
        transitional_window: bool,
        wall: f64,
        qpc: u64,
    ) -> String {
        totals.frames += 1;
        let color = uniform(pixels);
        let transitional = color.is_none() && transitional_window;
        let result = match color {
            Some(_) => {
                totals.verified += 1;
                "verified"
            }
            None if transitional => {
                totals.transitional += 1;
                "transitional"
            }
            None => {
                totals.not_uniform += 1;
                "not_uniform"
            }
        };
        let size = (descriptor.width, descriptor.height);
        if totals.sizes.last() != Some(&size) {
            totals.sizes.push(size);
        }
        let mut record = json!({
            "wall": (wall * 1e3).round() / 1e3,
            "qpc_ns": qpc,
            "frame_qpc_ns": descriptor.timestamp_ns,
            "cursor": descriptor.cursor,
            "sequence": descriptor.sequence,
            "fence_value": descriptor.fence_value,
            "slot": descriptor.slot_id,
            "pool_id": descriptor.pool_id,
            "generation": descriptor.config_generation,
            "epoch": epoch,
            "width": descriptor.width,
            "height": descriptor.height,
            "dropped_before_publish": descriptor.dropped_before_publish,
            "producer_drop_count": descriptor.producer_drop_count,
            "result": result,
            "colour": color,
        });
        if color.is_none() {
            let (kind, summary) = analyse(pixels, descriptor.width, descriptor.height);
            totals.count_kind(kind);
            let mut png = None;
            if let Some(forensics) = forensics.as_mut() {
                forensics.non_uniform += 1;
                if forensics.saved < forensics.max_saved {
                    let name = format!(
                        "nonuniform-{:04}-seq{}-fence{}-slot{}-gen{}.png",
                        forensics.non_uniform,
                        descriptor.sequence,
                        descriptor.fence_value,
                        descriptor.slot_id,
                        descriptor.config_generation
                    );
                    match save_png(&forensics.dir.join(&name), pixels, descriptor.width, descriptor.height) {
                        Ok(()) => {
                            forensics.saved += 1;
                            png = Some(name);
                        }
                        Err(error) => log.line(&format!("saving {name} failed: {error}")),
                    }
                }
            }
            log.line(&format!(
                "{} frame: {kind}; frame_qpc_ns {} cursor {} sequence {} fence {} slot {} pool {} generation {} epoch {} {}x{}; {}; png {}",
                if transitional { "transitional" } else { "NON-UNIFORM" },
                descriptor.timestamp_ns,
                descriptor.cursor,
                descriptor.sequence,
                descriptor.fence_value,
                descriptor.slot_id,
                descriptor.pool_id,
                descriptor.config_generation,
                epoch,
                descriptor.width,
                descriptor.height,
                summary,
                png.as_deref().unwrap_or("-"),
            ));
            record["kind"] = json!(kind);
            record["summary"] = summary;
            record["png"] = json!(png);
        }
        if let Some(forensics) = forensics.as_mut() {
            let _ = writeln!(forensics.frames, "{record}");
            let _ = forensics.frames.flush();
        }
        format!(
            "frame {}x{} generation {} epoch {}: {}",
            descriptor.width,
            descriptor.height,
            descriptor.config_generation,
            epoch,
            match (color, transitional) {
                (Some(color), _) => format!("uniform {color}, verified"),
                (None, true) => "not uniform, transitional (repaint after a new pool generation)".to_owned(),
                (None, false) => "NOT uniform".to_owned(),
            }
        )
    }

    /// One attachment: read frames until the publication closes, the
    /// producer leaves, or the deadline passes.
    fn attach_once(
        log: &Log,
        native: &NativeCaptureInfo,
        session_id: &str,
        deadline: Instant,
        totals: &mut Totals,
        forensics: &mut Option<Forensics>,
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
                    let (wall, qpc) = (wall_seconds(), qpc_ns());
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
                    if generation != Some(descriptor.config_generation) {
                        generation = Some(descriptor.config_generation);
                        generation_since = Instant::now();
                    }
                    let transitional_window = generation_since.elapsed() < REPAINT_GRACE;
                    let summary = check_frame(
                        log,
                        forensics,
                        totals,
                        &descriptor,
                        opened.epoch,
                        &pixels,
                        transitional_window,
                        wall,
                        qpc,
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
        let mut frames_dir = None;
        let mut max_saved = 200;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            let mut value = || args.next().expect("option value");
            match arg.as_str() {
                "--session-id" => session_id = Some(value()),
                "--endpoint" => endpoint = Some(value()),
                "--seconds" => seconds = value().parse().expect("--seconds N"),
                "--log" => path = Some(value()),
                "--frames-dir" => frames_dir = Some(PathBuf::from(value())),
                "--max-saved" => max_saved = value().parse().expect("--max-saved N"),
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
        let mut forensics = frames_dir.map(|dir| {
            std::fs::create_dir_all(&dir).expect("frames directory");
            let frames = BufWriter::new(File::create(dir.join("frames.jsonl")).expect("frames.jsonl"));
            Forensics {
                dir,
                frames,
                max_saved,
                saved: 0,
                non_uniform: 0,
            }
        });
        log.line(&format!(
            "consumer pid {} for session {session_id}; clock pair wall {:.3} qpc_ns {}",
            std::process::id(),
            wall_seconds(),
            qpc_ns()
        ));
        let deadline = Instant::now() + Duration::from_secs(seconds);
        let mut totals = Totals::default();
        let mut refused = None;
        while Instant::now() < deadline {
            match attach_once(&log, &native, &session_id, deadline, &mut totals, &mut forensics) {
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
            "summary: attaches {}, epochs {:?}, frames {}, verified {}, transitional {}, not uniform {}, kinds {:?}, saved {}, sizes {:?}, final refusal {:?}",
            totals.attaches,
            totals.epochs,
            totals.frames,
            totals.verified,
            totals.transitional,
            totals.not_uniform,
            totals.kinds,
            forensics.as_ref().map_or(0, |forensics| forensics.saved),
            totals.sizes,
            refused
        ));
        if totals.verified > 0 && totals.not_uniform == 0 {
            std::process::ExitCode::SUCCESS
        } else {
            std::process::ExitCode::FAILURE
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn frame(width: usize, height: usize, paint: impl Fn(usize, usize) -> [u8; 4]) -> Vec<u8> {
            let paint = &paint;
            (0..height).flat_map(|y| (0..width).flat_map(move |x| paint(x, y))).collect()
        }

        const RED: [u8; 4] = FIXTURE_COLOURS[0].1;
        const GREEN: [u8; 4] = FIXTURE_COLOURS[1].1;
        const BLACK: [u8; 4] = [0, 0, 0, 255];

        #[test]
        fn classifies_non_uniform_frames() {
            let torn = frame(40, 30, |_, y| if y < 12 { RED } else { GREEN });
            let (kind, summary) = analyse(&torn, 40, 30);
            assert_eq!(kind, "split-horizontal");
            assert_eq!(summary["row_runs"]["count"], 2);
            assert_eq!(
                analyse(&frame(40, 30, |x, _| if x < 5 { GREEN } else { RED }), 40, 30).0,
                "split-vertical"
            );
            assert_eq!(
                analyse(&frame(40, 30, |x, y| if (x + y) % 2 == 0 { GREEN } else { RED }), 40, 30).0,
                "mixed-fixture-colours"
            );
            let cursor = frame(200, 100, |x, y| {
                if (50..62).contains(&x) && (20..40).contains(&y) {
                    BLACK
                } else {
                    RED
                }
            });
            let (kind, summary) = analyse(&cursor, 200, 100);
            assert_eq!(kind, "small-overlay");
            assert_eq!(summary["other_bbox"], json!({ "x": 50, "y": 20, "width": 12, "height": 20 }));
            assert_eq!(
                analyse(&frame(200, 100, |_, y| if y < 3 { BLACK } else { RED }), 200, 100).0,
                "edge-band"
            );
            assert_eq!(
                analyse(&frame(40, 30, |x, _| if x < 30 { BLACK } else { RED }), 40, 30).0,
                "foreign-dominant"
            );
        }
    }
}
