use super::{CANCEL_POLL, CANCELLED, ManagedProcess, check_cancelled};
use crate::{
    assistant_launch::OpenCodeCli,
    assistant_preferences::{valid_model_id, valid_opencode_variant},
};
use serde_json::Value;
use std::{
    collections::HashSet,
    fmt::Write as _,
    io::{self, BufRead, BufReader, Read},
    net::{Ipv4Addr, SocketAddrV4},
    path::Path,
    process::Stdio,
    sync::{
        atomic::AtomicBool,
        mpsc::{self, RecvTimeoutError},
    },
    thread,
    time::{Duration, Instant},
};

const STARTUP_LIMIT: usize = 16 * 1024;
const MAX_CATALOG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MODELS: usize = 2000;
const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OpenCodeCatalogModel {
    pub id: String,
    pub name: String,
    pub variants: Vec<String>,
}

pub fn discover_opencode_models(
    cli: &OpenCodeCli,
    workspace: &Path,
    cancel: &AtomicBool,
) -> Result<Vec<OpenCodeCatalogModel>, String> {
    check_cancelled(cancel)?;
    let mut secret = [0_u8; 32];
    getrandom::fill(&mut secret).map_err(|_| "Could not authenticate OpenCode model discovery")?;
    let mut password = String::with_capacity(64);
    for byte in secret {
        write!(password, "{byte:02x}").expect("string writes succeed");
    }
    let mut command = cli
        .catalog_server_command(Some(workspace), &password)
        .map_err(|_| "Could not configure OpenCode model discovery")?;
    command
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let process = ManagedProcess::spawn(command, cancel)
        .map_err(|_| "Could not start OpenCode model discovery")?;
    thread::scope(move |scope| {
        let mut process = process;
        let output = process
            .child
            .stdout
            .take()
            .ok_or("Missing OpenCode catalog server output")?;
        let (ready, address) = mpsc::sync_channel(1);
        thread::Builder::new()
            .spawn_scoped(scope, move || {
                let mut output = BufReader::new(output);
                let address = read_address(&mut output);
                let _ = ready.send(address);
                let _ = io::copy(&mut output, &mut io::sink());
            })
            .map_err(|_| "Could not read OpenCode model discovery")?;
        let deadline = Instant::now() + TIMEOUT;
        let result = loop {
            check_cancelled(cancel)?;
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                break Err("OpenCode model discovery timed out".into());
            }
            match address.recv_timeout(remaining.min(CANCEL_POLL)) {
                Ok(Ok(address)) => {
                    break request_catalog(
                        address,
                        &password,
                        deadline.saturating_duration_since(Instant::now()),
                    );
                }
                Ok(Err(error)) => break Err(error.into()),
                Err(RecvTimeoutError::Disconnected) => {
                    break Err("OpenCode model discovery stopped unexpectedly".into());
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
        };
        drop(process);
        check_cancelled(cancel).map_err(|_| CANCELLED.to_owned())?;
        result
    })
}

fn read_address(input: &mut impl BufRead) -> Result<SocketAddrV4, &'static str> {
    let mut bounded = input.take(STARTUP_LIMIT as u64);
    let mut line = String::new();
    while bounded
        .read_line(&mut line)
        .map_err(|_| "Invalid OpenCode server response")?
        != 0
    {
        if let Some(port) = parse_listening_port(&line) {
            return Ok(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
        }
        line.clear();
    }
    Err("OpenCode model discovery stopped before reporting its loopback address")
}

fn parse_listening_port(line: &str) -> Option<u16> {
    let (_, address) = line.split_once("server listening on http://127.0.0.1:")?;
    let port_end = address
        .find(|character: char| !character.is_ascii_digit())
        .unwrap_or(address.len());
    let tail = &address[port_end..];
    if !matches!(tail.trim(), "" | "/") {
        return None;
    }
    address[..port_end]
        .parse::<u16>()
        .ok()
        .filter(|port| *port != 0)
}

fn request_catalog(
    address: SocketAddrV4,
    password: &str,
    remaining: Duration,
) -> Result<Vec<OpenCodeCatalogModel>, String> {
    if remaining.is_zero() {
        return Err("OpenCode model discovery timed out".into());
    }
    let client = zed_reqwest::blocking::Client::builder()
        .no_proxy()
        .redirect(zed_reqwest::redirect::Policy::none())
        .connect_timeout(remaining.min(Duration::from_secs(3)))
        .timeout(remaining.min(Duration::from_secs(15)))
        .build()
        .map_err(|_| "Could not start the OpenCode catalog client")?;
    let response = client
        .get(format!("http://{address}/api/model"))
        .basic_auth("opencode", Some(password))
        .send()
        .map_err(|_| "Could not load OpenCode model catalog")?;
    if !response.status().is_success() {
        return Err("Could not load OpenCode model catalog".into());
    }
    let mut bytes = Vec::new();
    response
        .take(MAX_CATALOG_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| "Could not read OpenCode model catalog")?;
    if bytes.len() as u64 > MAX_CATALOG_BYTES {
        return Err("OpenCode model catalog is too large".into());
    }
    let catalog = serde_json::from_slice(&bytes).map_err(|_| "Invalid OpenCode model catalog")?;
    parse_models(&catalog).map_err(str::to_owned)
}

fn parse_models(catalog: &Value) -> Result<Vec<OpenCodeCatalogModel>, &'static str> {
    let data = catalog["data"]
        .as_array()
        .ok_or("Invalid OpenCode model catalog")?;
    if data.len() > MAX_MODELS {
        return Err("OpenCode model catalog is too large");
    }
    let mut ids = HashSet::new();
    let mut models = Vec::new();
    for item in data {
        let (Some(provider), Some(model), Some(variants)) = (
            item["providerID"].as_str(),
            item["id"].as_str(),
            item["variants"].as_array(),
        ) else {
            continue;
        };
        if provider.is_empty()
            || provider.contains(['/', '#'])
            || provider.chars().any(char::is_whitespace)
            || model.is_empty()
            || model.contains('#')
            || model.chars().any(char::is_whitespace)
        {
            continue;
        }
        let id = format!("{provider}/{model}");
        if id.starts_with('-') || !valid_model_id(&id) || !ids.insert(id.clone()) {
            continue;
        }
        let name = item["name"]
            .as_str()
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .unwrap_or(model);
        let mut seen = HashSet::new();
        let variants = variants
            .iter()
            .filter_map(|variant| variant["id"].as_str())
            .filter(|candidate| valid_opencode_variant(candidate) && seen.insert(*candidate))
            .map(str::to_owned)
            .collect();
        models.push(OpenCodeCatalogModel {
            id,
            name: name.into(),
            variants,
        });
    }
    Ok(models)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Cursor;

    #[test]
    fn accepts_only_an_ephemeral_ipv4_loopback_endpoint() {
        let output = b"ready\nserver listening on http://127.0.0.1:41097\n";
        let address = read_address(&mut Cursor::new(output)).unwrap();
        assert_eq!(address, SocketAddrV4::new(Ipv4Addr::LOCALHOST, 41097));
        assert!(parse_listening_port("server listening on http://example.org:41097").is_none());
        assert!(
            parse_listening_port("server listening on http://127.0.0.1:41097@example.org")
                .is_none()
        );
        assert!(parse_listening_port("server listening on http://127.0.0.1:0").is_none());
    }

    #[test]
    fn handles_provider_models_and_deduplicates_valid_variants() {
        let models = parse_models(&json!({"data":[
            {"providerID":"local","id":"family/model","name":" Research ","variants":[{"id":"low"},{"id":"low"},{"id":"invalid#variant"},{"id":"high"}]},
            {"providerID":"invalid/provider","id":"name","variants":[]},
            {"providerID":"local","id":"second","variants":[]}
        ]})).unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(
            models[0],
            OpenCodeCatalogModel {
                id: "local/family/model".into(),
                name: "Research".into(),
                variants: vec!["low".into(), "high".into()]
            }
        );
        assert_eq!(models[1].name, "second");
    }
}
