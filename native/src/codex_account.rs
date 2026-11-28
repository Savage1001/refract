use crate::{
    codex_connection::{AccountWorkspace, CodexMode},
    managed_codex::{self, RuntimeProgress},
};
use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        Arc, Mutex, MutexGuard, TryLockError,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    time::{SystemTime, UNIX_EPOCH},
};
use zeroize::Zeroize;

pub mod oauth;
mod storage;

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Registration {
    pub(crate) client_id: String,
    pub(crate) subject: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) email: Option<String>,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Session {
    #[serde(flatten)]
    pub(crate) registration: Registration,
    pub(crate) access_token: String,
    pub(crate) refresh_token: String,
    pub(crate) expires_at: u64,
}

impl Drop for Session {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
    }
}

#[derive(Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountData {
    pub(crate) host_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) registration: Option<Registration>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) session: Option<Session>,
}

impl AccountData {
    pub fn load(profile: &Path) -> io::Result<Option<Self>> {
        storage::load(profile)
    }

    pub fn save(&self, profile: &Path) -> io::Result<()> {
        storage::save(profile, self)
    }
}

pub struct CodexAccount {
    profile: PathBuf,
    directory: PathBuf,
    operation: Mutex<()>,
    active: AtomicUsize,
}

pub struct AccountStatus {
    pub mode: CodexMode,
    pub installed: bool,
    pub signed_in: bool,
    pub email: Option<String>,
    pub has_registration: bool,
    pub remembered: Option<String>,
}

pub struct CodexConnection {
    account: Arc<CodexAccount>,
    workspace: AccountWorkspace,
    executable: PathBuf,
    session: Session,
}

impl Drop for CodexConnection {
    fn drop(&mut self) {
        self.account.active.fetch_sub(1, Ordering::AcqRel);
    }
}

impl CodexAccount {
    pub fn new(profile: PathBuf) -> Self {
        Self {
            directory: profile.join("codex-account"),
            profile,
            operation: Mutex::new(()),
            active: AtomicUsize::new(0),
        }
    }

    pub fn mode(&self) -> CodexMode {
        CodexMode::load(&self.profile)
    }

    pub fn status(&self) -> io::Result<AccountStatus> {
        let data = AccountData::load(&self.profile)?;
        let session = data.as_ref().and_then(|data| data.session.as_ref());
        let registration = data.as_ref().and_then(|data| data.registration.as_ref());
        Ok(AccountStatus {
            mode: self.mode(),
            installed: managed_codex::executable(&self.directory).is_some(),
            signed_in: session.is_some(),
            email: session.and_then(|session| session.registration.email.clone()),
            has_registration: registration.is_some(),
            remembered: registration.and_then(|registration| registration.email.clone()),
        })
    }

    pub fn select(
        &self,
        mode: CodexMode,
        cancel: &AtomicBool,
        progress: impl FnMut(RuntimeProgress),
    ) -> io::Result<()> {
        let _operation = self.setup_operation()?;
        mode.save(&self.profile)?;
        if mode == CodexMode::Account {
            managed_codex::install(&self.directory, cancel, progress)?;
        }
        Ok(())
    }

    pub fn install(
        &self,
        cancel: &AtomicBool,
        progress: impl FnMut(RuntimeProgress),
    ) -> io::Result<()> {
        let _operation = self.setup_operation()?;
        managed_codex::install(&self.directory, cancel, progress)?;
        Ok(())
    }

