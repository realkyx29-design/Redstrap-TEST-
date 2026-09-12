//! Tracing setup: daily-rotated log files plus an in-memory ring.
//!
//! The file appender is non-blocking (a background thread does the I/O, so
//! logging never stalls launches). The ring buffer keeps the last N events
//! for the settings UI's log viewer without touching the disk.

use std::path::Path;
use std::sync::{Arc, Mutex};

use tracing_subscriber::fmt::MakeWriter;

use crate::error::Result;

/// One captured log line for the in-app viewer.
#[derive(Debug, Clone)]
pub struct LogLine {
    pub level: String,
    pub target: String,
    pub message: String,
}

/// Fixed-capacity ring buffer shared with the log writer.
#[derive(Debug, Clone)]
pub struct LogRing {
    inner: Arc<Mutex<RingInner>>,
}

#[derive(Debug)]
struct RingInner {
    lines: std::collections::VecDeque<LogLine>,
    capacity: usize,
}

impl LogRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(RingInner {
                lines: std::collections::VecDeque::new(),
                capacity: capacity.max(16),
            })),
        }
    }

    pub fn push(&self, line: LogLine) {
        let mut inner = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        if inner.lines.len() >= inner.capacity {
            inner.lines.pop_front();
        }
        inner.lines.push_back(line);
    }

    pub fn snapshot(&self) -> Vec<LogLine> {
        match self.inner.lock() {
            Ok(g) => g.lines.iter().cloned().collect(),
            Err(poisoned) => poisoned.into_inner().lines.iter().cloned().collect(),
        }
    }

    pub fn clear(&self) {
        match self.inner.lock() {
            Ok(mut g) => g.lines.clear(),
            Err(poisoned) => poisoned.into_inner().lines.clear(),
        }
    }
}

struct RingWriter {
    ring: LogRing,
    level: &'static str,
    target: String,
    buffer: Vec<u8>,
}

impl std::io::Write for RingWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.buffer.extend_from_slice(bytes);
        while let Some(pos) = self.buffer.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buffer.drain(..=pos).collect();
            let message = String::from_utf8_lossy(&line).trim_end().to_string();
            if !message.is_empty() {
                self.ring.push(LogLine {
                    level: self.level.to_string(),
                    target: self.target.clone(),
                    message,
                });
            }
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

struct RingMakeWriter {
    ring: LogRing,
}

impl<'a> MakeWriter<'a> for RingMakeWriter {
    type Writer = RingWriter;

    fn make_writer(&'a self) -> Self::Writer {
        RingWriter {
            ring: self.ring.clone(),
            level: "INFO",
            target: String::new(),
            buffer: Vec::new(),
        }
    }

    fn make_writer_for(&'a self, meta: &tracing::Metadata<'a>) -> Self::Writer {
        RingWriter {
            ring: self.ring.clone(),
            level: meta.level().as_str(),
            target: meta.target().to_string(),
            buffer: Vec::new(),
        }
    }
}

/// Initialize global logging: file appender + ring buffer.
/// Returns the ring and a guard that must stay alive for the process
/// lifetime (dropping it flushes the background log thread).
///
/// `verbose` selects debug-level output; otherwise info for Red Strap
/// targets and warnings for dependencies. `RUST_LOG` overrides both.
pub fn init(
    logs_dir: &Path,
    file_prefix: &str,
    ring_capacity: usize,
    verbose: bool,
) -> Result<(LogRing, LogGuard)> {
    let ring = LogRing::new(ring_capacity);

    std::fs::create_dir_all(logs_dir).map_err(|e| crate::error::Error::with_path(&logs_dir.to_path_buf(), e))?;
    let file_appender = tracing_appender::rolling::daily(logs_dir, file_prefix);
    let (file_writer, guard) = tracing_appender::non_blocking(file_appender);

    let ring_maker = RingMakeWriter { ring: ring.clone() };
    // Tee: every event goes to both the file and the ring.
    let tee = TeeMaker {
        file: file_writer,
        ring: ring_maker,
    };

    let default_directive = if verbose { "redstrap=debug,debug" } else { "redstrap=info,warn" };
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(default_directive));

    let subscriber = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(tee)
        .with_ansi(false)
        .with_target(true)
        .finish();

    // `try_init` so tests and re-entry don't panic.
    let _ = tracing::subscriber::set_global_default(subscriber);

    Ok((ring, LogGuard { _guard: guard }))
}

/// Keeps the non-blocking file writer alive.
pub struct LogGuard {
    _guard: tracing_appender::non_blocking::WorkerGuard,
}

struct TeeMaker<F, R> {
    file: F,
    ring: R,
}

struct TeeWriter<F, R> {
    file: F,
    ring: R,
}

impl<F: std::io::Write, R: std::io::Write> std::io::Write for TeeWriter<F, R> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let n = self.file.write(bytes)?;
        let _ = self.ring.write(&bytes[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        let _ = self.ring.flush();
        self.file.flush()
    }
}

impl<'a, F, R> MakeWriter<'a> for TeeMaker<F, R>
where
    F: MakeWriter<'a>,
    R: MakeWriter<'a>,
{
    type Writer = TeeWriter<F::Writer, R::Writer>;

    fn make_writer(&'a self) -> Self::Writer {
        TeeWriter {
            file: self.file.make_writer(),
            ring: self.ring.make_writer(),
        }
    }

    fn make_writer_for(&'a self, meta: &tracing::Metadata<'a>) -> Self::Writer {
        TeeWriter {
            file: self.file.make_writer_for(meta),
            ring: self.ring.make_writer_for(meta),
        }
    }
}
