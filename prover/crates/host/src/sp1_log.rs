//! The SP1 proving commands' log, and the SP1 errors that end the process.
//!
//! SP1 6.3's local prover runs a proof as tasks on an in-process worker. When the controller
//! or the executor task fails (the native executor crashing on a full `/dev/shm`, say), the
//! worker logs `Controller: task failed` or `CoreExecute: task failed` and then completes the
//! task as if it had succeeded: nothing reports the failure to the caller, and `prove()` waits
//! for output that never comes until the caller's timeout kills it. A proving command therefore
//! exits with [`FATAL_TASK_EXIT`] on the first such error, whatever `RUST_LOG` lets through.

use tracing::{
    field::{Field, Visit},
    Event, Level, Metadata, Subscriber,
};
use tracing_subscriber::{
    filter::{filter_fn, EnvFilter},
    fmt::format::FmtSpan,
    layer::{Context, SubscriberExt},
    util::SubscriberInitExt,
    Layer,
};

/// The exit status of a proving command that stopped because SP1 failed a task its proof
/// needs.
pub const FATAL_TASK_EXIT: i32 = 70;

/// Where SP1's local worker logs its task failures.
const WORKER_TARGET: &str = "sp1_prover::worker::node::full::init";

/// The start of each message the worker logs for a failure that ends the proof without telling
/// the caller: a controller or executor task that failed, a request it could not read, a task
/// that panicked.
const FATAL_MESSAGES: [&str; 4] = [
    "Controller: task failed",
    "CoreExecute: task failed",
    "CoreExecute: failed to parse request",
    "task panicked",
];

/// Install the global subscriber of a proving command: SP1's log as
/// `sp1_sdk::utils::setup_logger` prints it (compact, on stdout, filtered by `RUST_LOG` and off
/// without it), and an exit with [`FATAL_TASK_EXIT`] on SP1's fatal task errors, which no
/// `RUST_LOG` hides.
pub fn init() {
    let log_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("off"));
    subscriber(log_filter, exit_on_fatal_task_error).init();
}

/// SP1's log through `log_filter`, with `on_fatal` called on each fatal task error.
fn subscriber(log_filter: EnvFilter, on_fatal: fn(&str)) -> impl Subscriber + Send + Sync {
    let log_filter = log_filter
        .add_directive("hyper=off".parse().expect("directive"))
        .add_directive("slop_keccak_air=off".parse().expect("directive"))
        .add_directive("p3_fri=off".parse().expect("directive"))
        .add_directive("p3_dft=off".parse().expect("directive"))
        .add_directive("p3_challenger=off".parse().expect("directive"));
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::fmt::layer()
                .compact()
                .with_file(false)
                .with_target(false)
                .with_thread_names(false)
                .with_span_events(FmtSpan::CLOSE)
                .with_filter(log_filter),
        )
        .with(FatalTaskErrors::layer(on_fatal))
}

fn exit_on_fatal_task_error(message: &str) {
    eprintln!(
        "bridge-return-host: SP1 failed a task the proof needs and will not report it, so the \
         proof cannot finish; exiting with status {FATAL_TASK_EXIT}: {message}"
    );
    std::process::exit(FATAL_TASK_EXIT);
}

/// Hands `on_fatal` the message of each fatal task error SP1's worker logs.
struct FatalTaskErrors {
    on_fatal: fn(&str),
}

impl FatalTaskErrors {
    /// The layer, which sees only the worker's errors, whatever filter the log has.
    fn layer<S>(on_fatal: fn(&str)) -> impl Layer<S>
    where
        S: Subscriber + for<'a> tracing_subscriber::registry::LookupSpan<'a>,
    {
        Self { on_fatal }.with_filter(filter_fn(worker_error))
    }
}

fn worker_error(metadata: &Metadata<'_>) -> bool {
    *metadata.level() == Level::ERROR && metadata.target() == WORKER_TARGET
}

impl<S: Subscriber> Layer<S> for FatalTaskErrors {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let mut message = MessageField(String::new());
        event.record(&mut message);
        if FATAL_MESSAGES.iter().any(|m| message.0.starts_with(m)) {
            (self.on_fatal)(&message.0);
        }
    }
}

struct MessageField(String);

impl Visit for MessageField {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use super::*;

    thread_local! {
        static SEEN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    fn record(message: &str) {
        SEEN.with(|seen| seen.borrow_mut().push(message.to_string()));
    }

    /// Run `log` under the proving commands' subscriber with `RUST_LOG` set to `log_filter`, and
    /// return the fatal task errors it reported.
    fn reported_under(log_filter: &str, log: impl FnOnce()) -> Vec<String> {
        let subscriber = subscriber(EnvFilter::new(log_filter), record);
        tracing::subscriber::with_default(subscriber, log);
        SEEN.with(|seen| seen.take())
    }

    fn reported(log: impl FnOnce()) -> Vec<String> {
        reported_under("off", log)
    }

    #[test]
    fn the_workers_fatal_task_errors_are_reported() {
        let seen = reported(|| {
            tracing::error!(
                target: "sp1_prover::worker::node::full::init",
                "CoreExecute: task failed: {:?}",
                "Fatal(failed to execute chunk: Child native executor crashed, details: CrashDetails { signal: 7, addr: 0, operation: 0 })"
            );
            tracing::error!(target: "sp1_prover::worker::node::full::init", "Controller: task failed: {}", "boom");
            tracing::error!(target: "sp1_prover::worker::node::full::init", "CoreExecute: failed to parse request: {}", "bad");
            tracing::error!(target: "sp1_prover::worker::node::full::init", "task panicked: {}", "oops");
        });
        assert_eq!(seen.len(), 4, "{seen:?}");
        assert!(seen[0].starts_with("CoreExecute: task failed: \"Fatal(failed to execute chunk"));
        assert!(seen[0].contains("signal: 7"));
        assert_eq!(seen[1], "Controller: task failed: boom");
    }

    #[test]
    fn other_errors_warnings_and_other_modules_are_not() {
        let seen = reported(|| {
            // The worker retries this task itself.
            tracing::error!(target: "sp1_prover::worker::node::full::init", "task failed with retryable error: {}", "x");
            tracing::error!(target: "sp1_prover::worker::node::full::init", "Controller: deleting input artifact failed: {}", "x");
            tracing::warn!(target: "sp1_prover::worker::node::full::init", "CoreExecute: task failed: {}", "x");
            tracing::error!(target: "sp1_prover::worker::controller", "CoreExecute: task failed: {}", "x");
            tracing::error!("CoreExecute: task failed: {}", "x");
        });
        assert!(seen.is_empty(), "{seen:?}");
    }

    #[test]
    fn they_are_reported_whatever_the_log_lets_through() {
        for filter in ["off", "error", "info", "sp1_prover=off"] {
            let seen = reported_under(filter, || {
                tracing::error!(target: "sp1_prover::worker::node::full::init", "CoreExecute: task failed: {}", "x");
            });
            assert_eq!(seen, ["CoreExecute: task failed: x"], "RUST_LOG={filter}");
        }
    }
}
