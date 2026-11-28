use super::{AssistantTools, MAX_BYTES};
use crate::Project;
use serde_json::{Value, json};
use std::{
    fmt::Write as _,
    io::{self, BufRead, BufReader, Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};
use subtle::ConstantTimeEq;

const MAX_HEADERS: usize = 16 * 1024;
const MAX_BODY: usize = 6 * MAX_BYTES;
const CONNECTION_TIMEOUT: Duration = Duration::from_secs(15);

pub struct AssistantToolServer {
    address: SocketAddr,
    url: String,
    token: String,
    stop: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    worker: Option<JoinHandle<Project>>,
}

impl AssistantToolServer {
    pub fn start(mut tools: AssistantTools) -> io::Result<Self> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let address = listener.local_addr()?;
        let host = address.to_string();
        let url = format!("http://{host}/mcp");
        let mut random = [0_u8; 32];
        getrandom::fill(&mut random)
            .map_err(|_| io::Error::other("Couldn’t create an assistant tool capability"))?;
        let mut token = String::with_capacity(64);
        for byte in random {
            write!(token, "{byte:02x}").expect("string writes succeed");
        }
        let authorization = format!("Bearer {token}");
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = stop.clone();
        let cancel = tools.cancel.clone();
        let worker = thread::Builder::new()
            .name("refract-assistant-tools".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if worker_stop.load(Ordering::Acquire) {
                        break;
                    }
                    let Ok(mut stream) = stream else {
                        break;
                    };
                    if stream.set_read_timeout(Some(CONNECTION_TIMEOUT)).is_ok()
                        && stream.set_write_timeout(Some(CONNECTION_TIMEOUT)).is_ok()
                    {
                        let _ = serve(&mut stream, &host, authorization.as_bytes(), &mut tools);
                    }
                }
                tools.project
            })?;
        Ok(Self {
            address,
            url,
            token,
            stop,
            cancel,
            worker: Some(worker),
        })
    }

    pub fn url(&self) -> &str {
        &self.url
    }

    pub fn token(&self) -> &str {
        &self.token
    }

    pub fn finish(mut self) -> Option<Project> {
        self.stop_worker()
    }

    fn stop_worker(&mut self) -> Option<Project> {
        let worker = self.worker.take()?;
        self.stop.store(true, Ordering::Release);
        self.cancel.store(true, Ordering::Release);
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(250));
        worker.join().ok()
    }
}

impl Drop for AssistantToolServer {
    fn drop(&mut self) {
        self.stop_worker();
    }
}

fn serve(
    stream: &mut TcpStream,
    host: &str,
    authorization: &[u8],
    tools: &mut AssistantTools,
) -> io::Result<()> {
    let request = receive_request(&mut BufReader::new(&mut *stream), host, authorization);
    match request {
        Ok(request) => match response(request, tools) {
            Some(response) => send_response(stream, "200 OK", Some(response)),
            None => send_response(stream, "202 Accepted", None),
        },
        Err(failure) => send_response(stream, failure.status(), None),
    }
}

fn send_response(stream: &mut impl Write, status: &str, body: Option<Value>) -> io::Result<()> {
    let body = body
        .map(|value| serde_json::to_vec(&value))
        .transpose()?
        .unwrap_or_default();
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n",
        body.len()
    )?;
    if status.starts_with("405") {
        stream.write_all(b"Allow: POST\r\n")?;
    }
    stream.write_all(b"\r\n")?;
    stream.write_all(&body)?;
    stream.flush()
}

