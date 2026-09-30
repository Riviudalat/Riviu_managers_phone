//! Opt-in, local phase diagnostics for the production-composer rehearsal.
//! This subscriber does not change any device action or deadline.
use anyhow::Context;
use serde_json::{Map, Value};
use std::{
    fs::{File, OpenOptions},
    io::Write,
    path::PathBuf,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::Instant,
};
use tracing::{
    field::{Field, Visit},
    span::{Attributes, Id, Record},
    Event, Level, Metadata, Subscriber,
};

struct RehearsalTrace {
    file: Mutex<File>,
    started: Instant,
    next_span: AtomicU64,
}
impl RehearsalTrace {
    fn accepts(metadata: &Metadata<'_>) -> bool {
        matches!(*metadata.level(), Level::ERROR | Level::WARN | Level::INFO)
            || (*metadata.level() == Level::DEBUG
                && (metadata.target().starts_with("riviu_android_driver::agent")
                    || metadata
                        .target()
                        .starts_with("riviu_android_driver::session::observation")))
    }
}
#[derive(Default)]
struct Fields(Map<String, Value>);
impl Fields {
    fn insert(&mut self, field: &Field, value: Value) {
        let name = field.name().to_ascii_lowercase();
        let secret = [
            "key",
            "token",
            "password",
            "secret",
            "authorization",
            "credential",
        ]
        .iter()
        .any(|word| name.contains(word));
        self.0.insert(
            field.name().to_owned(),
            if secret {
                Value::String("[redacted]".into())
            } else {
                value
            },
        );
    }
}
impl Visit for Fields {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.insert(field, Value::String(format!("{value:?}")));
    }
    fn record_str(&mut self, field: &Field, value: &str) {
        self.insert(field, Value::String(value.into()));
    }
    fn record_bool(&mut self, field: &Field, value: bool) {
        self.insert(field, Value::Bool(value));
    }
    fn record_u64(&mut self, field: &Field, value: u64) {
        self.insert(field, Value::from(value));
    }
    fn record_i64(&mut self, field: &Field, value: i64) {
        self.insert(field, Value::from(value));
    }
    fn record_f64(&mut self, field: &Field, value: f64) {
        self.insert(field, Value::from(value));
    }
}
impl Subscriber for RehearsalTrace {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        Self::accepts(metadata)
    }
    fn new_span(&self, _: &Attributes<'_>) -> Id {
        Id::from_u64(self.next_span.fetch_add(1, Ordering::Relaxed))
    }
    fn record(&self, _: &Id, _: &Record<'_>) {}
    fn record_follows_from(&self, _: &Id, _: &Id) {}
    fn enter(&self, _: &Id) {}
    fn exit(&self, _: &Id) {}
    fn event(&self, event: &Event<'_>) {
        let metadata = event.metadata();
        if !Self::accepts(metadata) {
            return;
        }
        let mut fields = Fields::default();
        event.record(&mut fields);
        let row = serde_json::json!({
            "at": chrono::Utc::now().to_rfc3339(),
            "elapsedMs": self.started.elapsed().as_millis(),
            "level": metadata.level().as_str(), "target": metadata.target(),
            "name": metadata.name(), "file": metadata.file(), "line": metadata.line(),
            "fields": fields.0,
        });
        if let (Ok(mut line), Ok(mut file)) = (serde_json::to_vec(&row), self.file.lock()) {
            line.push(b'\n');
            if let Err(error) = file.write_all(&line) {
                // Diagnostics cannot change completed effects or retry policy.
                eprintln!("rehearsal trace write failed: {error}");
            }
        }
    }
}
pub(super) fn install_from_environment() -> anyhow::Result<()> {
    let Some(path) = std::env::var_os("RIVIU_REHEARSAL_TRACE") else {
        return Ok(());
    };
    let path = PathBuf::from(path);
    anyhow::ensure!(
        path.is_absolute(),
        "RIVIU_REHEARSAL_TRACE must be an absolute local path"
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create rehearsal trace directory")?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path)
        .context("create a new rehearsal trace file")?;
    tracing::subscriber::set_global_default(RehearsalTrace {
        file: Mutex::new(file),
        started: Instant::now(),
        next_span: AtomicU64::new(1),
    })
    .context("install rehearsal diagnostic subscriber")?;
    tracing::info!(path = %path.display(), "rehearsal phase diagnostics enabled");
    Ok(())
}
