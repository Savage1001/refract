use super::AccountData;
use crate::project_management::{has_one_link, is_link};
use base64::{Engine, engine::general_purpose::STANDARD};
use ring::aead::{AES_256_GCM, Aad, LessSafeKey, NONCE_LEN, Nonce, UnboundKey};
use serde_json::{Map, Value};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::Path,
    ptr,
};
use windows_sys::Win32::{
    Foundation::LocalFree,
    Security::Cryptography::{
        CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
    },
};
use zeroize::{Zeroize, Zeroizing};

const STATE_FILE: &str = "Local State";
const ACCOUNT_FILE: &str = "connection.bin";
const MAX_STATE: u64 = 8 * 1024 * 1024;
const MAX_ACCOUNT: u64 = 1024 * 1024;
const KEY_HEADER: &[u8] = b"DPAPI";
const DATA_HEADER: &[u8] = b"v10";
const KEY_BYTES: usize = 32;

pub(super) fn load(profile: &Path) -> io::Result<Option<AccountData>> {
    let account = profile.join("codex-account");
    if !check_directory(&account)? {
        return Ok(None);
    }
    let Some(encrypted) = read_optional_file(&account.join(ACCOUNT_FILE), MAX_ACCOUNT)? else {
        return Ok(None);
    };
    let state =
        read_optional_file(&profile.join(STATE_FILE), MAX_STATE)?.ok_or_else(unavailable)?;
    let state: Value = serde_json::from_slice(&state).map_err(|_| unavailable())?;
    let key = key_from_state(&state)?.ok_or_else(unavailable)?;
    decrypt_account(encrypted, &key).map(Some)
}

pub(super) fn save(profile: &Path, data: &AccountData) -> io::Result<()> {
    fs::create_dir_all(profile)?;
    let account = profile.join("codex-account");
    match fs::create_dir(&account) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(error),
    }
    if !check_directory(&account)? {
        return Err(unavailable());
    }
    let lock_path = account.join("connection.lock");
    if lock_path
        .symlink_metadata()
        .is_ok_and(|info| !info.is_file() || is_link(&info))
    {
        return Err(unavailable());
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)?;
    if !has_one_link(&lock)? {
        return Err(unavailable());
    }
    lock.lock()?;
    let connection = account.join(ACCOUNT_FILE);
    let current = read_optional_file(&connection, MAX_ACCOUNT)?;
    let key = get_or_create_key(profile, current.is_some())?;
    if let Some(current) = current {
        // Preserve an existing connection that cannot be authenticated and decoded.
        let _ = decrypt_account(current, &key)?;
    }
    let encrypted = encrypt_account(data, &key)?;
    replace_file(&connection, &encrypted)
}

fn get_or_create_key(profile: &Path, existing_account: bool) -> io::Result<LessSafeKey> {
    let file = profile.join(STATE_FILE);
    let previous = read_optional_file(&file, MAX_STATE)?;
    let mut state = match &previous {
        Some(bytes) => serde_json::from_slice::<Value>(bytes).map_err(|_| unavailable())?,
        None => Value::Object(Map::new()),
    };
    if let Some(key) = key_from_state(&state)? {
        return Ok(key);
    }
    if existing_account {
        return Err(unavailable());
    }
    let mut raw = Zeroizing::new([0_u8; KEY_BYTES]);
    getrandom::fill(raw.as_mut()).map_err(|_| unavailable())?;
    let protected = dpapi(&raw[..], true)?;
    let mut bytes = KEY_HEADER.to_vec();
    bytes.extend_from_slice(&protected);
    let state_map = state.as_object_mut().ok_or_else(unavailable)?;
    let crypt = state_map
        .entry("os_crypt")
        .or_insert_with(|| Value::Object(Map::new()))
        .as_object_mut()
        .ok_or_else(unavailable)?;
    crypt.insert(
        "encrypted_key".into(),
        Value::String(STANDARD.encode(bytes)),
    );
    let updated = serde_json::to_vec(&state).map_err(|_| unavailable())?;
    if updated.len() as u64 > MAX_STATE {
        return Err(unavailable());
    }
    let staged = staged_file(&file, &updated)?;
    if read_optional_file(&file, MAX_STATE)? != previous {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Refract’s profile changed while setting up secure storage.",
        ));
    }
    if previous.is_some() {
        staged.persist(&file).map_err(|error| error.error)?;
    } else {
        staged
            .persist_noclobber(&file)
            .map_err(|error| error.error)?;
    }
    make_key(&raw[..])
}