fn response(request: Value, tools: &mut AssistantTools) -> Option<Value> {
    let id = request.get("id").cloned();
    let method = request["method"].as_str();
    if request["jsonrpc"] != "2.0"
        || method.is_none()
        || id
            .as_ref()
            .is_some_and(|id| !id.is_string() && !id.is_i64())
    {
        return Some(rpc_error(
            id.unwrap_or(Value::Null),
            -32600,
            "Invalid request",
        ));
    }
    let id = id?;
    let data = match method.unwrap() {
        "initialize" => {
            let proposed = request["params"]["protocolVersion"]
                .as_str()
                .unwrap_or_default();
            let version = match proposed {
                "2024-11-05" | "2025-03-26" | "2025-06-18" | "2025-11-25" => proposed,
                _ => "2025-11-25",
            };
            json!({
                "protocolVersion":version,
                "capabilities":{"tools":{"listChanged":false}},
                "serverInfo":{"name":"refract","version":"1.0.0"},
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": AssistantTools::definitions()}),
        "tools/call" => {
            let params = &request["params"];
            let Some(name) = params["name"].as_str() else {
                return Some(rpc_error(id, -32602, "Invalid tool parameters"));
            };
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            if !arguments.is_object() {
                return Some(rpc_error(id, -32602, "Invalid tool arguments"));
            }
            match tools.call(name, arguments) {
                Ok(result) => json!({"content":[{"type":"text","text":result.to_string()}]}),
                Err(error) => {
                    json!({"isError":true,"content":[{"type":"text","text":error.to_string()}]})
                }
            }
        }
        _ => return Some(rpc_error(id, -32601, "Method not found")),
    };
    Some(json!({"jsonrpc":"2.0","id":id,"result":data}))
}

fn rpc_error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}

#[derive(Debug)]
enum HttpFailure {
    Invalid,
    Forbidden,
    NotFound,
    Method,
    Timeout,
    TooLarge,
    MediaType,
}

impl HttpFailure {
    fn status(&self) -> &'static str {
        match self {
            Self::Invalid => "400 Bad Request",
            Self::Forbidden => "403 Forbidden",
            Self::NotFound => "404 Not Found",
            Self::Method => "405 Method Not Allowed",
            Self::Timeout => "408 Request Timeout",
            Self::TooLarge => "413 Content Too Large",
            Self::MediaType => "415 Unsupported Media Type",
        }
    }
}

impl From<io::Error> for HttpFailure {
    fn from(error: io::Error) -> Self {
        match error.kind() {
            io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => Self::Timeout,
            _ => Self::Invalid,
        }
    }
}

fn receive_request(
    reader: &mut impl BufRead,
    host: &str,
    authorization: &[u8],
) -> Result<Value, HttpFailure> {
    let deadline = Instant::now() + CONNECTION_TIMEOUT;
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        let remaining = MAX_HEADERS.saturating_sub(header.len());
        read_line(reader, &mut header, remaining, deadline)?;
    }
    let mut fields = [httparse::EMPTY_HEADER; 48];
    let mut request = httparse::Request::new(&mut fields);
    if !request
        .parse(&header)
        .map_err(|_| HttpFailure::Invalid)?
        .is_complete()
    {
        return Err(HttpFailure::Invalid);
    }
    if single_header(request.headers, "origin")?.is_some()
        || single_header(request.headers, "host")? != Some(host.as_bytes())
        || !single_header(request.headers, "authorization")?
            .is_some_and(|provided| bool::from(provided.ct_eq(authorization)))
    {
        return Err(HttpFailure::Forbidden);
    }
    if request.path != Some("/mcp") {
        return Err(HttpFailure::NotFound);
    }
    if request.method != Some("POST") {
        return Err(HttpFailure::Method);
    }
    let media_type = single_header(request.headers, "content-type")?
        .and_then(|value| std::str::from_utf8(value).ok())
        .and_then(|value| value.split(';').next())
        .map(str::trim);
    if !media_type.is_some_and(|value| value.eq_ignore_ascii_case("application/json")) {
        return Err(HttpFailure::MediaType);
    }
    let content_length = single_header(request.headers, "content-length")?;
    let transfer_encoding = single_header(request.headers, "transfer-encoding")?;
    let body = match (content_length, transfer_encoding) {
        (Some(length), None) => {
            let length = std::str::from_utf8(length)
                .ok()
                .and_then(|length| length.parse().ok())
                .ok_or(HttpFailure::Invalid)?;
            let mut body = Vec::new();
            read_body(reader, &mut body, length, deadline)?;
            body
        }
        (None, Some(encoding)) if encoding.eq_ignore_ascii_case(b"chunked") => {
            read_chunks(reader, deadline)?
        }
        _ => return Err(HttpFailure::Invalid),
    };
    serde_json::from_slice(&body).map_err(|_| HttpFailure::Invalid)
}

