use crate::assistant_preferences::{valid_model_id, valid_opencode_variant};
use std::{
    env,
    fmt::Write,
    io,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Debug)]
pub struct CodexCli {
    program: PathBuf,
    npm_script: Option<PathBuf>,
}

impl CodexCli {
    fn command(&self) -> Command {
        let mut command = hidden_command(&self.program);
        if let Some(script) = &self.npm_script {
            command.arg(script);
        }
        command
    }

    pub fn version_command(&self) -> Command {
        let mut command = self.command();
        command.arg("--version");
        command
    }

    pub fn app_server_command(&self, cwd: &Path) -> Command {
        let mut command = self.command();
        command.current_dir(cwd).args([
            "app-server",
            "--config",
            "allow_login_shell=false",
            "--config",
            "thread_unload_delay_secs=0",
        ]);
        command
    }
}

#[derive(Clone, Debug)]
pub struct OpenCodeCli {
    program: PathBuf,
}

impl OpenCodeCli {
    pub fn version_command(&self) -> Command {
        let mut command = hidden_command(&self.program);
        command.arg("--version");
        command
    }

    pub fn run_help_command(&self) -> Command {
        let mut command = hidden_command(&self.program);
        command.args(["run", "--help"]);
        command
    }

    pub fn run_command(
        &self,
        cwd: &Path,
        session_id: Option<&str>,
        images: &[PathBuf],
        model: Option<&str>,
        variant: Option<&str>,
        standalone: bool,
    ) -> io::Result<Command> {
        if model.is_some_and(|id| !valid_model_id(id) || id.contains('#') || id.starts_with('-')) {
            return Err(invalid("Invalid OpenCode model"));
        }
        if variant.is_some_and(|id| !valid_opencode_variant(id)) {
            return Err(invalid("Invalid OpenCode variant"));
        }
        let mut command = hidden_command(&self.program);
        command.current_dir(cwd).args(["run", "--format", "json"]);
        if standalone {
            command.arg("--standalone");
        }
        if let Some(model) = model {
            command.arg("--model");
            match variant {
                Some(variant) => command.arg(format!("{model}#{variant}")),
                None => command.arg(model),
            };
        }
        if let Some(session_id) = session_id.filter(|id| !id.is_empty()) {
            check_session_id(session_id)?;
            command.args(["--session", session_id]);
        }
        for file in images {
            let file = if file.is_absolute() {
                file.clone()
            } else {
                cwd.join(file)
            };
            command.arg("--file").arg(file);
        }
        Ok(command)
    }

    pub fn prepare_session_command(&self, cwd: &Path, session_id: &str) -> io::Result<Command> {
        check_session_id(session_id)?;
        let mut endpoint = String::from("/api/session/");
        for byte in session_id.bytes() {
            if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
                endpoint.push(char::from(byte));
            } else {
                write!(&mut endpoint, "%{byte:02X}").expect("writing to String cannot fail");
            }
        }
        endpoint.push_str("/interrupt");
        let mut command = hidden_command(&self.program);
        command
            .current_dir(cwd)
            .args(["api", "POST", &endpoint, "--standalone"]);
        Ok(command)
    }

    pub fn catalog_server_command(
        &self,
        cwd: Option<&Path>,
        password: &str,
    ) -> io::Result<Command> {
        if password.is_empty() {
            return Err(invalid("OpenCode catalog requires a server password"));
        }
        let mut command = hidden_command(&self.program);
        command
            .args(["serve", "--hostname", "127.0.0.1", "--port", "0"])
            .env("OPENCODE_SERVER_USERNAME", "opencode")
            .env("OPENCODE_SERVER_PASSWORD", password);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        Ok(command)
    }
}

pub fn discover_codex_cli() -> io::Result<CodexCli> {
    CliPaths::current().codex()
}

pub fn discover_opencode_cli() -> io::Result<OpenCodeCli> {
    CliPaths::current().opencode()
}

#[derive(Clone, Copy)]
enum Platform {
    Windows,
    Unix,
}

struct CliPaths {
    platform: Platform,
    appdata: Option<PathBuf>,
    home: Option<PathBuf>,
    path: Vec<PathBuf>,
}

