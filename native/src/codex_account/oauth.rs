use super::{Registration, Session};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use ring::signature::{RSA_PKCS1_2048_8192_SHA256, RsaPublicKeyComponents};
use serde::{Deserialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fmt, io,
    io::{Read, Write},
    net::{Ipv4Addr, TcpListener, TcpStream},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use subtle::ConstantTimeEq;
use url::{Url, form_urlencoded};
use zed_reqwest::{blocking::Client, redirect};
use zeroize::{Zeroize, Zeroizing};

const ISSUER: &str = "https://auth.openai.com";
const RESOURCE: &str = "https://api.openai.com/v1";
const AUTHORIZE: &str = "https://auth.openai.com/api/accounts/authorize";
const TOKEN: &str = "https://auth.openai.com/api/accounts/oauth/token";
const REVOKE: &str = "https://auth.openai.com/api/accounts/oauth/revoke";
const JWKS: &str = "https://auth.openai.com/.well-known/jwks.json";
const SCOPE: &str = "chatgpt.tokens.use.direct";
const SCOPES: &str =
    "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct";
const DYNAMIC_CLIENT: &str = "dynamic_agent_client";
const CALLBACK: &str = "/auth/callback";
const NETWORK_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_HTTP_BODY: u64 = 4 * 1024 * 1024;
const MAX_CALLBACK: usize = 8192;
const POLL: Duration = Duration::from_millis(100);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Error {
    Cancelled,
    SignInRequired(&'static str),
    Failed(&'static str),
}

impl fmt::Display for Error {
    fn fmt(&self, display: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => display.write_str("ChatGPT sign-in was cancelled."),
            Self::SignInRequired(message) | Self::Failed(message) => display.write_str(message),
        }
    }
}

impl std::error::Error for Error {}

const SIGN_IN_FAILED: Error = Error::Failed("Could not sign in to ChatGPT. Try again.");
const EXCHANGE_FAILED: Error = Error::Failed("OpenAI could not complete ChatGPT sign-in.");
const IDENTITY_FAILED: Error = Error::Failed("Could not verify ChatGPT identity.");
const MISSING_SESSION: Error = Error::Failed("OpenAI returned an incomplete ChatGPT session.");

#[derive(Default, Deserialize)]
struct Tokens {
    access_token: Option<String>,
    refresh_token: Option<String>,
    expires_in: Option<f64>,
    id_token: Option<String>,
    token_type: Option<String>,
    scope: Option<String>,
    error: Option<String>,
}

impl Drop for Tokens {
    fn drop(&mut self) {
        self.access_token.zeroize();
        self.refresh_token.zeroize();
        self.id_token.zeroize();
    }
}

struct Challenge {
    state: String,
    nonce: String,
    verifier: Zeroizing<String>,
}

impl Challenge {
    fn new() -> Result<Self, Error> {
        Ok(Self {
            state: random_challenge()?,
            nonce: random_challenge()?,
            verifier: Zeroizing::new(random_challenge()?),
        })
    }

    fn url(&self, host_id: &str, registration: Option<&Registration>, redirect: &str) -> Url {
        let client_id = registration.map_or(DYNAMIC_CLIENT, |value| &value.client_id);
        let encoded = URL_SAFE_NO_PAD.encode(Sha256::digest(self.verifier.as_bytes()));
        let mut url = Url::parse(AUTHORIZE).expect("fixed HTTPS authorization endpoint");
        let mut query = url.query_pairs_mut();
        query.extend_pairs([
            ("client_id", client_id),
            ("ext_agent_host_id", host_id),
            ("response_type", "code"),
            ("redirect_uri", redirect),
            ("resource", RESOURCE),
            ("scope", SCOPES),
            ("state", &self.state),
            ("nonce", &self.nonce),
            ("code_challenge_method", "S256"),
            ("code_challenge", &encoded),
        ]);
        if let Some(registration) = registration {
            if let Some(email) = registration.email.as_deref() {
                query.append_pair("login_hint", email);
            }
        } else {
            query.append_pair("agent_name_hint", "Refract");
        }
        drop(query);
        url
    }
}

fn random_challenge() -> Result<String, Error> {
    let mut bytes = Zeroizing::new([0_u8; 32]);
    getrandom::fill(&mut *bytes).map_err(|_| SIGN_IN_FAILED)?;
    Ok(URL_SAFE_NO_PAD.encode(&*bytes))
}

pub fn authorize(
    host_id: &str,
    registration: Option<&Registration>,
    open_browser: impl FnOnce(&str) -> io::Result<()>,
    cancel: &AtomicBool,
) -> Result<Session, Error> {
    check_cancelled(cancel)?;
    let deadline = Instant::now() + Duration::from_secs(5 * 60);
    let challenge = Challenge::new()?;
    let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|_| SIGN_IN_FAILED)?;
    listener.set_nonblocking(true).map_err(|_| SIGN_IN_FAILED)?;
    let host = listener
        .local_addr()
        .map_err(|_| SIGN_IN_FAILED)?
        .to_string();
    let redirect = format!("http://{host}{CALLBACK}");
    let url = challenge.url(host_id, registration, &redirect);
    open_browser(url.as_str()).map_err(|_| SIGN_IN_FAILED)?;
    let mut callback = wait_callback(&listener, &host, &challenge.state, deadline, cancel)?;
    drop(listener);
    if let Some(error) = callback.remove("error") {
        return Err(if error == "access_denied" {
            Error::Cancelled
        } else {
            Error::Failed("ChatGPT sign-in failed.")
        });
    }
    let code = Zeroizing::new(callback.remove("code").unwrap_or_default());
    let returned = callback.get("client_id").filter(|value| !value.is_empty());
    let client = registration.map(|value| &value.client_id).or(returned);
    let Some(client_id) = client.filter(|value| {
        !value.is_empty()
            && value.as_str() != DYNAMIC_CLIENT
            && returned.is_none_or(|returned| returned == *value)
            && !code.is_empty()
    }) else {
        return Err(Error::Failed("OpenAI did not finish registering Refract."));
    };
    let client = client_builder(NETWORK_TIMEOUT)?;
    let tokens = exchange(
        &client,
        &[
            ("grant_type", "authorization_code"),
            ("client_id", client_id),
            ("code", &code),
            ("code_verifier", &challenge.verifier),
            ("redirect_uri", &redirect),
        ],
        false,
        cancel,
    )?;
    let id_token = tokens.id_token.as_deref().ok_or(IDENTITY_FAILED)?;
    let identity = verify_identity(&client, id_token, client_id, &challenge.nonce, cancel)?;
    if registration.is_some_and(|registered| registered.subject != identity.subject) {
        return Err(Error::Failed(
            "Choose the same ChatGPT account to reconnect.",
        ));
    }
    let session = session(identity, tokens, None)?;
    check_cancelled(cancel)?;
    Ok(session)
}

pub fn renew(existing: &Session, cancel: &AtomicBool) -> Result<Session, Error> {
    let client = client_builder(NETWORK_TIMEOUT)?;
    let tokens = exchange(
        &client,
        &[
            ("grant_type", "refresh_token"),
            ("client_id", &existing.registration.client_id),
            ("refresh_token", &existing.refresh_token),
        ],
        true,
        cancel,
    )?;
    session(
        existing.registration.clone(),
        tokens,
        Some(&existing.refresh_token),
    )
}

pub fn revoke(existing: &Session, cancel: &AtomicBool) -> bool {
    let Ok(client) = client_builder(Duration::from_secs(3)) else {
        return false;
    };
    for delay in [0, 300, 900] {
        let until = Instant::now() + Duration::from_millis(delay);
        while Instant::now() < until && check_cancelled(cancel).is_ok() {
            thread::sleep(POLL);
        }
        if check_cancelled(cancel).is_err() {
            return false;
        }
        if let Ok(response) = form(
            &client,
            REVOKE,
            [
                ("token", existing.refresh_token.as_str()),
                ("token_type_hint", "refresh_token"),
                ("client_id", existing.registration.client_id.as_str()),
            ],
        )
        .send()
            && !response.status().is_server_error()
        {
            return response.status().is_success();
        }
    }
    false
}

fn client_builder(timeout: Duration) -> Result<Client, Error> {
    Client::builder()
        .https_only(true)
        .redirect(redirect::Policy::none())
        .connect_timeout(timeout)
        .timeout(timeout)
        .build()
        .map_err(|_| EXCHANGE_FAILED)
}

fn form<'a>(
    client: &Client,
    url: &str,
    fields: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> zed_reqwest::blocking::RequestBuilder {
    let body = form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish();
    client
        .post(url)
        .header("Content-Type", "application/x-www-form-urlencoded")
        .body(body)
}

fn exchange(
    client: &Client,
    fields: &[(&str, &str)],
    refresh: bool,
    cancel: &AtomicBool,
) -> Result<Tokens, Error> {
    check_cancelled(cancel)?;
    let response = form(
        client,
        TOKEN,
        fields.iter().copied().chain([("resource", RESOURCE)]),
    )
    .send()
    .map_err(|_| EXCHANGE_FAILED)?;
    let success = response.status().is_success();
    let tokens: Tokens = read_json(response, EXCHANGE_FAILED)?;
    check_cancelled(cancel)?;
    if !success {
        if refresh
            && matches!(
                tokens.error.as_deref(),
                Some("invalid_grant" | "invalid_client")
            )
        {
            return Err(Error::SignInRequired(
                "Your ChatGPT session ended. Sign in again in Settings.",
            ));
        }
        return Err(EXCHANGE_FAILED);
    }
    Ok(tokens)
}

fn session(
    registration: Registration,
    mut tokens: Tokens,
    old_refresh: Option<&str>,
) -> Result<Session, Error> {
    if (tokens.scope.is_some() || old_refresh.is_none())
        && !tokens
            .scope
            .as_deref()
            .is_some_and(|scope| scope.split_whitespace().any(|word| word == SCOPE))
    {
        return Err(Error::SignInRequired(
            "ChatGPT plan usage was not granted. Sign in again and allow Refract to use your plan.",
        ));
    }
    if !tokens
        .token_type
        .as_deref()
        .is_some_and(|kind| kind.eq_ignore_ascii_case("bearer"))
    {
        return Err(MISSING_SESSION);
    }
    let expires_in = tokens
        .expires_in
        .filter(|seconds| seconds.is_finite() && *seconds > 0.)
        .ok_or(MISSING_SESSION)?;
    let expiry = now_ms()
        .checked_add((expires_in * 1000.) as u64)
        .ok_or(MISSING_SESSION)?;
    let access_token = Zeroizing::new(
        tokens
            .access_token
            .take()
            .filter(|token| !token.is_empty())
            .ok_or(MISSING_SESSION)?,
    );
    let refresh_token = tokens
        .refresh_token
        .take()
        .or_else(|| old_refresh.map(str::to_owned))
        .filter(|token| !token.is_empty())
        .ok_or(MISSING_SESSION)?;
    Ok(Session {
        registration,
        access_token: access_token.to_string(),
        refresh_token,
        expires_at: expiry,
    })
}

fn check_cancelled(cancel: &AtomicBool) -> Result<(), Error> {
    if cancel.load(Ordering::Relaxed) {
        return Err(Error::Cancelled);
    }
    Ok(())
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis() as u64)
}

