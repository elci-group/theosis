use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Version {
    major: u64,
    minor: u64,
    patch: u64,
}

impl Version {
    fn parse(raw: &str) -> Result<Self, String> {
        let value = raw.trim().trim_start_matches('v');
        let mut parts = value.split('.');
        let parse = |part: Option<&str>, name: &str| {
            part.ok_or_else(|| format!("missing {name} component"))
                .and_then(|v| {
                    v.parse::<u64>()
                        .map_err(|_| format!("invalid {name} component"))
                })
        };
        let version = Self {
            major: parse(parts.next(), "major")?,
            minor: parse(parts.next(), "minor")?,
            patch: parse(parts.next(), "patch")?,
        };
        if parts.next().is_some() {
            return Err("version must contain exactly A.B.C".into());
        }
        Ok(version)
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

#[derive(Debug)]
enum Error {
    Io(io::Error),
    Command(String),
    Version(String),
    Config(String),
}
impl From<io::Error> for Error {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "I/O failure: {e}"),
            Self::Command(e) => write!(f, "command failure: {e}"),
            Self::Version(e) => write!(f, "version failure: {e}"),
            Self::Config(e) => write!(f, "configuration failure: {e}"),
        }
    }
}

fn run(program: &str, args: &[&str], cwd: Option<&Path>) -> Result<String, Error> {
    let mut cmd = Command::new(program);
    cmd.args(args);
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    let out = cmd
        .output()
        .map_err(|e| Error::Command(format!("{program}: {e}")))?;
    if !out.status.success() {
        return Err(Error::Command(format!(
            "{program} {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn installed_version(binary: &Path) -> Result<Version, Error> {
    let path = binary.to_string_lossy();
    let output = run(&path, &["--version"], None)?;
    let token = output
        .split_whitespace()
        .find(|v| Version::parse(v).is_ok())
        .ok_or_else(|| Error::Version(format!("{path} did not report A.B.C")))?;
    Version::parse(token).map_err(Error::Version)
}

fn cargo_version(source: &Path) -> Result<Version, Error> {
    let file = fs::read_to_string(source.join("Cargo.toml"))?;
    let package = file
        .lines()
        .skip_while(|line| line.trim() != "[package]")
        .skip(1)
        .take_while(|line| !line.trim_start().starts_with('['))
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            if key.trim() == "version" {
                Some(
                    value
                        .trim()
                        .trim_matches('"')
                        .trim_matches('\'')
                        .to_string(),
                )
            } else {
                None
            }
        })
        .ok_or_else(|| Error::Version("Cargo.toml has no [package] version".into()))?;
    Version::parse(&package).map_err(Error::Version)
}

fn temp_workspace() -> Result<PathBuf, Error> {
    let base = env::var_os("THEOSIS_STATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(".theosis"));
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|e| Error::Config(e.to_string()))?
        .as_millis();
    let path = base.join("transactions").join(stamp.to_string());
    fs::create_dir_all(&path)?;
    Ok(path)
}

fn json_escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
}

fn main() -> ExitCode {
    match execute() {
        Ok(code) => ExitCode::from(code),
        Err(e) => {
            eprintln!("theosis: {e}");
            ExitCode::from(1)
        }
    }
}

fn execute() -> Result<u8, Error> {
    let mut args = env::args().skip(1);
    let command = args.next().unwrap_or_else(|| "check".into());
    if command == "--version" {
        println!("theosis {}", env!("CARGO_PKG_VERSION"));
        return Ok(0);
    }
    if command == "--help" || command == "help" {
        println!("theosis [check|update] [--remote URL] [--ref REF] [--binary PATH] [--install-dir DIR] [--json]");
        return Ok(0);
    }
    let mut remote = None;
    let mut reference = "main".to_string();
    let mut binary = None;
    let mut install_dir = None;
    let mut json = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--remote" => remote = args.next(),
            "--ref" => {
                reference = args
                    .next()
                    .ok_or_else(|| Error::Config("--ref requires a value".into()))?
            }
            "--binary" => binary = args.next().map(PathBuf::from),
            "--install-dir" => install_dir = args.next().map(PathBuf::from),
            "--json" => json = true,
            other => return Err(Error::Config(format!("unknown argument {other}"))),
        }
    }
    if command != "check" && command != "update" {
        return Err(Error::Config(format!("unknown command {command}")));
    }
    let remote = remote
        .or_else(|| run("git", &["config", "--get", "remote.origin.url"], None).ok())
        .ok_or_else(|| Error::Config("remote is required (--remote or git origin)".into()))?;
    let binary = binary.unwrap_or_else(|| PathBuf::from("theosis"));
    let local = installed_version(&binary)?;
    let workspace = temp_workspace()?;
    let source = workspace.join("source");
    run(
        "git",
        &[
            "clone",
            "--depth",
            "1",
            "--branch",
            &reference,
            &remote,
            source
                .to_str()
                .ok_or_else(|| Error::Config("invalid source path".into()))?,
        ],
        None,
    )?;
    let commit = run("git", &["rev-parse", "HEAD"], Some(&source))?;
    let target = cargo_version(&source)?;
    let decision = if target > local {
        "UPDATE"
    } else if target == local {
        "NOOP"
    } else {
        "LOCAL_NEWER"
    };
    if json {
        println!("{{\"installed\":\"{local}\",\"remote\":\"{target}\",\"commit\":\"{}\",\"decision\":\"{decision}\"}}", json_escape(&commit));
    } else {
        println!(
            "Installed: {local}\nRemote:    {target}\nCommit:    {commit}\nDecision:  {decision}"
        );
    }
    if command == "check" || target <= local {
        return Ok(if target < local { 10 } else { 0 });
    }
    let recipe = source.join(".baby.toml");
    let mut baby_args = vec!["--user"];
    if recipe.exists() {
        baby_args.extend([
            "--recipe",
            recipe
                .to_str()
                .ok_or_else(|| Error::Config("invalid recipe path".into()))?,
        ]);
    }
    run("baby", &baby_args, Some(&source))
        .map_err(|e| Error::Command(format!("Baby build/install failed: {e}")))?;
    let installed_path = install_dir
        .map(|d| d.join(binary.file_name().unwrap_or_default()))
        .unwrap_or(binary);
    let actual = installed_version(&installed_path)?;
    if actual != target {
        return Err(Error::Version(format!(
            "verification mismatch: expected {target}, got {actual}"
        )));
    }
    fs::write(workspace.join("transaction.json"), format!("{{\"previous_version\":\"{local}\",\"target_version\":\"{target}\",\"commit\":\"{}\",\"result\":\"success\"}}\n", json_escape(&commit)))?;
    println!("Verified: {actual}");
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_and_compares_numerically() {
        assert!(Version::parse("1.10.0").unwrap() > Version::parse("1.9.9").unwrap());
    }
    #[test]
    fn rejects_non_abc() {
        assert!(Version::parse("1.2").is_err());
        assert!(Version::parse("1.2.3.4").is_err());
    }
}
