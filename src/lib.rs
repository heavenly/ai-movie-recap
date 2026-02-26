use once_cell::sync::Lazy;
use std::sync::{Arc, Mutex};

pub mod api;
pub mod clip_plan;
pub mod config;
pub mod ffmpeg;
pub mod generator;
pub mod platform;

pub type GeneratorLogHook = Arc<Mutex<dyn Fn(&str) + Send + Sync + 'static>>;

static LOG_HOOK: Lazy<Mutex<Option<GeneratorLogHook>>> = Lazy::new(|| Mutex::new(None));

pub fn set_log_hook(hook: Option<GeneratorLogHook>) {
    if let Ok(mut guard) = LOG_HOOK.lock() {
        *guard = hook;
    }
}

pub(crate) fn logv(tag: &str, message: &str) {
    eprintln!("[{}] {}", tag, message);

    if let Ok(guard) = LOG_HOOK.lock() {
        if let Some(hook) = guard.as_ref() {
            if let Ok(callback) = hook.lock() {
                let line = format!("[{}] {}", tag, message);
                callback(&line);
            }
        }
    }
}

pub(crate) fn logi(message: impl AsRef<str>) {
    logv("INFO", message.as_ref());
}

pub(crate) fn logok(message: impl AsRef<str>) {
    logv("OK", message.as_ref());
}

pub(crate) fn logw(message: impl AsRef<str>) {
    logv("WARN", message.as_ref());
}

pub mod init;