fn read_json<T: DeserializeOwned>(body: impl Read, error: Error) -> Result<T, Error> {
    let mut bytes = Zeroizing::new(Vec::new());
    body.take(MAX_HTTP_BODY + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| error)?;
    if bytes.len() as u64 > MAX_HTTP_BODY {
        return Err(error);
    }
    serde_json::from_slice(&bytes).map_err(|_| error)
}

#[derive(Deserialize)]
struct TokenHeader {
    alg: String,
    kid: String,
}

#[derive(Deserialize)]
struct KeySet {
    keys: Vec<IdentityKey>,
}

#[derive(Deserialize)]
struct IdentityKey {
    kty: String,
    kid: String,
    n: Option<String>,
    e: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum Audience {
    One(String),
    Many(Vec<String>),
}

#[derive(Deserialize)]
struct Claims {
    iss: String,
    aud: Audience,
    azp: Option<String>,
    nonce: String,
    exp: f64,
    sub: String,
    email: Option<String>,
}

fn verify_identity(
    client: &Client,
    token: &str,
    client_id: &str,
    nonce: &str,
    cancel: &AtomicBool,
) -> Result<Registration, Error> {
    let (signed, signature) = token.rsplit_once('.').ok_or(IDENTITY_FAILED)?;
    let (header, payload) = signed.split_once('.').ok_or(IDENTITY_FAILED)?;
    let decode = |text| URL_SAFE_NO_PAD.decode(text).map_err(|_| IDENTITY_FAILED);
    let header: TokenHeader =
        serde_json::from_slice(&decode(header)?).map_err(|_| IDENTITY_FAILED)?;
    if header.alg != "RS256" || header.kid.is_empty() {
        return Err(IDENTITY_FAILED);
    }
    check_cancelled(cancel)?;
    let response = client
        .get(JWKS)
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|_| IDENTITY_FAILED)?;
    let keys: KeySet = read_json(response, IDENTITY_FAILED)?;
    let mut candidates = keys
        .keys
        .iter()
        .filter(|key| key.kty == "RSA" && key.kid == header.kid);
    let key = candidates.next().ok_or(IDENTITY_FAILED)?;
    if candidates.next().is_some() {
        return Err(IDENTITY_FAILED);
    }
    let modulus = decode(key.n.as_deref().ok_or(IDENTITY_FAILED)?)?;
    let exponent = decode(key.e.as_deref().ok_or(IDENTITY_FAILED)?)?;
    let key = RsaPublicKeyComponents {
        n: modulus,
        e: exponent,
    };
    key.verify(
        &RSA_PKCS1_2048_8192_SHA256,
        signed.as_bytes(),
        &decode(signature)?,
    )
    .map_err(|_| IDENTITY_FAILED)?;
    let claims: Claims = serde_json::from_slice(&decode(payload)?).map_err(|_| IDENTITY_FAILED)?;
    let audience = match &claims.aud {
        Audience::One(value) => std::slice::from_ref(value),
        Audience::Many(values) => values.as_slice(),
    };
    if claims.iss != ISSUER
        || !audience.iter().any(|audience| audience == client_id)
        || ((audience.len() > 1 || claims.azp.is_some())
            && claims.azp.as_deref() != Some(client_id))
        || !bool::from(claims.nonce.as_bytes().ct_eq(nonce.as_bytes()))
        || !claims.exp.is_finite()
        || claims.exp <= now_ms() as f64 / 1000.
        || claims.sub.is_empty()
    {
        return Err(IDENTITY_FAILED);
    }
    Ok(Registration {
        client_id: client_id.to_owned(),
        subject: claims.sub,
        email: claims.email,
    })
}