fn single_header<'a>(
    headers: &[httparse::Header<'a>],
    name: &str,
) -> Result<Option<&'a [u8]>, HttpFailure> {
    let mut headers = headers
        .iter()
        .filter(|header| header.name.eq_ignore_ascii_case(name));
    let value = headers.next().map(|header| header.value);
    if headers.next().is_some() {
        return Err(HttpFailure::Invalid);
    }
    Ok(value)
}

fn read_line(
    reader: &mut impl BufRead,
    output: &mut Vec<u8>,
    limit: usize,
    deadline: Instant,
) -> Result<(), HttpFailure> {
    let start = output.len();
    loop {
        if Instant::now() >= deadline {
            return Err(HttpFailure::Timeout);
        }
        let bytes = reader.fill_buf()?;
        if bytes.is_empty() {
            return Err(HttpFailure::Invalid);
        }
        let end = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map(|index| index + 1);
        let count = end.unwrap_or(bytes.len());
        if output.len() - start + count > limit {
            return Err(HttpFailure::TooLarge);
        }
        output.extend_from_slice(&bytes[..count]);
        reader.consume(count);
        if end.is_some() {
            return Ok(());
        }
    }
}

fn read_body(
    reader: &mut impl Read,
    output: &mut Vec<u8>,
    mut count: usize,
    deadline: Instant,
) -> Result<(), HttpFailure> {
    if count > MAX_BODY.saturating_sub(output.len()) {
        return Err(HttpFailure::TooLarge);
    }
    let mut buffer = [0; 8192];
    while count > 0 {
        if Instant::now() >= deadline {
            return Err(HttpFailure::Timeout);
        }
        let max = count.min(buffer.len());
        let read = reader.read(&mut buffer[..max])?;
        if read == 0 {
            return Err(HttpFailure::Invalid);
        }
        output.extend_from_slice(&buffer[..read]);
        count -= read;
    }
    Ok(())
}