fn key_from_state(state: &Value) -> io::Result<Option<LessSafeKey>> {
    let state = state.as_object().ok_or_else(unavailable)?;
    let Some(crypt) = state.get("os_crypt") else {
        return Ok(None);
    };
    let crypt = crypt.as_object().ok_or_else(unavailable)?;
    let Some(encoded) = crypt.get("encrypted_key") else {
        return Ok(None);
    };
    let encoded = encoded.as_str().ok_or_else(unavailable)?;
    let protected = STANDARD.decode(encoded).map_err(|_| unavailable())?;
    let protected = protected.strip_prefix(KEY_HEADER).ok_or_else(unavailable)?;
    let bytes = dpapi(protected, false)?;
    make_key(&bytes).map(Some)
}

fn make_key(bytes: &[u8]) -> io::Result<LessSafeKey> {
    UnboundKey::new(&AES_256_GCM, bytes)
        .map(LessSafeKey::new)
        .map_err(|_| unavailable())
}

fn encrypt_account(data: &AccountData, key: &LessSafeKey) -> io::Result<Vec<u8>> {
    let mut nonce = [0_u8; NONCE_LEN];
    getrandom::fill(&mut nonce).map_err(|_| unavailable())?;
    let mut bytes = Zeroizing::new(serde_json::to_vec(data).map_err(|_| unavailable())?);
    key.seal_in_place_append_tag(
        Nonce::assume_unique_for_key(nonce),
        Aad::empty(),
        &mut *bytes,
    )
    .map_err(|_| unavailable())?;
    let mut result = DATA_HEADER.to_vec();
    result.extend_from_slice(&nonce);
    result.extend_from_slice(&bytes);
    if result.len() as u64 > MAX_ACCOUNT {
        return Err(unavailable());
    }
    Ok(result)
}

fn decrypt_account(mut bytes: Vec<u8>, key: &LessSafeKey) -> io::Result<AccountData> {
    if !bytes.starts_with(DATA_HEADER)
        || bytes.len() < DATA_HEADER.len() + NONCE_LEN + AES_256_GCM.tag_len()
    {
        return Err(unavailable());
    }
    let split = DATA_HEADER.len() + NONCE_LEN;
    let nonce = Nonce::try_assume_unique_for_key(&bytes[DATA_HEADER.len()..split])
        .map_err(|_| unavailable())?;
    let mut sealed = Zeroizing::new(bytes.split_off(split));
    let decoded = key
        .open_in_place(nonce, Aad::empty(), &mut sealed)
        .map_err(|_| unavailable())?;
    serde_json::from_slice(decoded).map_err(|_| unavailable())
}

