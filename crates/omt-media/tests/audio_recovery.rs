//! Audio must resume after a brief TCP outage.
//!
//! A tiny loopback proxy sits between Test Patterns (sender) and Studio
//! Monitor (receiver). Cutting every proxied connection for a second exercises
//! OMT's reconnect path plus the receive-side audio recovery.

use std::io;
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use omt_media::{ReceiveWorker, SendSession, SendSessionConfig, SessionState};

struct Proxy {
    port: u16,
    stop: Arc<AtomicBool>,
    cut: Arc<AtomicBool>,
    clients: Arc<std::sync::Mutex<Vec<TcpStream>>>,
    handle: Option<thread::JoinHandle<()>>,
}

impl Proxy {
    fn start(upstream_port: u16) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("proxy listener");
        listener.set_nonblocking(true).expect("proxy nonblocking");
        let port = listener.local_addr().expect("proxy addr").port();
        let stop = Arc::new(AtomicBool::new(false));
        let cut = Arc::new(AtomicBool::new(false));
        let clients = Arc::new(std::sync::Mutex::new(Vec::new()));

        let stop_thread = Arc::clone(&stop);
        let cut_thread = Arc::clone(&cut);
        let clients_thread = Arc::clone(&clients);
        let handle = thread::spawn(move || {
            while !stop_thread.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((client, _)) => {
                        if cut_thread.load(Ordering::Relaxed) {
                            let _ = client.shutdown(Shutdown::Both);
                            continue;
                        }
                        let Ok(upstream) = TcpStream::connect(("127.0.0.1", upstream_port)) else {
                            let _ = client.shutdown(Shutdown::Both);
                            continue;
                        };
                        // A nonblocking listener can hand out nonblocking sockets
                        // on Windows; io::copy would then spin and close at once.
                        let _ = client.set_nonblocking(false);
                        let _ = upstream.set_nonblocking(false);
                        if let Ok(tracked) = client.try_clone() {
                            clients_thread.lock().unwrap().push(tracked);
                        }
                        let Ok(client_read) = client.try_clone() else {
                            continue;
                        };
                        let Ok(upstream_read) = upstream.try_clone() else {
                            continue;
                        };
                        thread::spawn(move || {
                            let mut from = client;
                            let mut to = upstream;
                            let _ = io::copy(&mut from, &mut to);
                        });
                        thread::spawn(move || {
                            let mut from = upstream_read;
                            let mut to = client_read;
                            let _ = io::copy(&mut from, &mut to);
                        });
                    }
                    Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            port,
            stop,
            cut,
            clients,
            handle: Some(handle),
        }
    }

    fn cut(&self) {
        self.cut.store(true, Ordering::Relaxed);
        for client in self.clients.lock().unwrap().drain(..) {
            let _ = client.shutdown(Shutdown::Both);
        }
    }

    fn restore(&self) {
        self.cut.store(false, Ordering::Relaxed);
    }
}

impl Drop for Proxy {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn wait_for_audio(worker: &ReceiveWorker, minimum: u64, timeout: Duration) -> Option<(u64, u64)> {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        let counters = *worker.latest().counters.lock();
        let state = *worker.latest().session_state.lock();
        if state == SessionState::Connected && counters.audio_frames >= minimum {
            return Some((counters.audio_frames, counters.frames_decoded));
        }
        thread::sleep(Duration::from_millis(20));
    }
    None
}

#[test]
fn audio_resumes_after_a_brief_outage() {
    let width = 320i32;
    let height = 180i32;
    let stride = width as usize * 2;
    let frame = Arc::new(vec![128u8; stride * height as usize]);
    let provider_pixels = Arc::clone(&frame);
    let provider = Arc::new(move |_idx| provider_pixels.as_ref().clone());

    let sender = SendSession::start(
        SendSessionConfig {
            name: format!("Audio Recovery {}", std::process::id()),
            width,
            height,
            fps_n: 30,
            fps_d: 1,
            ..SendSessionConfig::default()
        },
        provider,
    )
    .expect("start sender");
    let sender_port = sender.stats().port;
    let proxy = Proxy::start(sender_port);
    let worker = ReceiveWorker::spawn();
    worker.connect(format!("omt://127.0.0.1:{}/AudioRecovery", proxy.port));

    let Some((before_audio, before_video)) = wait_for_audio(&worker, 10, Duration::from_secs(20))
    else {
        worker.shutdown();
        panic!("audio never started");
    };
    assert!(before_video > 0, "video never started");

    proxy.cut();
    thread::sleep(Duration::from_millis(900));
    proxy.restore();

    let recovered = wait_for_audio(&worker, before_audio + 10, Duration::from_secs(25));
    let counters = *worker.latest().counters.lock();
    let state = *worker.latest().session_state.lock();
    let stats = *worker.latest().stats.lock();
    eprintln!(
        "audio recovery: before={before_audio} after={} video={} state={state:?} reconnects={}",
        counters.audio_frames, counters.frames_decoded, stats.reconnects
    );
    worker.shutdown();
    drop(proxy);
    drop(sender);

    assert!(
        recovered.is_some(),
        "audio did not resume after the outage (before={before_audio}, after={}, state={state:?})",
        counters.audio_frames
    );
    assert!(
        counters.frames_decoded > before_video,
        "video did not resume after the outage"
    );
    // `reconnects` is diagnostic only: the watchdog may have forced a fresh
    // session whose counter starts at zero again.
    eprintln!("receiver reconnects={}", stats.reconnects);
}
