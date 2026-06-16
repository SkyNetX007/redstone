// redstone-app/src/main.rs
#[tokio::main]
async fn main() {
    let sys_lang = sys_locale::get_locale().unwrap_or_else(|| "en-US".to_string());
    let locale = redstone_cli::RedstoneConfig::effective_locale(&sys_lang);
    redstone_cli::init_locale(&locale);

    redstone_cli::run_cli().await;
}
