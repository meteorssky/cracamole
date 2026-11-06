//! Log output in the C guacd format: `guacd[<pid>]: LEVEL:<TAB>message`.

use crate::config::LogLevel;
use std::fmt;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::fmt::format::Writer;
use tracing_subscriber::fmt::{FmtContext, FormatEvent, FormatFields};
use tracing_subscriber::registry::LookupSpan;

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
        writeln!(
            w,
            "{}",
            format_line(std::process::id(), ev.metadata().level(), &m.0)
        )
    }
}

/// Install the global subscriber writing to stderr. Safe to call once; later calls are ignored.
pub fn init(level: LogLevel) {
    let _ = tracing_subscriber::fmt()
        .with_max_level(level.tracing_level())
        .with_writer(std::io::stderr)
        .event_format(GuacdFormat)
        .try_init();
}
