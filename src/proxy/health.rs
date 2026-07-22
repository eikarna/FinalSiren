//! Server-side proxy liveness checker.
//!
//! Probes proxy relays by attempting a TCP connect (the same primitive the live
//! tunnel uses in `conn.rs`) and reporting whether the connection was
//! established within a deadline. The relay list from `proxy_kv` contains plain
//! `ip:port` strings with no per-relay credentials, so a connect-success check
//! is the correct and complete liveness signal — there is no protocol handshake
//! to perform against a transparent TCP relay.
//!
//! Timeouts: the `worker` 0.6.x `Socket` API exposes no built-in connect
//! timeout, and this crate builds tokio without the `time` feature, so the
//! deadline is implemented by racing `opened()` against `worker::Delay` (a
//! WASM-native timer backed by `setTimeout`). Because `worker` pulls in
//! `futures-util` with `default-features = false`, its `select`/`race`
//! combinators are not available, so the race is hand-rolled below.

use std::future::poll_fn;
use std::future::Future;
use std::pin::Pin;
use std::task::Poll;
use std::time::Duration;

use futures_util::stream::StreamExt;
use serde::Serialize;
use worker::{console_log, Date, Delay, Socket};

type BoxFuture<'a, T> = Pin<Box<dyn Future<Output = T> + 'a>>;

/// Race two futures to completion, returning whichever resolves first as an
/// `Either`. A hand-rolled `select` — kept local because `futures-util` is
/// pulled in by `worker` with `default-features = false`, so its `race`/`select`
/// combinators are not compiled in.
enum Either<A, B> {
    Left(A),
    Right(B),
}

async fn race<A, B>(mut left: BoxFuture<'_, A>, mut right: BoxFuture<'_, B>) -> Either<A, B> {
    poll_fn(|cx| {
        // Poll `left` first; on the rare chance both are ready in the same wake,
        // the connect future wins, which is the favorable outcome.
        if let Poll::Ready(a) = left.as_mut().poll(cx) {
            return Poll::Ready(Either::Left(a));
        }
        if let Poll::Ready(b) = right.as_mut().poll(cx) {
            return Poll::Ready(Either::Right(b));
        }
        Poll::Pending
    })
    .await
}

/// Outcome of probing a single proxy relay.
#[derive(Debug, Serialize)]
pub struct ProbeResult {
    pub addr: String,
    pub port: u16,
    pub alive: bool,
    /// Round-trip connect latency in milliseconds. `None` when the relay is
    /// unreachable or the probe exceeded its deadline.
    pub latency_ms: Option<u32>,
}

impl ProbeResult {
    fn dead(addr: String, port: u16) -> Self {
        ProbeResult {
            addr,
            port,
            alive: false,
            latency_ms: None,
        }
    }
}

/// A target to probe, parsed from an `ip:port` (or `host:port`) relay string.
#[derive(Debug, Clone)]
pub struct Target {
    pub addr: String,
    pub port: u16,
}

/// Parse an `ip:port` / `host:port` relay entry into a [`Target`].
///
/// The tunnel stores relays with a `:` separator and rewrites them to `-`
/// before proxying (see `lib.rs`); this checker keeps the raw `:` form, which
/// is what `Socket::connect` expects. Entries are `[host]:port` for IPv6 and
/// `host:port` otherwise.
pub fn parse_target(raw: &str) -> Option<Target> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    // Bracketed IPv6 literal: [::1]:443
    if let Some(rest) = raw.strip_prefix('[') {
        let (host, after) = rest.split_once(']')?;
        let port = after.trim_start_matches(':').parse::<u16>().ok()?;
        return Some(Target {
            addr: host.to_string(),
            port,
        });
    }
    let (host, port) = raw.rsplit_once(':')?;
    // Reject ambiguous unbracketed IPv6 (contains multiple ':').
    if host.contains(':') {
        return None;
    }
    let port = port.parse::<u16>().ok()?;
    if host.is_empty() {
        return None;
    }
    Some(Target {
        addr: host.to_string(),
        port,
    })
}

/// Probe one relay. Resolves `alive: true` only when the TCP connection
/// completes within `timeout_ms`.
///
/// `Delay` is not cancellable, so on a fast connect the timer future is simply
/// dropped (unawaited) once `opened()` wins the race — it is a one-shot
/// `setTimeout` that harmlessly fires later with no waker attached.
pub async fn probe(target: Target, timeout_ms: u32) -> ProbeResult {
    let started = Date::now().as_millis();

    // Step 1: open the socket object. This is the JS `connect()` call; it can
    // fail immediately for malformed addresses or blocked ports (e.g. 25).
    let mut socket = match Socket::builder().connect(target.addr.clone(), target.port) {
        Ok(s) => s,
        Err(e) => {
            console_log!("[health] connect() rejected {}: {}", target.addr, e);
            return ProbeResult::dead(target.addr, target.port);
        }
    };

    // Step 2: race the actual handshake (`opened()`) against the deadline.
    // `opened()` winning → connected; `Delay` winning → timed out.
    let opened_fut: BoxFuture<worker::Result<worker::SocketInfo>> =
        Box::pin(socket.opened());
    let timer_fut: BoxFuture<()> =
        Box::pin(Delay::from(Duration::from_millis(timeout_ms as u64)));

    let connected = matches!(race(opened_fut, timer_fut).await, Either::Left(Ok(_)));

    // Always close: on success we don't need the socket, on failure/timeout we
    // want to release the underlying connection promptly.
    let _ = socket.close().await;

    if connected {
        let latency = (Date::now().as_millis() - started) as u32;
        ProbeResult {
            addr: target.addr,
            port: target.port,
            alive: true,
            latency_ms: Some(latency),
        }
    } else {
        ProbeResult::dead(target.addr, target.port)
    }
}

/// Probe many targets concurrently with a bounded number of in-flight
/// connections, returning results in input order.
///
/// Bounded concurrency keeps us within Cloudflare's guidance for simultaneous
/// outbound sockets per request and prevents a single slow/dead relay from
/// serializing the whole sweep.
pub async fn probe_all(targets: Vec<Target>, concurrency: usize, timeout_ms: u32) -> Vec<ProbeResult> {
    // Enumerate so we can place each result back at its original index, since
    // buffer_unordered yields results in completion order.
    let total = targets.len();
    let mut results: Vec<Option<ProbeResult>> = (0..total).map(|_| None).collect();

    let stream = futures_util::stream::iter(targets.into_iter().enumerate())
        .map(|(i, target)| async move {
            let res = probe(target, timeout_ms).await;
            (i, res)
        })
        .buffer_unordered(concurrency);

    stream
        .for_each(|(i, res)| {
            results[i] = Some(res);
            std::future::ready(())
        })
        .await;

    results.into_iter().flatten().collect()
}