fn read_chunks(reader: &mut impl BufRead, deadline: Instant) -> Result<Vec<u8>, HttpFailure> {
    let mut body = Vec::new();
    let mut line = Vec::new();
    loop {
        line.clear();
        read_line(reader, &mut line, 1024, deadline)?;
        let count = line
            .strip_suffix(b"\r\n")
            .and_then(|line| line.split(|byte| *byte == b';').next())
            .and_then(|count| std::str::from_utf8(count).ok())
            .and_then(|count| usize::from_str_radix(count, 16).ok())
            .ok_or(HttpFailure::Invalid)?;
        if count == 0 {
            let mut trailer = Vec::new();
            read_line(reader, &mut trailer, MAX_HEADERS, deadline)?;
            while trailer != b"\r\n" && !trailer.ends_with(b"\r\n\r\n") {
                let remaining = MAX_HEADERS.saturating_sub(trailer.len());
                read_line(reader, &mut trailer, remaining, deadline)?;
            }
            return Ok(body);
        }
        read_body(reader, &mut body, count, deadline)?;
        let mut ending = [0; 2];
        reader.read_exact(&mut ending)?;
        if ending != *b"\r\n" {
            return Err(HttpFailure::Invalid);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{conversation::ConversationId, create_project};
    use std::io::Cursor;

    const HOST: &str = "127.0.0.1:41234";
    const AUTHORIZATION: &str = "Bearer fixture-capability";

    fn request(headers: &str, body: &str) -> Cursor<Vec<u8>> {
        Cursor::new(format!(
            "POST /mcp HTTP/1.1\r\nHost: {HOST}\r\nAuthorization: {AUTHORIZATION}\r\nContent-Type: application/json\r\n{headers}\r\n{body}"
        ).into_bytes())
    }

    #[test]
    fn accepts_bounded_json_with_fixed_or_chunked_body() {
        let body = r#"{"jsonrpc":"2.0","method":"ping","id":1}"#;
        let size = format!("Content-Length: {}\r\n", body.len());
        let expected: Value = serde_json::from_str(body).unwrap();
        assert_eq!(
            receive_request(&mut request(&size, body), HOST, AUTHORIZATION.as_bytes()).unwrap(),
            expected
        );
        let chunks = format!("{:x}\r\n{body}\r\n0\r\n\r\n", body.len());
        assert_eq!(
            receive_request(
                &mut request("Transfer-Encoding: chunked\r\n", &chunks),
                HOST,
                AUTHORIZATION.as_bytes()
            )
            .unwrap(),
            expected
        );
    }

    #[test]
    fn rejects_cross_origin_invalid_authority_ambiguous_or_oversized_requests() {
        assert!(matches!(
            receive_request(
                &mut request("Content-Length: 2\r\nOrigin: https://example.org\r\n", "{}"),
                HOST,
                AUTHORIZATION.as_bytes()
            ),
            Err(HttpFailure::Forbidden)
        ));
        assert!(matches!(
            receive_request(
                &mut request("Content-Length: 2\r\n", "{}"),
                "localhost:41234",
                AUTHORIZATION.as_bytes()
            ),
            Err(HttpFailure::Forbidden)
        ));
        assert!(matches!(
            receive_request(
                &mut request("Content-Length: 2\r\n", "{}"),
                HOST,
                b"Bearer different-capability"
            ),
            Err(HttpFailure::Forbidden)
        ));
        assert!(matches!(
            receive_request(
                &mut request("Content-Length: 2\r\nTransfer-Encoding: chunked\r\n", "{}"),
                HOST,
                AUTHORIZATION.as_bytes()
            ),
            Err(HttpFailure::Invalid)
        ));
        let length = format!("Content-Length: {}\r\n", MAX_BODY + 1);
        assert!(matches!(
            receive_request(&mut request(&length, ""), HOST, AUTHORIZATION.as_bytes()),
            Err(HttpFailure::TooLarge)
        ));
    }

    #[test]
    fn serves_only_registered_tools_for_the_selected_project() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let mut tools = AssistantTools::new(
            root.path().to_owned(),
            project,
            ConversationId::default(),
            Arc::new(AtomicBool::new(false)),
        );
        let init = response(
            json!({
                "jsonrpc":"2.0","method":"initialize","id":"init",
                "params":{"protocolVersion":"2025-03-26"}
            }),
            &mut tools,
        )
        .unwrap();
        assert_eq!(init["result"]["protocolVersion"], "2025-03-26");
        assert!(
            response(
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                &mut tools
            )
            .is_none()
        );
        let list = response(
            json!({"jsonrpc":"2.0","method":"tools/list","id":1}),
            &mut tools,
        )
        .unwrap();
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 9);
        let args = json!({"name":"project_list"});
        let call = response(
            json!({"jsonrpc":"2.0","method":"tools/call","params":args,"id":2}),
            &mut tools,
        )
        .unwrap();
        let content: Value =
            serde_json::from_str(call["result"]["content"][0]["text"].as_str().unwrap()).unwrap();
        assert_eq!(content["files"], json!(["main.tex"]));
    }

    #[test]
    #[ignore = "opens a loopback listener for CI"]
    fn serves_an_authenticated_turn_and_releases_its_listener() {
        let root = tempfile::tempdir().unwrap();
        let project = create_project(root.path(), "Paper").unwrap();
        let tools = AssistantTools::new(
            root.path().to_owned(),
            project.clone(),
            ConversationId::default(),
            Arc::new(AtomicBool::new(false)),
        );
        let server = AssistantToolServer::start(tools).unwrap();
        assert!(server.address.ip().is_loopback());
        let body =
            r#"{"jsonrpc":"2.0","method":"tools/call","id":1,"params":{"name":"project_list"}}"#;
        let mut client = TcpStream::connect(server.address).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        write!(
            client,
            "POST /mcp HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
            server.address,
            server.token(),
            body.len(),
            body,
        ).unwrap();
        let mut reply = String::new();
        client.read_to_string(&mut reply).unwrap();
        assert!(reply.starts_with("HTTP/1.1 200"));
        let (_, reply) = reply.split_once("\r\n\r\n").unwrap();
        let result: Value = serde_json::from_str(reply).unwrap();
        assert_eq!(
            result["result"]["content"][0]["text"],
            r#"{"files":["main.tex"]}"#
        );
        let address = server.address;
        assert_eq!(server.finish().unwrap().id, project.id);
        assert!(TcpStream::connect_timeout(&address, Duration::from_millis(250)).is_err());
    }
}