fn wait_callback(
    listener: &TcpListener,
    host: &str,
    state: &str,
    deadline: Instant,
    cancel: &AtomicBool,
) -> Result<HashMap<String, String>, Error> {
    loop {
        check_cancelled(cancel)?;
        if Instant::now() >= deadline {
            return Err(Error::Failed("ChatGPT sign-in timed out."));
        }
        match listener.accept() {
            Ok((mut stream, peer)) if peer.ip().is_loopback() => {
                stream
                    .set_read_timeout(Some(POLL))
                    .map_err(|_| SIGN_IN_FAILED)?;
                stream
                    .set_write_timeout(Some(POLL))
                    .map_err(|_| SIGN_IN_FAILED)?;
                let until = deadline.min(Instant::now() + Duration::from_secs(3));
                let callback = read_callback(&mut stream, host, state, until, cancel);
                let (status, text) = if callback.is_some() {
                    (
                        "200 OK",
                        "Return to Refract to finish connecting your ChatGPT account.",
                    )
                } else {
                    ("400 Bad Request", "Invalid Refract sign-in callback.")
                };
                let _ = write!(
                    stream,
                    "HTTP/1.1 {status}\r\nCache-Control: no-store\r\nContent-Type: text/plain; charset=utf-8\r\nReferrer-Policy: no-referrer\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                );
                if let Some(callback) = callback {
                    return Ok(callback);
                }
            }
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL),
            Err(_) => return Err(SIGN_IN_FAILED),
        }
    }
}