    pub fn sign_in(
        &self,
        new_registration: bool,
        open_browser: impl FnOnce(&str) -> io::Result<()>,
        cancel: &AtomicBool,
    ) -> io::Result<()> {
        let _operation = self.setup_operation()?;
        if self.mode() != CodexMode::Account || managed_codex::executable(&self.directory).is_none()
        {
            return Err(io::Error::other("Install the Codex runtime first."));
        }
        let mut data = match AccountData::load(&self.profile)? {
            Some(data) => data,
            None => {
                let data = AccountData {
                    host_id: format!("urn:uuid:{}", uuid::Uuid::new_v4()),
                    registration: None,
                    session: None,
                };
                data.save(&self.profile)?;
                data
            }
        };
        if data.session.is_some() {
            return Err(io::Error::other(
                "Sign out before connecting another account.",
            ));
        }
        let registration = data.registration.as_ref().filter(|_| !new_registration);
        let session = oauth::authorize(&data.host_id, registration, open_browser, cancel)
            .map_err(auth_error)?;
        if cancel.load(Ordering::Relaxed) {
            return Err(auth_error(oauth::Error::Cancelled));
        }
        data.registration = Some(session.registration.clone());
        data.session = Some(session);
        data.save(&self.profile)
    }

    pub fn sign_out(&self, cancel: &AtomicBool) -> io::Result<bool> {
        let _operation = self.setup_operation()?;
        if self.active.load(Ordering::Acquire) != 0 {
            return Err(io::Error::other(
                "Wait for the Codex response before signing out.",
            ));
        }
        let Some(mut data) = AccountData::load(&self.profile)? else {
            return Ok(true);
        };
        let Some(session) = data.session.take() else {
            return Ok(true);
        };
        let revoked = oauth::revoke(&session, cancel);
        data.save(&self.profile)?;
        Ok(revoked)
    }

    pub fn connection(self: &Arc<Self>, cancel: &AtomicBool) -> io::Result<CodexConnection> {
        let _operation = self.setup_operation()?;
        let executable = managed_codex::executable(&self.directory).ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "Download the Codex runtime in Refract Settings.",
            )
        })?;
        let mut data = AccountData::load(&self.profile)?
            .filter(|data| data.session.is_some())
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "Sign in with ChatGPT in Refract Settings.",
                )
            })?;
        let previous = data.session.as_ref().expect("checked session");
        let current = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_millis() as u64);
        if previous.expires_at.saturating_sub(current) <= 120_000 {
            match oauth::renew(previous, cancel) {
                Ok(session) => {
                    data.session = Some(session);
                    data.save(&self.profile)?;
                }
                Err(error @ oauth::Error::SignInRequired(_)) => {
                    data.session = None;
                    data.save(&self.profile)?;
                    return Err(auth_error(error));
                }
                Err(error) => return Err(auth_error(error)),
            }
        }
        let session = data.session.take().expect("checked account connection");
        let workspace = AccountWorkspace::prepare(&self.profile, &session.registration.client_id)?;
        self.active.fetch_add(1, Ordering::AcqRel);
        Ok(CodexConnection {
            account: self.clone(),
            workspace,
            executable,
            session,
        })
    }

    fn setup_operation(&self) -> io::Result<MutexGuard<'_, ()>> {
        match self.operation.try_lock() {
            Ok(guard) => Ok(guard),
            Err(TryLockError::WouldBlock) => Err(io::Error::new(
                io::ErrorKind::WouldBlock,
                "Codex setup is already in progress.",
            )),
            Err(TryLockError::Poisoned(_)) => {
                Err(io::Error::other("Restart Refract to finish Codex setup."))
            }
        }
    }
}

impl CodexConnection {
    pub fn account_key(&self) -> &str {
        self.workspace.key()
    }

    pub fn app_server_command(&self, workspace: &Path) -> io::Result<Command> {
        self.workspace
            .app_server_command(&self.executable, workspace, &self.session.access_token)
    }
}

fn auth_error(error: oauth::Error) -> io::Error {
    let kind = match error {
        oauth::Error::Cancelled => io::ErrorKind::Interrupted,
        oauth::Error::SignInRequired(_) => io::ErrorKind::PermissionDenied,
        oauth::Error::Failed(_) => io::ErrorKind::Other,
    };
    io::Error::new(kind, error)
}
