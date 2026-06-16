// redstone-core/src/lib.rs
pub mod config;
pub mod ipc;
pub mod profile;
pub mod slp;

rust_i18n::i18n!("../redstone-i18n/locales", fallback = "en");

pub fn init_locale(locale: &str) {
    rust_i18n::set_locale(locale);
}