impl CliPaths {
    fn current() -> Self {
        let (platform, home_variable) = if cfg!(windows) {
            (Platform::Windows, "USERPROFILE")
        } else {
            (Platform::Unix, "HOME")
        };
        let home = absolute_env_path(home_variable);
        let appdata = absolute_env_path("APPDATA");
        let mut path: Vec<_> = env::var_os("PATH")
            .as_deref()
            .map(env::split_paths)
            .into_iter()
            .flatten()
            .filter(|directory| directory.is_absolute())
            .collect();
        if matches!(platform, Platform::Unix) {
            if let Some(home) = &home {
                path.push(home.join(".local/bin"));
            }
            if cfg!(target_os = "macos") {
                path.push(PathBuf::from("/opt/homebrew/bin"));
            }
            path.push(PathBuf::from("/usr/local/bin"));
            path.push(PathBuf::from("/usr/bin"));
        }
        Self {
            platform,
            appdata,
            home,
            path,
        }
    }

    fn codex(&self) -> io::Result<CodexCli> {
        let program = match self.platform {
            Platform::Windows => {
                let npm = self.appdata.as_ref().map(|directory| directory.join("npm"));
                let npm_script = npm
                    .iter()
                    .chain(&self.path)
                    .filter(|directory| directory.is_absolute())
                    .map(|directory| directory.join("node_modules/@openai/codex/bin/codex.js"))
                    .find(|script| script.is_file());
                if let Some(script) = npm_script
                    && let Some(node) = self.find_in_path("node.exe")
                {
                    return Ok(CodexCli {
                        program: node,
                        npm_script: Some(script),
                    });
                }
                self.find_in_path("codex.exe")
            }
            Platform::Unix => self.find_in_path("codex"),
        };
        program
            .map(|program| CodexCli {
                program,
                npm_script: None,
            })
            .ok_or_else(|| missing("Codex"))
    }

    fn opencode(&self) -> io::Result<OpenCodeCli> {
        let program = match self.platform {
            Platform::Windows => self
                .appdata
                .iter()
                .filter(|directory| directory.is_absolute())
                .flat_map(|directory| {
                    [
                        directory.join("npm/node_modules/@opencode/cli/bin/opencode.exe"),
                        directory.join("npm/node_modules/opencode-ai/bin/opencode.exe"),
                    ]
                })
                .chain(
                    self.home
                        .iter()
                        .filter(|directory| directory.is_absolute())
                        .map(|home| home.join(".opencode/bin/opencode.exe")),
                )
                .find(|file| self.executable(file))
                .or_else(|| self.find_in_path("opencode.exe")),
            Platform::Unix => self.find_in_path("opencode").or_else(|| {
                self.home
                    .as_ref()
                    .filter(|directory| directory.is_absolute())
                    .map(|home| home.join(".opencode/bin/opencode"))
                    .filter(|file| self.executable(file))
            }),
        };
        program
            .map(|program| OpenCodeCli { program })
            .ok_or_else(|| missing("OpenCode"))
    }

    fn find_in_path(&self, basename: &str) -> Option<PathBuf> {
        self.path
            .iter()
            .filter(|directory| directory.is_absolute())
            .map(|directory| directory.join(basename))
            .find(|file| self.executable(file))
    }

    fn executable(&self, file: &Path) -> bool {
        file.is_file() && (matches!(self.platform, Platform::Windows) || unix_executable(file))
    }
}

fn absolute_env_path(name: &str) -> Option<PathBuf> {
    env::var_os(name)
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn unix_executable(path: &Path) -> bool {
    rustix::fs::access(path, rustix::fs::Access::EXEC_OK).is_ok()
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn unix_executable(_: &Path) -> bool {
    false
}

fn hidden_command(program: &Path) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
        let mut command = Command::new(program);
        command.creation_flags(CREATE_NO_WINDOW);
        command
    }
    #[cfg(not(windows))]
    Command::new(program)
}

fn check_session_id(session_id: &str) -> io::Result<()> {
    if !valid_model_id(session_id)
        || session_id.starts_with('-')
        || matches!(session_id, "." | "..")
    {
        return Err(invalid("Invalid OpenCode session ID"));
    }
    Ok(())
}