fn read_callback(
    stream: &mut TcpStream,
    host: &str,
    state: &str,
    deadline: Instant,
    cancel: &AtomicBool,
) -> Option<HashMap<String, String>> {
    let mut bytes = Zeroizing::new([0_u8; MAX_CALLBACK]);
    let mut used = 0;
    while used < bytes.len() && Instant::now() < deadline && check_cancelled(cancel).is_ok() {
        match stream.read(&mut bytes[used..]) {
            Ok(0) => return None,
            Ok(count) => {
                used += count;
                if bytes[..used].windows(4).any(|window| window == b"\r\n\r\n") {
                    return parse_callback(&bytes[..used], host, state);
                }
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
                ) => {}
            Err(_) => return None,
        }
    }
    None
}

fn parse_callback(bytes: &[u8], host: &str, state: &str) -> Option<HashMap<String, String>> {
    let mut headers = [httparse::EMPTY_HEADER; 48];
    let mut request = httparse::Request::new(&mut headers);
    if !request.parse(bytes).ok()?.is_complete() || request.method != Some("GET") {
        return None;
    }
    let mut hosts = request
        .headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case("Host"));
    if hosts.next()?.value != host.as_bytes() || hosts.next().is_some() {
        return None;
    }
    let (path, query) = request.path?.split_once('?')?;
    if path != CALLBACK {
        return None;
    }
    let mut parameters = HashMap::new();
    for (name, value) in form_urlencoded::parse(query.as_bytes()) {
        if matches!(name.as_ref(), "state" | "code" | "client_id" | "error")
            && parameters
                .insert(name.into_owned(), value.into_owned())
                .is_some()
        {
            return None;
        }
    }
    let callback_state = parameters.remove("state")?;
    bool::from(callback_state.as_bytes().ct_eq(state.as_bytes())).then_some(parameters)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_accepts_matching_loopback_oauth_callbacks() {
        let host = "127.0.0.1:12345";
        let request = |target, host| format!("GET {target} HTTP/1.1\r\nHost: {host}\r\n\r\n");
        let valid = request("/auth/callback?state=expected&code=synthetic-code", host);
        assert_eq!(
            parse_callback(valid.as_bytes(), host, "expected").unwrap()["code"],
            "synthetic-code"
        );
        for (path, header) in [
            ("/auth/callback?state=wrong&code=synthetic-code", host),
            ("/auth/callback?state=expected&state=expected", host),
            ("/auth/callback?state=expected", "localhost:12345"),
            ("//127.0.0.1:12345/auth/callback?state=expected", host),
        ] {
            assert!(parse_callback(request(path, header).as_bytes(), host, "expected").is_none());
        }
    }
}
