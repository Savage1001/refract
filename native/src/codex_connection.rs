use crate::{
    METADATA_LIMIT, project_management::is_link, read_local_file, tex::process::configure_process,
};
use sha2::{Digest, Sha256};
use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum CodexMode {
    #[default]
    Account,
    Cli,
}

impl CodexMode {
    pub fn load(profile: &Path) -> Self {
        let root = profile.join("codex-account");
        if let Ok(contents) = read_local_file(&root, &root.join("mode"), METADATA_LIMIT)
            && contents == b"cli"
        {
            return Self::Cli;
        }
        Self::Account
    }

    pub fn save(self, profile: &Path) -> io::Result<()> {
        fs::create_dir_all(profile)?;
        let root = profile.join("codex-account");
        ensure_account_directory(&root)?;
        let mut stage = tempfile::NamedTempFile::new_in(&root)?;
        stage.write_all(match self {
            Self::Account => b"account",
            Self::Cli => b"cli",
        })?;
        stage.as_file().sync_all()?;
        stage
            .persist(root.join("mode"))
            .map_err(|error| error.error)?;
        Ok(())
    }
}

pub struct AccountWorkspace {
    account_key: String,
    home: PathBuf,
}

impl AccountWorkspace {
    pub fn prepare(profile: &Path, client_id: &str) -> io::Result<Self> {
        if client_id.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Missing ChatGPT account registration",
            ));
        }
        fs::create_dir_all(profile)?;
        let account = profile.join("codex-account");
        let workspaces = account.join("workspaces");
        let account_key = format!("{:x}", Sha256::digest(client_id.as_bytes()));
        let home = workspaces.join(&account_key);
        for destination in [&account, &workspaces, &home] {
            ensure_account_directory(destination)?;
        }
        Ok(Self { account_key, home })
    }

    pub fn key(&self) -> &str {
        &self.account_key
    }

    pub fn app_server_command(
        &self,
        executable: &Path,
        cwd: &Path,
        access_token: &str,
    ) -> io::Result<Command> {
        if access_token.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "Sign in with ChatGPT in Refract Settings.",
            ));
        }
        let runtime = executable.parent().and_then(Path::parent).ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "Invalid Codex executable")
        })?;
        let inherited = env::var_os("PATH");
        let path = env::join_paths(
            std::iter::once(runtime.join("codex-path"))
                .chain(inherited.as_deref().into_iter().flat_map(env::split_paths)),
        )
        .map_err(io::Error::other)?;
        #[cfg(windows)]
        let cwd = dunce::simplified(cwd);
        let cwd_text = cwd.to_str().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid Codex project directory",
            )
        })?;
        let project = serde_json::to_string(cwd_text)?;
        let mut command = Command::new(executable);
        configure_process(&mut command);
        command
            .current_dir(cwd)
            .args(["--listen", "stdio://"])
            .env("PATH", path)
            .env("CODEX_HOME", &self.home)
            .env("REFRACT_CHATGPT_ACCESS_TOKEN", access_token);
        for key in [
            "OPENAI_API_KEY",
            "OPENAI_BASE_URL",
            "CODEX_ACCESS_TOKEN",
            "CODEX_API_KEY",
        ] {
            command.env_remove(key);
        }
        for option in [
            "allow_login_shell=false",
            "thread_unload_delay_secs=0",
            "model_provider=\"openai_chatgpt_plan\"",
            "model_providers.openai_chatgpt_plan.name=\"ChatGPT plan\"",
            "model_providers.openai_chatgpt_plan.base_url=\"https://api.openai.com/v1\"",
            "model_providers.openai_chatgpt_plan.env_key=\"REFRACT_CHATGPT_ACCESS_TOKEN\"",
            "model_providers.openai_chatgpt_plan.wire_api=\"responses\"",
            "model_providers.openai_chatgpt_plan.requires_openai_auth=false",
            "model_providers.openai_chatgpt_plan.supports_websockets=false",
            "shell_environment_policy.exclude=[\"REFRACT_CHATGPT_ACCESS_TOKEN\"]",
            "features.tool_search=false",
        ] {
            command.args(["-c", option]);
        }
        command.args([
            "-c",
            &format!("projects.{project}.trust_level=\"untrusted\""),
        ]);
        Ok(command)
    }
}

fn ensure_account_directory(directory: &Path) -> io::Result<()> {
    match fs::create_dir(directory) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    let info = directory.symlink_metadata()?;
    if !info.is_dir() || is_link(&info) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Unsafe Refract account directory",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{collections::HashMap, ffi::OsStr};

    #[test]
    fn preserves_the_existing_codex_mode_file() {
        let profile = tempfile::tempdir().unwrap();
        assert_eq!(CodexMode::load(profile.path()), CodexMode::Account);
        CodexMode::Cli.save(profile.path()).unwrap();
        let file = profile.path().join("codex-account/mode");
        assert_eq!(fs::read(&file).unwrap(), b"cli");
        assert_eq!(CodexMode::load(profile.path()), CodexMode::Cli);
        CodexMode::Account.save(profile.path()).unwrap();
        assert_eq!(fs::read(&file).unwrap(), b"account");
        assert_eq!(CodexMode::load(profile.path()), CodexMode::Account);
    }

    #[test]
    fn reuses_the_existing_account_workspace_and_limits_token_scope() {
        let profile = tempfile::tempdir().unwrap();
        let id = "synthetic-client";
        let workspace = AccountWorkspace::prepare(profile.path(), id).unwrap();
        let key = format!("{:x}", Sha256::digest(id.as_bytes()));
        assert_eq!(workspace.key(), key);
        let home = profile
            .path()
            .join("codex-account")
            .join("workspaces")
            .join(key);
        assert!(home.is_dir());
        let runtime = profile.path().join("runtime");
        let executable = runtime.join("bin/codex-app-server.exe");
        let cwd = profile.path().join("my project");
        let command = workspace
            .app_server_command(&executable, &cwd, "synthetic-authorization")
            .unwrap();
        let environment: HashMap<_, _> = command.get_envs().collect();
        assert_eq!(command.get_program(), executable);
        assert_eq!(command.get_current_dir(), Some(cwd.as_path()));
        assert_eq!(
            environment[OsStr::new("CODEX_HOME")],
            Some(home.as_os_str())
        );
        assert_eq!(environment[OsStr::new("OPENAI_API_KEY")], None);
        assert_eq!(environment[OsStr::new("CODEX_ACCESS_TOKEN")], None);
        assert_eq!(
            env::split_paths(environment[OsStr::new("PATH")].unwrap()).next(),
            Some(runtime.join("codex-path"))
        );
        let arguments: Vec<_> = command
            .get_args()
            .map(|value| value.to_str().unwrap())
            .collect();
        assert!(
            arguments
                .contains(&"shell_environment_policy.exclude=[\"REFRACT_CHATGPT_ACCESS_TOKEN\"]")
        );
        assert!(
            arguments
                .iter()
                .any(|option| option.ends_with(".trust_level=\"untrusted\""))
        );
        assert!(
            !arguments
                .iter()
                .any(|option| option.contains("synthetic-authorization"))
        );
    }
}
