use std::cell::Cell;
use std::sync::atomic::AtomicI32;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;
use std::time::Instant;

use atomic_float::AtomicF64;
use derive_selflog::SelfLog;
use portable_atomic::AtomicI128;
use portable_atomic::AtomicU128;

#[derive(Default, SelfLog)]
struct Shared {
    scanned: AtomicU64,
    balance: AtomicI32,
    ratio: AtomicF64,
    label: String,
}

#[derive(Default, SelfLog)]
struct Local {
    count: u32,
    balance: i64,
    total: f64,
}

#[derive(Default, SelfLog)]
struct Wide {
    shared: AtomicU128,
    signed: AtomicI128,
    local: u128,
}

#[derive(Default, SelfLog)]
struct Extras {
    #[selflog(extra(kind = transform, key = saved_mb, value = |saved| saved as f64 / 1e6), extra(kind = transform, key = saved_kb, value = |saved| saved / 1000))]
    saved: AtomicU64,
    #[selflog(extra(kind = transform, key = label_length, value = |label| label.len()))]
    label: String,
}

#[derive(Default, SelfLog)]
struct Labeled {
    platform: String,
    #[selflog(entry(key = source))]
    origin: String,
    #[selflog(entry(include = false))]
    hidden: AtomicU64,
    #[selflog(entry(key = images))]
    scanned: AtomicU64,
    #[selflog(entry(include = false))]
    cache: Vec<u8>,
}

#[derive(SelfLog)]
struct Timed {
    #[selflog(entry(include = false), extra(kind = elapsed, unit = s))]
    started: Instant,
    #[selflog(entry(include = false), extra(kind = elapsed, unit = ms, key = batch_ms))]
    batch: Instant,
    #[selflog(extra(kind = rate, reference = started, unit = s))]
    images: AtomicU64,
    #[selflog(extra(kind = rate, reference = started, unit = min, key = bytes_rate))]
    bytes: f64,
}

#[derive(Default, SelfLog)]
struct Unshared {
    count: u32,
    #[selflog(entry(include = false))]
    cache: Cell<u32>,
}

#[test]
fn log_every_level() {
    let shared = Shared { label: String::from("images"), ..Shared::default() };

    shared.log_trace("Trace");
    shared.log_debug("Debug");
    shared.log_info("Info");
    shared.log_warn("Warn");
    shared.log_error("Error");

    assert_eq!(shared.label, "images");
}

#[test]
fn log_plain_and_wide_numbers() {
    let local = Local { count: 3, balance: -7, total: 2.5 };
    let wide = Wide { local: u64::MAX as u128 + 1, ..Wide::default() };

    local.log_info("Local");
    wide.log_debug("Wide");

    assert_eq!(wide.local, u64::MAX as u128 + 1);
}

#[test]
fn log_elapsed_and_rates() {
    let timed = Timed { started: Instant::now(), batch: Instant::now(), images: AtomicU64::new(5), bytes: 2.5 };

    timed.log_info("Scanning");

    assert_eq!(timed.images.load(Ordering::Relaxed), 5);
}

#[test]
fn log_extras() {
    let extras = Extras { saved: AtomicU64::new(2_500_000), label: String::from("images") };

    extras.log_info("Finished");

    assert_eq!(extras.saved.load(Ordering::Relaxed), 2_500_000);
}

#[test]
fn log_opt_in_and_opt_out() {
    let labeled = Labeled { platform: String::from("Atari_2600"), origin: String::from("emumovies"), ..Labeled::default() };
    labeled.hidden.fetch_add(1, Ordering::Relaxed);

    labeled.log_info("Updating");

    assert_eq!(labeled.hidden.load(Ordering::Relaxed), 1);
    assert!(labeled.cache.is_empty());
}

#[test]
fn log_every_interval_while_guard_lives() {
    let shared = Shared::default();

    thread::scope(|scope| {
        let _progress = shared.log_info_every(scope, Duration::from_millis(10), "Scanning");
        shared.scanned.fetch_add(3, Ordering::Relaxed);
        thread::sleep(Duration::from_millis(50));
    });

    assert_eq!(shared.scanned.load(Ordering::Relaxed), 3);
}

#[test]
fn log_every_stops_without_waiting_for_interval() {
    let shared = Shared::default();
    let started = Instant::now();

    thread::scope(|scope| {
        let _progress = shared.log_info_every(scope, Duration::from_secs(60), "Scanning");
    });

    assert!(started.elapsed() < Duration::from_secs(1));
}

#[cfg(feature = "tracing")]
#[test]
fn log_at_level_given() {
    let shared = Shared::default();

    for level in [tracing::Level::TRACE, tracing::Level::DEBUG, tracing::Level::INFO, tracing::Level::WARN, tracing::Level::ERROR] {
        shared.log(level, "Leveled");
    }
    thread::scope(|scope| {
        let _progress = shared.log_every(tracing::Level::INFO, scope, Duration::from_millis(10), "Scanning");
        thread::sleep(Duration::from_millis(50));
    });

    assert_eq!(shared.scanned.load(Ordering::Relaxed), 0);
}

#[test]
fn log_on_struct_that_is_not_sync() {
    let unshared = Unshared::default();
    unshared.cache.set(1);

    unshared.log_info("Unshared");

    assert_eq!(unshared.count, 0);
}
