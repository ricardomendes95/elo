//! Logging: every `tracing` event goes to a daily file
//! (`<dir>/logs/elo-YYYY-MM-DD.log`) and to an in-memory ring buffer shown in
//! Configurações → Logs. Old files (> 14 days) are pruned at start.

use std::collections::VecDeque;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const RING: usize = 2000;

struct State {
    dir: PathBuf,
    lines: Mutex<VecDeque<String>>,
}

static STATE: OnceLock<State> = OnceLock::new();

/// Days since 1970-01-01 → (year, month, day) (civil calendar, UTC).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + if m <= 2 { 1 } else { 0 }, m, d)
}

fn today() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let (y, m, d) = civil(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// Directory holding the log files.
pub fn log_dir() -> PathBuf {
    STATE
        .get()
        .map(|s| s.dir.clone())
        .unwrap_or_else(|| crate::config::config_dir().join("logs"))
}

/// The most recent log lines (oldest first).
pub fn recent() -> Vec<String> {
    STATE
        .get()
        .map(|s| s.lines.lock().unwrap().iter().cloned().collect())
        .unwrap_or_default()
}

/// Clear the in-memory view (files are kept).
pub fn clear_view() {
    if let Some(s) = STATE.get() {
        s.lines.lock().unwrap().clear();
    }
}

struct LineWriter {
    buf: Vec<u8>,
}

impl Write for LineWriter {
    fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(b);
        Ok(b.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Drop for LineWriter {
    fn drop(&mut self) {
        let Some(st) = STATE.get() else { return };
        if self.buf.is_empty() {
            return;
        }
        let path = st.dir.join(format!("elo-{}.log", today()));
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
            let _ = f.write_all(&self.buf);
        }
        let text = String::from_utf8_lossy(&self.buf);
        let mut lines = st.lines.lock().unwrap();
        for l in text.lines().filter(|l| !l.trim().is_empty()) {
            if lines.len() >= RING {
                lines.pop_front();
            }
            lines.push_back(l.to_string());
        }
    }
}

fn prune(dir: &Path) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    let limit = std::time::Duration::from_secs(14 * 86_400);
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if !(name.starts_with("elo-") && name.ends_with(".log")) {
            continue;
        }
        if let Ok(meta) = e.metadata() {
            if meta.modified().ok().and_then(|t| t.elapsed().ok()).is_some_and(|age| age > limit) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Install the global logger writing under `base/logs`. Safe to call once.
pub fn init(base: &Path) {
    let dir = base.join("logs");
    let _ = std::fs::create_dir_all(&dir);
    prune(&dir);
    if STATE.set(State { dir, lines: Mutex::new(VecDeque::new()) }).is_err() {
        return;
    }
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| "info,iroh=warn,iroh_relay=warn,netwatch=warn,portmapper=warn,swarm_discovery=warn,hickory=warn,quinn=warn,quinn_proto=warn,wgpu=warn,eframe=warn,egui_glow=warn".into());
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_ansi(false)
        .with_target(false)
        .with_writer(|| LineWriter { buf: Vec::new() })
        .try_init();
    tracing::info!("Elo {} iniciado", env!("CARGO_PKG_VERSION"));
}

#[cfg(test)]
mod tests {
    #[test]
    fn civil_dates() {
        assert_eq!(super::civil(0), (1970, 1, 1));
        assert_eq!(super::civil(19_723), (2024, 1, 1));
        assert_eq!(super::civil(20_778), (2026, 11, 21));
    }
}
