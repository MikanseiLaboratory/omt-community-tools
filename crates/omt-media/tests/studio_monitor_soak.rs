use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use omt_media::{ReceiveWorker, SendSession, SendSessionConfig, SessionState};

#[test]
fn studio_monitor_playout_keeps_advancing() {
    let width = 640i32;
    let height = 360i32;
    let stride = width as usize * 2;
    let pixels = Arc::new(vec![128u8; stride * height as usize]);
    let provider_pixels = Arc::clone(&pixels);
    let provider = Arc::new(move |_idx| provider_pixels.as_ref().clone());

    let sender = SendSession::start(
        SendSessionConfig {
            name: "Studio Monitor Soak".into(),
            width,
            height,
            fps_n: 30,
            fps_d: 1,
            ..SendSessionConfig::default()
        },
        provider,
    )
    .expect("start sender");
    let port = sender.stats().port;
    assert!(port > 0);

    let worker = ReceiveWorker::spawn();
    worker.connect(format!("omt://127.0.0.1:{port}"));

    // Debug CI (especially macOS) encodes/decodes well below 30 fps. Wait long
    // enough after the first presented frame to prove playout is still moving.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut samples = Vec::new();
    let mut last_sample = Instant::now() - Duration::from_secs(1);
    let mut first_video_at = None::<Instant>;
    while Instant::now() < deadline {
        if last_sample.elapsed() >= Duration::from_secs(1) {
            let latest = worker.latest();
            let counters = *latest.counters.lock();
            let stats = *latest.stats.lock();
            let state = *latest.session_state.lock();
            if counters.frames_decoded > 0 && first_video_at.is_none() {
                first_video_at = Some(Instant::now());
            }
            samples.push((
                counters.frames_decoded,
                counters.audio_frames,
                stats.frames_decoded,
                stats.bytes_received,
                state,
            ));
            last_sample = Instant::now();
            if first_video_at.is_some_and(|t| t.elapsed() >= Duration::from_secs(5)) {
                break;
            }
        }
        thread::sleep(Duration::from_millis(10));
    }

    worker.shutdown();
    drop(sender);

    eprintln!("Studio Monitor playout samples: {samples:?}");
    let first = samples
        .iter()
        .position(|s| s.0 > 0)
        .expect("no presented video");
    let final_sample = samples.last().expect("samples");
    assert_eq!(final_sample.4, SessionState::Connected);
    assert!(
        final_sample.0 > samples[first].0 + 15,
        "presented video stopped advancing: {samples:?}"
    );
    assert!(
        final_sample.2 > samples[first].2 + 15,
        "decoded video stopped advancing: {samples:?}"
    );
}