fn missing(provider: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{provider} was not found. Install and authenticate {provider} before using it."),
    )
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{ffi::OsStr, fs};

    fn fixture_file(root: &Path, name: &str) -> PathBuf {
        let path = root.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, []).unwrap();
        path
    }

    #[test]
    fn uses_windows_npm_codex_with_node_and_falls_back_to_a_native_binary() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let script = fixture_file(root, "roaming/npm/node_modules/@openai/codex/bin/codex.js");
        let node = fixture_file(root, "Node tools/node.exe");
        let native = fixture_file(root, "cli/codex.exe");
        let layout = CliPaths {
            platform: Platform::Windows,
            appdata: Some(root.join("roaming")),
            home: None,
            path: vec![root.join("Node tools"), root.join("cli")],
        };
        let codex = layout.codex().unwrap();
        let command = codex.app_server_command(root);
        assert_eq!(Path::new(command.get_program()), node);
        assert_eq!(command.get_current_dir(), Some(root));
        let mut arguments = command.get_args();
        assert_eq!(Path::new(arguments.next().unwrap()), script);
        assert_eq!(
            arguments.collect::<Vec<_>>(),
            [
                OsStr::new("app-server"),
                OsStr::new("--config"),
                OsStr::new("allow_login_shell=false"),
                OsStr::new("--config"),
                OsStr::new("thread_unload_delay_secs=0"),
            ]
        );
        fs::remove_file(node).unwrap();
        let codex = layout.codex().unwrap();
        assert_eq!(Path::new(codex.version_command().get_program()), native);
        fs::remove_file(native).unwrap();
        fixture_file(root, "cli/codex.cmd");
        assert_eq!(layout.codex().unwrap_err().kind(), io::ErrorKind::NotFound);
    }

    #[test]
    fn finds_existing_windows_opencode_installations_in_reference_order() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let first = fixture_file(
            root,
            "roaming/npm/node_modules/@opencode/cli/bin/opencode.exe",
        );
        let second = fixture_file(
            root,
            "roaming/npm/node_modules/opencode-ai/bin/opencode.exe",
        );
        let third = fixture_file(root, "profile/.opencode/bin/opencode.exe");
        let fourth = fixture_file(root, "cli/opencode.exe");
        let layout = CliPaths {
            platform: Platform::Windows,
            appdata: Some(root.join("roaming")),
            home: Some(root.join("profile")),
            path: vec![root.join("cli")],
        };
        for expected in [first, second, third, fourth] {
            assert_eq!(
                Path::new(layout.opencode().unwrap().version_command().get_program()),
                expected,
            );
            fs::remove_file(expected).unwrap();
        }
        assert_eq!(
            layout.opencode().unwrap_err().kind(),
            io::ErrorKind::NotFound
        );
    }

    #[cfg(any(target_os = "linux", target_os = "macos"))]
    #[test]
    fn only_selects_executable_unix_programs_from_absolute_paths() {
        use std::os::unix::fs::PermissionsExt;

        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let non_executable = fixture_file(root, "first/codex");
        let codex = fixture_file(root, "second/codex");
        let opencode = fixture_file(root, "profile/.opencode/bin/opencode");
        fs::set_permissions(non_executable, fs::Permissions::from_mode(0o600)).unwrap();
        for file in [&codex, &opencode] {
            fs::set_permissions(file, fs::Permissions::from_mode(0o700)).unwrap();
        }
        let layout = CliPaths {
            platform: Platform::Unix,
            appdata: None,
            home: Some(root.join("profile")),
            path: vec![root.join("first"), PathBuf::from("."), root.join("second")],
        };
        assert_eq!(
            layout.codex().unwrap().version_command().get_program(),
            codex.as_os_str()
        );
        assert_eq!(
            layout.opencode().unwrap().version_command().get_program(),
            opencode.as_os_str()
        );
    }

    #[test]
    fn constructs_opencode_arguments_without_interpolating_shell_text() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let opencode = OpenCodeCli {
            program: root.join("some cli"),
        };
        let image = root.join("paper & figures.png");
        let command = opencode
            .run_command(
                root,
                Some("ses_1"),
                std::slice::from_ref(&image),
                Some("vendor/family/model"),
                Some("deep"),
                true,
            )
            .unwrap();
        let mut expected: Vec<_> = [
            "run",
            "--format",
            "json",
            "--standalone",
            "--model",
            "vendor/family/model#deep",
            "--session",
            "ses_1",
            "--file",
        ]
        .map(OsStr::new)
        .to_vec();
        expected.push(image.as_os_str());
        assert_eq!(command.get_args().collect::<Vec<_>>(), expected);
        assert_eq!(command.get_current_dir(), Some(root));
        assert!(
            opencode
                .run_command(root, Some("--help"), &[], None, None, false)
                .is_err()
        );
        assert!(
            opencode
                .run_command(root, None, &[], Some("provider/model#variant"), None, false)
                .is_err()
        );
        let preparation = opencode.prepare_session_command(root, "ses_a/b#c").unwrap();
        assert_eq!(
            preparation.get_args().collect::<Vec<_>>(),
            [
                "api",
                "POST",
                "/api/session/ses_a%2Fb%23c/interrupt",
                "--standalone"
            ]
            .map(OsStr::new)
        );
    }
}
