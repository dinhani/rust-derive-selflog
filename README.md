# derive-selflog

Derive that generates methods that log the fields of a struct, once or every interval.

`#[derive(SelfLog)]` generates, for each log level (`trace`, `debug`, `info`, `warn`, `error`):

| Method                                        | Does                                                                                                     |
| --------------------------------------------- | -------------------------------------------------------------------------------------------------------- |
| `log_<level>(message)`                        | Logs the message with the fields as `key=value` pairs                                                    |
| `log_<level>_every(scope, interval, message)` | Calls `log_<level>(message)` each interval on a thread of the scope, until the returned value is dropped |

With the `tracing` feature it also generates two methods that take the level as a `tracing::Level`:

| Method                                       | Does                                            |
| -------------------------------------------- | ----------------------------------------------- |
| `log(level, message)`                        | Same as `log_<level>`, at the given level       |
| `log_every(level, scope, interval, message)` | Same as `log_<level>_every`, at the given level |

The struct needs named fields. `#[selflog(...)]` goes on fields, not on the struct.

Every field is logged unless it has `entry(include = false)`.

- **Numeric fields** (`u8` to `u128`, `usize`, `i8` to `i128`, `isize`, `f32`, `f64`) and **atomic fields** (`AtomicU8` to `AtomicU64`, `AtomicUsize`, `AtomicI8` to `AtomicI64`, `AtomicIsize`, `AtomicU128` and `AtomicI128` from `portable-atomic`, and `AtomicF32`, `AtomicF64` from `atomic_float`) are logged by value. Atomic fields are read with relaxed ordering.
- Other fields are logged with `Display`. A field without `Display`, such as `Instant`, needs `entry(include = false)`.

`log_<level>_every` and `log_every` spawn their loop on the `thread::scope` the caller passes, and return a guard. Dropping the guard stops the loop at once, without waiting for the rest of the interval. Keep it in a named variable (`_progress`, not `_`) until the work ends; at the end of the scope closure it is dropped before the scope joins. These two methods need the struct to be `Sync`; the others do not.

## Install

```toml
[dependencies]
derive-selflog = { git = "https://github.com/dinhani/rust-derive-selflog", tag = "v0.1.0" }
```

By default the log methods print with `println!`. With the `tracing` feature they log through `tracing` instead, and the crate that uses the derive needs `tracing` as a dependency:

```toml
[dependencies]
derive-selflog = { git = "https://github.com/dinhani/rust-derive-selflog", tag = "v0.1.0", features = ["tracing"] }
tracing = "0.1"
```

Without the `tracing` feature every log method prints the same way, without the level.

## Usage

```rust
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;
use std::thread;
use std::time::Duration;

use derive_selflog::SelfLog;

#[derive(Default, SelfLog)]
struct Totals {
    platform: String,
    scanned: AtomicU64,
}

let totals = &Totals { platform: String::from("Atari_2600"), ..Totals::default() };
thread::scope(|scope| {
    let _progress = totals.log_info_every(scope, Duration::from_secs(5), "Scanning");
    for _ in 0..3615 {
        totals.scanned.fetch_add(1, Ordering::Relaxed);
    }
});
// Scanning platform=Atari_2600 scanned=1200   every 5 seconds while the loop runs
totals.log_info("Finished");
// Finished platform=Atari_2600 scanned=3615
```

## Options

Field options control the log line. Every option has the same format, and arguments are always named:

```
#[selflog(<option>(<argument> = <value>, ...))]
```

There are two options:

- **`entry`** controls the field's own entry.
- **`extra`** adds an entry after the field, computed from the field. It can be repeated.

| Option  | Argument    | Value                                                             | Default                           |
| ------- | ----------- | ----------------------------------------------------------------- | --------------------------------- |
| `entry` | `include`   | `true` or `false`                                                 | `true`                            |
| `entry` | `key`       | the entry's key                                                   | the field name                    |
| `extra` | `kind`      | `elapsed`, `rate` or `transform`                                  | required                          |
| `extra` | `key`       | the entry's key                                                   | depends on `kind`                 |
| `extra` | `reference` | a field with `elapsed() -> Duration`, for `kind = rate`           | required for `rate`               |
| `extra` | `value`     | a closure that receives the field's value, for `kind = transform` | required for `transform`          |
| `extra` | `unit`      | `ms`, `s`, `min` or `h`, for `kind = elapsed` and `kind = rate`   | required for `elapsed` and `rate` |

| `kind`      | Logs                                                                                                           | Arguments                  | Default `key`            |
| ----------- | -------------------------------------------------------------------------------------------------------------- | -------------------------- | ------------------------ |
| `elapsed`   | `<field>.elapsed()` in whole units                                                                             | `unit`, `key`              | `<field>_elapsed_<unit>` |
| `rate`      | the number divided by `<reference>.elapsed()` in units, rounded to one decimal; `0` before any time has passed | `reference`, `unit`, `key` | `<field>_per_<unit>`     |
| `transform` | the closure's result                                                                                           | `value`, `key`             | none: required           |

The `transform` closure receives the value for a numeric or atomic field and a reference for other fields. Two entries with the same key are a compile error.

```rust
#[derive(SmartDefault, SelfLog)]
struct Totals {
    #[selflog(entry(include = false), extra(kind = elapsed, unit = s, key = elapsed_s))]
    #[default(Instant::now())]
    started: Instant,
    platform: String,
    #[selflog(extra(kind = rate, reference = started, unit = s))]
    scanned: AtomicU64,
    #[selflog(entry(include = false))]
    retries: AtomicU64,
    #[selflog(extra(kind = transform, key = saved_mb, value = |saved| saved as f64 / 1e6))]
    saved: AtomicU64,
}

totals.log_info("Finished");
// Finished elapsed_s=30 platform=Atari_2600 scanned=3615 scanned_per_s=120.5 saved=2500000 saved_mb=2.5
```

The reference field can be of any type with an `elapsed() -> Duration` method, such as `Instant`; how and when it is set is up to the struct's owner. The example uses `smart-default` because `Instant` has no `Default`.
