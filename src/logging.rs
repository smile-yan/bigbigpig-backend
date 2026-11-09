//! Logging setup.
//!
//! Mirrors the Go logger: a human-readable console output (DEBUG and above)
//! plus a JSON file per local day at `logs/YYYY-MM-DD.log` (INFO and above).
//! `tracing-appender` rotates on UTC dates, so the file writer is implemented
//! here to keep the Go file names and roll over at local midnight.

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::Context;
use tracing_subscriber::fmt::time::ChronoLocal;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{Layer, filter::LevelFilter};

const LOG_DIR: &str = "./logs";
const TIME_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

pub fn init() -> anyhow::Result<()> {
    let file_writer = DailyFileWriter::new(LOG_DIR).context("Failed to open log file")?;
    let timer = ChronoLocal::new(TIME_FORMAT.to_string());

    let console_layer = tracing_subscriber::fmt::layer()
        .with_writer(io::stdout)
        .with_ansi(true)
        .with_target(false)
        .with_file(true)
        .with_line_number(true)
        .with_timer(timer.clone())
        .with_filter(LevelFilter::DEBUG);

    let file_layer = tracing_subscriber::fmt::layer()
        .json()
        .with_writer(file_writer)
        .with_timer(timer)
        .with_filter(LevelFilter::INFO);

    tracing_subscriber::registry()
        .with(console_layer)
        .with(file_layer)
        .try_init()
        .context("failed to initialize tracing")?;

    Ok(())
}

/// Local-time daily file writer producing `logs/YYYY-MM-DD.log`.
#[derive(Clone)]
struct DailyFileWriter {
    dir: Arc<PathBuf>,
    state: Arc<Mutex<DailyFile>>,
}

struct DailyFile {
    date: String,
    file: File,
}

impl DailyFileWriter {
    fn new(dir: impl AsRef<Path>) -> io::Result<Self> {
        let dir = dir.as_ref().to_path_buf();
        std::fs::create_dir_all(&dir)?;
        let date = today();
        let file = open_log_file(&dir, &date)?;
        Ok(Self {
            dir: Arc::new(dir),
            state: Arc::new(Mutex::new(DailyFile { date, file })),
        })
    }

    fn write(&self, buf: &[u8]) -> io::Result<usize> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let date = today();
        if state.date != date {
            state.file = open_log_file(&self.dir, &date)?;
            state.date = date;
        }
        state.file.write(buf)
    }

    fn flush(&self) -> io::Result<()> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        state.file.flush()
    }
}

fn today() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

fn open_log_file(dir: &Path, date: &str) -> io::Result<File> {
    OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join(format!("{date}.log")))
}

struct DailyFileWriterGuard<'a>(&'a DailyFileWriter);

impl Write for DailyFileWriterGuard<'_> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl<'a> tracing_subscriber::fmt::writer::MakeWriter<'a> for DailyFileWriter {
    type Writer = DailyFileWriterGuard<'a>;

    fn make_writer(&'a self) -> Self::Writer {
        DailyFileWriterGuard(self)
    }
}