fn dpapi(bytes: &[u8], encrypt: bool) -> io::Result<Zeroizing<Vec<u8>>> {
    let mut input = Zeroizing::new(bytes.to_vec());
    let input = CRYPT_INTEGER_BLOB {
        cbData: input.len().try_into().map_err(|_| unavailable())?,
        pbData: input.as_mut_ptr(),
    };
    let mut output = CRYPT_INTEGER_BLOB::default();
    // DPAPI accepts only this buffer and is explicitly forbidden from showing UI.
    let success = unsafe {
        if encrypt {
            CryptProtectData(
                &input,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptUnprotectData(
                &input,
                ptr::null_mut(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if success == 0 || output.pbData.is_null() {
        return Err(unavailable());
    }
    // DPAPI owns this allocation until LocalFree. Erase it before releasing it.
    let decoded = unsafe {
        let buffer = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
        let decoded = Zeroizing::new(buffer.to_vec());
        buffer.zeroize();
        LocalFree(output.pbData.cast());
        decoded
    };
    Ok(decoded)
}

fn check_directory(path: &Path) -> io::Result<bool> {
    match path.symlink_metadata() {
        Ok(info) if info.is_dir() && !is_link(&info) => Ok(true),
        Ok(_) => Err(unavailable()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(error),
    }
}

fn read_optional_file(path: &Path, limit: u64) -> io::Result<Option<Vec<u8>>> {
    match path.symlink_metadata() {
        Ok(info) if info.is_file() && !is_link(&info) && info.len() <= limit => {}
        Ok(_) => return Err(unavailable()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error),
    }
    let file = File::open(path)?;
    if !has_one_link(&file)? {
        return Err(unavailable());
    }
    let mut bytes = Vec::new();
    file.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(unavailable());
    }
    Ok(Some(bytes))
}

fn staged_file(path: &Path, bytes: &[u8]) -> io::Result<tempfile::NamedTempFile> {
    let mut stage = tempfile::NamedTempFile::new_in(path.parent().ok_or_else(unavailable)?)?;
    stage.write_all(bytes)?;
    stage.as_file().sync_all()?;
    Ok(stage)
}

fn replace_file(path: &Path, bytes: &[u8]) -> io::Result<()> {
    staged_file(path, bytes)?
        .persist(path)
        .map_err(|error| error.error)?;
    Ok(())
}

fn unavailable() -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        "Secure storage is unavailable for ChatGPT sign-in.",
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_account::{Registration, Session};
    use serde_json::json;

    fn fixture() -> AccountData {
        let registration = Registration {
            client_id: "fixture-client".into(),
            subject: "fixture-subject".into(),
            email: None,
        };
        AccountData {
            host_id: "urn:uuid:00000000-0000-4000-8000-000000000000".into(),
            registration: Some(registration.clone()),
            session: Some(Session {
                registration,
                access_token: "fixture-access".into(),
                refresh_token: "fixture-refresh".into(),
                expires_at: 123_456_789,
            }),
        }
    }

    #[test]
    fn round_trips_scoped_dpapi_storage_and_keeps_unrelated_profile_state() {
        let profile = tempfile::tempdir().unwrap();
        let state = profile.path().join(STATE_FILE);
        fs::write(&state, br#"{"existing":{"keep":true}}"#).unwrap();
        assert!(load(profile.path()).unwrap().is_none());
        save(profile.path(), &fixture()).unwrap();
        let state: Value = serde_json::from_slice(&fs::read(&state).unwrap()).unwrap();
        assert_eq!(state["existing"], json!({ "keep": true }));
        assert!(state["os_crypt"]["encrypted_key"].is_string());
        let loaded = load(profile.path()).unwrap().unwrap();
        assert_eq!(loaded.session.unwrap().access_token, "fixture-access");
        let data = fs::read(profile.path().join("codex-account").join(ACCOUNT_FILE)).unwrap();
        assert!(data.starts_with(DATA_HEADER));
        assert!(
            !data
                .windows(b"fixture-access".len())
                .any(|bytes| bytes == b"fixture-access")
        );
    }

    #[test]
    fn refuses_to_overwrite_an_unreadable_account_connection() {
        let profile = tempfile::tempdir().unwrap();
        save(profile.path(), &fixture()).unwrap();
        let path = profile.path().join("codex-account").join(ACCOUNT_FILE);
        let broken = b"invalid protected connection";
        fs::write(&path, broken).unwrap();
        assert!(load(profile.path()).is_err());
        assert!(save(profile.path(), &fixture()).is_err());
        assert_eq!(fs::read(path).unwrap(), broken);
    }
}
