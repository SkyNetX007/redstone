// redstone-cli/src/lib.rs
use clap::{FromArgMatches, Parser, Subcommand, ValueEnum};
pub use redstone_core::config::RedstoneConfig;
pub use redstone_core::init_locale;
use rust_i18n::t;

rust_i18n::i18n!("../redstone-i18n/locales", fallback = "en");

mod cmd;

#[derive(Parser)]
#[command(name = "redstone", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    #[command(about = "Start a Minecraft server in background")]
    Start {
        #[arg(help = "Profile name or path")]
        profile: String,
    },
    #[command(about = "Gracefully stop the server")]
    Stop {
        #[arg(help = "Profile name")]
        profile: String,
        #[arg(long, short, help = "Wait for server to fully stop")]
        wait: bool,
        #[arg(long, help = "Max seconds to wait before force-kill [default: 30]")]
        timeout: Option<u64>,
    },
    #[command(about = "Force kill the server")]
    Kill {
        #[arg(help = "Profile name")]
        profile: String,
    },
    #[command(about = "Restart the server")]
    Restart {
        #[arg(help = "Profile name")]
        profile: String,
    },
    #[command(about = "Query server status")]
    Status {
        #[arg(help = "Profile name")]
        profile: String,
    },
    #[command(about = "Attach to server console")]
    Attach {
        #[arg(help = "Profile name")]
        profile: String,
    },
    #[command(about = "List all registered servers")]
    List {
        #[arg(long, help = "Show only online servers")]
        online: bool,
        #[arg(long, help = "Show only offline servers")]
        offline: bool,
    },
    #[command(about = "Remove a server profile and data")]
    Rm {
        #[arg(help = "Profile name")]
        profile: String,
        #[arg(long, short, help = "Force remove even if running")]
        force: bool,
    },
    #[command(about = "Rename a profile")]
    Rename {
        #[arg(help = "Current profile name")]
        from: String,
        #[arg(help = "New profile name")]
        to: String,
    },
    #[command(about = "Follow server log")]
    Log {
        #[arg(help = "Profile name")]
        profile: String,
        #[arg(long, short, help = "Follow new log entries")]
        follow: bool,
    },
    #[command(about = "Generate shell completions")]
    Completion {
        #[arg(value_enum, help = "Shell type")]
        shell: Shell,
    },
    #[command(about = "Create a profile template")]
    Init {
        #[arg(value_enum, help = "Server type")]
        server_type: InitType,
        #[arg(short, long, help = "Output path [default: stdout]")]
        output: Option<String>,
    },
    #[command(about = "Get global config value")]
    Get {
        #[arg(help = "Config key (e.g. locale, default_profile)")]
        key: Option<String>,
        #[arg(short, long, help = "Show all values")]
        all: bool,
    },
    #[command(about = "Set a global config value")]
    Set {
        #[arg(help = "Config key (e.g. locale)")]
        key: String,
        #[arg(help = "Value to set")]
        value: String,
    },
    #[command(name = "_daemon", hide = true)]
    InternalDaemon {
        #[arg(help = "Path to profile YAML")]
        yaml_path: String,
    },
}

#[derive(Copy, Clone, ValueEnum)]
enum Shell {
    Bash,
    Fish,
    Zsh,
}

#[derive(Copy, Clone, ValueEnum)]
enum InitType {
    #[clap(name = "minecraft")]
    Minecraft,
    #[clap(name = "cmd")]
    Cmd,
}

pub async fn run_cli() {
    use clap::CommandFactory;
    let mut cmd = Cli::command().about(t!("app.about").to_string());

    cmd = cmd
        .mut_subcommand("start", |s| s.about(t!("app.cli.start_desc")))
        .mut_subcommand("stop", |s| s.about(t!("app.cli.stop_desc")))
        .mut_subcommand("kill", |s| s.about(t!("app.cli.kill_desc")))
        .mut_subcommand("restart", |s| s.about(t!("app.cli.restart_desc")))
        .mut_subcommand("status", |s| s.about(t!("app.cli.status_desc")))
        .mut_subcommand("attach", |s| s.about(t!("app.cli.attach_desc")))
        .mut_subcommand("list", |s| s.about(t!("app.cli.list_desc")))
        .mut_subcommand("rm", |s| s.about(t!("app.cli.rm_desc")))
        .mut_subcommand("rename", |s| s.about(t!("app.cli.rename_desc")))
        .mut_subcommand("log", |s| s.about(t!("app.cli.log_desc")))
        .mut_subcommand("completion", |s| s.about(t!("app.cli.completion_desc")))
        .mut_subcommand("init", |s| s.about(t!("app.cli.init_desc")))
        .mut_subcommand("get", |s| s.about(t!("app.cli.get_desc")))
        .mut_subcommand("set", |s| s.about(t!("app.cli.set_desc")));

    let matches = cmd.get_matches();
    let cli = Cli::from_arg_matches(&matches).unwrap_or_else(|e| e.exit());

    match cli.command {
        Some(Commands::Start { profile }) => cmd::start_cmd(&profile).await,
        Some(Commands::Stop {
            profile,
            wait,
            timeout,
        }) => cmd::stop_cmd(&profile, wait, timeout).await,
        Some(Commands::Kill { profile }) => cmd::kill_cmd(&profile).await,
        Some(Commands::Restart { profile }) => cmd::restart_cmd(&profile).await,
        Some(Commands::Status { profile }) => cmd::status_cmd(&profile).await,
        Some(Commands::Attach { profile }) => cmd::attach_cmd(&profile).await,
        Some(Commands::List { online, offline }) => cmd::list_cmd(online, offline).await,
        Some(Commands::Rm { profile, force }) => cmd::rm_cmd(&profile, force).await,
        Some(Commands::Rename { from, to }) => cmd::rename_cmd(&from, &to).await,
        Some(Commands::Log { profile, follow }) => cmd::log_cmd(&profile, follow).await,
        Some(Commands::Completion { shell }) => cmd::completion_cmd(shell),
        Some(Commands::Init {
            server_type,
            output,
        }) => cmd::init_cmd(server_type, output),
        Some(Commands::Get { key, all }) => cmd::get_cmd(key, all),
        Some(Commands::Set { key, value }) => cmd::set_cmd(&key, &value),
        Some(Commands::InternalDaemon { yaml_path }) => cmd::_daemon_cmd(&yaml_path).await,
        None => println!("{}", t!("app.desktop.start")),
    }
}
