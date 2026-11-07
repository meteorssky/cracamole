//! Log output in the C guacd format: `guacd[<pid>]: LEVEL:<TAB>message`.

use crate::config::LogLevel;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fmt;
use std::sync::Mutex;
use std::time::SystemTime;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

pub const MAX_LOG_ENTRIES: usize = 1000;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: String,
    pub target: String,
    pub message: String,
    pub raw: String,
}

static LOG_BUFFER: Mutex<Option<VecDeque<LogEntry>>> = Mutex::new(None);

fn current_time_iso8601() -> String {
    let now = SystemTime::now();
    let since_epoch = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let total_secs = since_epoch.as_secs();
    let millis = since_epoch.subsec_millis();

    let days = total_secs / 86400;
    let seconds_of_day = total_secs % 86400;
    let hours = seconds_of_day / 3600;
    let minutes = (seconds_of_day % 3600) / 60;
    let seconds = seconds_of_day % 60;

    let mut year = 1970;
    let mut d = days;
    loop {
        let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
        let days_in_year = if leap { 366 } else { 365 };
        if d < days_in_year {
            break;
        }
        d -= days_in_year;
        year += 1;
    }
    let leap = (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0);
    let days_in_months = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];
    let mut month = 1;
    for &dim in &days_in_months {
        if d < dim {
            break;
        }
        d -= dim;
        month += 1;
    }
    let day = d + 1;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}.{:03}Z",
        year, month, day, hours, minutes, seconds, millis
    )
}

pub fn append_log(entry: LogEntry) {
    let mut lock = match LOG_BUFFER.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let buf = lock.get_or_insert_with(|| VecDeque::with_capacity(MAX_LOG_ENTRIES));
    if buf.len() >= MAX_LOG_ENTRIES {
        buf.pop_front();
    }
    buf.push_back(entry);
}

fn level_severity(lvl: &str) -> Option<u8> {
    match lvl.to_ascii_uppercase().as_str() {
        "ERROR" => Some(5),
        "WARN" | "WARNING" => Some(4),
        "INFO" => Some(3),
        "DEBUG" => Some(2),
        "TRACE" => Some(1),
        _ => None,
    }
}

pub fn recent_logs(limit: Option<usize>, min_level: Option<&str>) -> Vec<LogEntry> {
    let lock = match LOG_BUFFER.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    let Some(buf) = lock.as_ref() else {
        return vec![];
    };
    let limit = limit.unwrap_or(200).clamp(1, MAX_LOG_ENTRIES);
    let min_sev = min_level.and_then(level_severity);

    buf.iter()
        .filter(|e| match min_sev {
            Some(sev) => level_severity(&e.level).unwrap_or(0) >= sev,
            None => true,
        })
        .rev()
        .take(limit)
        .cloned()
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect()
}

pub fn clear_logs() {
    let mut lock = match LOG_BUFFER.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    if let Some(buf) = lock.as_mut() {
        buf.clear();
    }
}

struct GuacdFormat;

struct Message(String);

impl Visit for Message {
    fn record_debug(&mut self, field: &Field, value: &dyn fmt::Debug) {
        if field.name() == "message" {
            self.0 = format!("{value:?}");
        } else {
            if !self.0.is_empty() {
                self.0.push(' ');
            }
            self.0.push_str(&format!("{}={value:?}", field.name()));
        }
    }
}

/// C guacd's level names.
pub fn level_name(l: &Level) -> &'static str {
    match *l {
        Level::ERROR => "ERROR",
        Level::WARN => "WARNING",
        Level::INFO => "INFO",
        Level::DEBUG => "DEBUG",
        Level::TRACE => "TRACE",
    }
}

pub fn format_line(pid: u32, level: &Level, message: &str) -> String {
    format!("guacd[{pid}]: {}:\t{message}", level_name(level))
}

impl<S, N> FormatEvent<S, N> for GuacdFormat
where
    S: Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        _ctx: &FmtContext<'_, S, N>,
        mut w: Writer<'_>,
        ev: &Event<'_>,
    ) -> fmt::Result {
        let mut m = Message(String::new());
        ev.record(&mut m);
        let raw = format_line(std::process::id(), ev.metadata().level(), &m.0);
        let timestamp = current_time_iso8601();
        append_log(LogEntry {
            timestamp,
            level: level_name(ev.metadata().level()).to_string(),
            target: ev.metadata().target().to_string(),
            message: m.0,
            raw: raw.clone(),
        });
        writeln!(w, "{}", raw)
    }
}

/// Install the global subscriber writing to stderr. Safe to call once; later calls are ignored.
pub fn init(level: LogLevel) {
    {
        let mut lock = match LOG_BUFFER.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        };
        if lock.is_none() {
            *lock = Some(VecDeque::with_capacity(MAX_LOG_ENTRIES));
        }
    }
    let _ = tracing_subscriber::fmt()
        .with_max_level(level.tracing_level())
        .with_writer(std::io::stderr)
        .event_format(GuacdFormat)
        .try_init();
}
