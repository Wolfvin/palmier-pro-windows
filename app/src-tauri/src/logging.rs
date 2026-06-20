//! Logging helpers.
//!
//! The worker convention (see `context-snapshot/palmier-pro-windows/CONTEXT.md`)
//! requires every error path to log with the format
//! `[module] <apa gagal>: <detail>` via `tlog_err!` / `eprintln!`. We expose a
//! `tlog_err!` macro that delegates to `tracing::error!` (so logs flow through
//! the standard subscriber when one is installed) and falls back to
//! `eprintln!`-shaped output otherwise.
//!
//! Example:
//! ```ignore
//! tlog_err!("mcp", "failed to bind listener", err);
//! // -> [mcp] failed to bind listener: Os { code: 98, kind: AddrInUse, ... }
//! ```

/// Logs an error in the canonical `[module] <what failed>: <detail>` format.
///
/// Usage:
/// - `tlog_err!("mcp", "failed to bind listener", err)` — when you have an
///   `Error` value, its `Display` impl is appended after `: `.
/// - `tlog_err!("mcp", "no editor available")` — when the failure has no
///   underlying error value.
#[macro_export]
macro_rules! tlog_err {
    ($module:expr, $what:expr, $detail:expr $(,)?) => {{
        let _module: &str = $module;
        let _what: &str = $what;
        let _detail = format!("{}", $detail);
        $crate::logging::log_error(_module, _what, || std::borrow::Cow::Owned(_detail));
    }};
    ($module:expr, $what:expr $(,)?) => {{
        let _module: &str = $module;
        let _what: &str = $what;
        $crate::logging::log_error(_module, _what, || std::borrow::Cow::Borrowed(""));
    }};
}

/// Logs an informational message in the canonical `[module] <what>: <detail>`
/// format. Mirrors `tlog_err!` but at info level.
#[macro_export]
macro_rules! tlog_info {
    ($module:expr, $what:expr, $detail:expr $(,)?) => {{
        let _module: &str = $module;
        let _what: &str = $what;
        let _detail = format!("{}", $detail);
        $crate::logging::log_info(_module, _what, || std::borrow::Cow::Owned(_detail));
    }};
    ($module:expr, $what:expr $(,)?) => {{
        let _module: &str = $module;
        let _what: &str = $what;
        $crate::logging::log_info(_module, _what, || std::borrow::Cow::Borrowed(""));
    }};
}

/// Builds the canonical `[module] what: detail` string used across the worker
/// codebase. Public so that other modules can format consistently when they
/// need to embed the prefix in a struct field or HTTP body.
pub fn format_line(module: &str, what: &str, detail: Option<&str>) -> String {
    match detail {
        Some(d) if !d.is_empty() => format!("[{}] {}: {}", module, what, d),
        _ => format!("[{}] {}", module, what),
    }
}

pub(crate) fn log_error(module: &str, what: &str, detail: impl FnOnce() -> std::borrow::Cow<'static, str>) {
    let d = detail();
    let d_str: &str = &d;
    let line = format_line(module, what, if d_str.is_empty() { None } else { Some(d_str) });
    // tracing if a subscriber is installed, eprintln as fallback / mirror.
    tracing::error!("{line}");
    eprintln!("{line}");
}

pub(crate) fn log_info(module: &str, what: &str, detail: impl FnOnce() -> std::borrow::Cow<'static, str>) {
    let d = detail();
    let d_str: &str = &d;
    let line = format_line(module, what, if d_str.is_empty() { None } else { Some(d_str) });
    tracing::info!("{line}");
    eprintln!("{line}");
}

/// Initializes the global tracing subscriber with sensible defaults for a CLI
/// / desktop process. Safe to call multiple times — subsequent calls are
/// no-ops because `tracing_subscriber::set_global_default` rejects re-init.
///
/// ANSI colors are disabled because our `tlog_err!` / `tlog_info!` macros
/// emit messages that start with `[module]` — when ANSI is enabled the
/// tracing fmt layer's color codes can interact badly with the leading `[`
/// of the message and corrupt the displayed prefix on some terminals.
pub fn init_tracing() {
    let filter = tracing_subscriber::EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info,palmier_pro_windows_lib=debug"));
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_ansi(false)
        .try_init();
}
