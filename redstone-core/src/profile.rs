// redstone-core/src/profile.rs
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub args: Option<Vec<String>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub server_dir: Option<String>,

    #[serde(default = "default_jar")]
    pub jar: String,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<MemoryConfig>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jvm_args: Option<Vec<String>>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_restart: Option<bool>,

    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auto_restart_delay: Option<u64>,

    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_jar() -> String {
    "server.jar".to_string()
}

fn default_port() -> u16 {
    25565
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryConfig {
    pub min: String,
    pub max: String,
}

#[derive(Debug)]
pub struct ResolvedProfile {
    pub inner: Profile,
    pub base_dir: PathBuf,
}

impl ResolvedProfile {
    pub fn load(path: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let content = std::fs::read_to_string(path)?;
        let mut inner: Profile = serde_yaml::from_str(&content)?;
        if let Some(ref cmd) = inner.command {
            let p = Path::new(cmd);
            let resolved = if p.is_absolute() {
                p.to_path_buf()
            } else {
                path.parent().unwrap_or(Path::new(".")).join(cmd)
            };
            inner.command = Some(
                std::fs::canonicalize(&resolved)
                    .unwrap_or(resolved)
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        let base_dir = path
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| PathBuf::from("."));
        Ok(Self { inner, base_dir })
    }

    pub fn jar_path(&self) -> PathBuf {
        let p = Path::new(&self.inner.jar);
        let resolved = if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.base_dir.join(p)
        };
        std::fs::canonicalize(&resolved).unwrap_or(resolved)
    }

    pub fn server_dir(&self) -> PathBuf {
        let resolved = match &self.inner.server_dir {
            Some(dir) => {
                let p = Path::new(dir);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    self.base_dir.join(p)
                }
            }
            None => self.base_dir.clone(),
        };
        std::fs::canonicalize(&resolved).unwrap_or(resolved)
    }

    pub fn build_java_command(&self) -> tokio::process::Command {
        use std::process::Stdio;

        if let Some(ref command) = self.inner.command {
            let mut cmd = tokio::process::Command::new(command);
            if let Some(ref args) = self.inner.args {
                cmd.args(args);
            }
            cmd.current_dir(self.server_dir())
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());

            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                cmd.as_std_mut()
                    .arg0(format!("[redstone] {} (native)", self.inner.name));
            }

            return cmd;
        }

        let mut cmd = tokio::process::Command::new("java");

        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.as_std_mut()
                .arg0(format!("[redstone] {} (java)", self.inner.name));
        }

        if let Some(ref mem) = self.inner.memory {
            cmd.arg(format!("-Xmx{}", mem.max));
            cmd.arg(format!("-Xms{}", mem.min));
        } else {
            cmd.arg("-Xmx2G");
            cmd.arg("-Xms2G");
        }

        if let Some(ref args) = self.inner.jvm_args {
            for arg in args {
                cmd.arg(arg);
            }
        }

        cmd.arg("-jar")
            .arg(self.jar_path())
            .arg("nogui")
            .current_dir(self.server_dir())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        cmd
    }

    pub fn write_canonical(&self, dest: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let mut out = self.inner.clone();
        out.jar = self.jar_path().to_string_lossy().into_owned();
        out.server_dir = Some(self.server_dir().to_string_lossy().into_owned());
        let yaml = serde_yaml::to_string(&out)?;
        std::fs::write(dest, yaml)?;
        Ok(())
    }
}

pub fn redstone_dir() -> PathBuf {
    dirs::config_dir()
        .unwrap_or_else(|| dirs::home_dir().unwrap_or_else(|| PathBuf::from(".")))
        .join("redstone")
}

pub fn default_profile_dir() -> PathBuf {
    redstone_dir().join("profiles")
}

pub fn default_log_dir() -> PathBuf {
    redstone_dir().join("logs")
}

pub fn validate_profile_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Profile name cannot be empty".to_string());
    }
    for c in name.chars() {
        if !c.is_ascii_alphanumeric() && c != '_' && c != '-' {
            return Err(format!(
                "Invalid character '{}' in profile name. Only letters, numbers, '_', and '-' are allowed.",
                c
            ));
        }
    }
    Ok(())
}

pub fn profile_data_dir(name: &str) -> PathBuf {
    default_profile_dir().join(name)
}

pub fn profile_log_dir(name: &str) -> PathBuf {
    profile_data_dir(name).join("log")
}

pub fn profile_log_path(name: &str) -> PathBuf {
    profile_log_dir(name).join("server.log")
}

pub fn profile_yaml_path(name: &str) -> PathBuf {
    default_profile_dir().join(format!("{}.yaml", name))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerState {
    pub name: String,
    pub pid: Option<u32>,
    pub running: bool,
    pub started_at: Option<u64>,
    pub stopped_at: Option<u64>,
    pub port: u16,
}

pub fn read_server_state(name: &str) -> Result<Option<ServerState>, Box<dyn std::error::Error>> {
    let p = profile_data_dir(name).join("server.json");
    if !p.exists() {
        return Ok(None);
    }
    let content = std::fs::read_to_string(p)?;
    let state: ServerState = serde_json::from_str(&content)?;
    Ok(Some(state))
}

pub fn write_server_state(
    name: &str,
    state: &ServerState,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = profile_data_dir(name);
    std::fs::create_dir_all(&dir)?;
    let file = std::fs::File::create(dir.join("server.json"))?;
    serde_json::to_writer_pretty(file, state)?;
    Ok(())
}

pub fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub struct ProfileEntry {
    pub name: String,
    pub yaml_path: PathBuf,
}

pub fn list_all_profiles() -> Vec<ProfileEntry> {
    let dir = default_profile_dir();
    if !dir.exists() {
        return vec![];
    }
    let mut entries = vec![];
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "yaml")
                && let Some(name) = path.file_stem().and_then(|s| s.to_str())
            {
                entries.push(ProfileEntry {
                    name: name.to_string(),
                    yaml_path: path,
                });
            }
        }
    }
    entries.sort_by(|a, b| a.name.cmp(&b.name));
    entries
}

pub fn resolve_profile_name(name_or_path: &str) -> String {
    if let Ok(path) = find_profile(name_or_path)
        && let Ok(profile) = ResolvedProfile::load(&path)
    {
        return profile.inner.name;
    }
    name_or_path.to_string()
}

pub fn find_profile(name_or_path: &str) -> Result<PathBuf, String> {
    let p = Path::new(name_or_path);

    if p.is_absolute() || name_or_path.contains('/') || name_or_path.contains('\\') {
        if p.exists() {
            return Ok(p.to_path_buf());
        }
        return Err(format!("Path '{}' not found", name_or_path));
    }

    let candidates = vec![
        PathBuf::from(format!("{}.yaml", name_or_path)),
        PathBuf::from(name_or_path).join(format!("{}.yaml", name_or_path)),
    ];

    for c in candidates {
        if c.exists() {
            return Ok(c);
        }
    }

    let dir_entry = default_profile_dir().join(format!("{}.yaml", name_or_path));
    if dir_entry.exists() {
        return Ok(dir_entry);
    }

    Err(format!(
        "profile '{}' not found in {}",
        name_or_path,
        default_profile_dir().display()
    ))
}
