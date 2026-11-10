//! The HTTP side of the control port: 127.0.0.1 only, a bearer token on every request, one thread
//! per request. Requests that need the game (Lua, status, screenshots) are queued to the game loop
//! and answered from there; event reads are served straight from the shared log.

use crate::events::{EventLog, Filter};
use crossbeam_channel::Sender;
use serde_json::{json, Value};
use std::io::Read;
use std::sync::Arc;
use std::time::Duration;
use tiny_http::{Header, Method, Request as HttpRequest, Response, Server};

/// The read-only web view of what the agent sees (#62).
const VIEW: &str = include_str!("view.html");
/// How long an HTTP thread waits for the game loop before answering 504.
const GAME_TIMEOUT: Duration = Duration::from_secs(60);
/// The longest an events read may wait for the next event.
const MAX_WAIT: Duration = Duration::from_secs(30);
/// Events one read returns unless it names a `limit`: few enough for a language model to read.
const DEFAULT_EVENTS: usize = 100;
/// Lua chunks larger than this are refused.
const MAX_BODY: usize = 1 << 20;

/// Work for the game loop, each with the channel its answer goes back on.
pub(crate) enum Job {
    Lua {
        code: String,
        budget: Option<u64>,
        reply: Sender<Value>,
    },
    Status {
        reply: Sender<Value>,
    },
    Screenshot {
        scale: f32,
        format: Format,
        reply: Sender<Result<Vec<u8>, String>>,
    },
}

/// How `/screenshot` encodes a frame: PNG (lossless, the default), or JPEG at a quality of 1-100,
/// several times smaller and quicker to encode, for a live view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Format {
    Png,
    Jpeg(u8),
}

impl Format {
    /// `format=jpeg` with an optional `quality` (default 85), else PNG.
    fn parse(format: Option<&str>, quality: Option<&str>) -> Self {
        match format {
            Some("jpeg" | "jpg") => {
                let quality = quality.and_then(|q| q.parse::<u8>().ok()).unwrap_or(85);
                Format::Jpeg(quality.clamp(1, 100))
            }
            _ => Format::Png,
        }
    }

    fn content_type(self) -> &'static str {
        match self {
            Format::Png => "image/png",
            Format::Jpeg(_) => "image/jpeg",
        }
    }
}

pub(crate) struct Port {
    pub token: String,
    pub jobs: Sender<Job>,
    pub events: Arc<EventLog>,
    /// The agent's modules (`env::scripts_dir`), served by `/files`.
    pub scripts: std::path::PathBuf,
}

/// Bind and serve on background threads. Returns the bound address.
pub(crate) fn start(port: u16, shared: Port) -> Result<std::net::SocketAddr, String> {
    let server =
        Server::http(("127.0.0.1", port)).map_err(|e| format!("bind 127.0.0.1:{port}: {e}"))?;
    let address = server
        .server_addr()
        .to_ip()
        .ok_or("control port did not bind an IP address")?;
    let shared = Arc::new(shared);
    std::thread::Builder::new()
        .name("agent-port".into())
        .spawn(move || {
            for request in server.incoming_requests() {
                let shared = shared.clone();
                let _ = std::thread::Builder::new()
                    .name("agent-request".into())
                    .spawn(move || handle(&shared, request));
            }
        })
        .map_err(|e| e.to_string())?;
    Ok(address)
}

fn handle(shared: &Port, mut request: HttpRequest) {
    let response =
        if !authorized(&request, &shared.token) && !view_with_token(&request, &shared.token) {
            json_response(401, &json!({"error": "missing or wrong bearer token"}))
        } else {
            route(shared, &mut request)
        };
    let _ = request.respond(response);
}

fn authorized(request: &HttpRequest, token: &str) -> bool {
    let expected = format!("Bearer {token}");
    request.headers().iter().any(|h| {
        h.field.equiv("Authorization")
            && constant_time_eq(h.value.as_str().as_bytes(), expected.as_bytes())
    })
}

/// `GET /view?token=…`: a browser cannot set a header from the address bar, so the page alone takes
/// the token as a query parameter (loopback only, like everything here).
fn view_with_token(request: &HttpRequest, token: &str) -> bool {
    let url = request.url();
    *request.method() == Method::Get
        && url.split('?').next() == Some("/view")
        && url
            .split_once('?')
            .map(|(_, q)| {
                q.split('&')
                    .any(|kv| constant_time_eq(kv.as_bytes(), format!("token={token}").as_bytes()))
            })
            .unwrap_or(false)
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

type Reply = Response<std::io::Cursor<Vec<u8>>>;

fn route(shared: &Port, request: &mut HttpRequest) -> Reply {
    let (path, query) = request
        .url()
        .split_once('?')
        .map_or((request.url().to_owned(), String::new()), |(p, q)| {
            (p.to_owned(), q.to_owned())
        });
    let param = |name: &str| {
        query
            .split('&')
            .filter_map(|kv| kv.split_once('='))
            .find(|(k, _)| *k == name)
            .map(|(_, v)| v.to_owned())
    };
    if path == "/files" || path.starts_with("/files/") {
        return files(
            shared,
            request,
            path.trim_start_matches("/files").trim_start_matches('/'),
        );
    }
    match (request.method(), path.as_str()) {
        (Method::Get, "/view") => Response::from_data(
            VIEW.replace(
                "__TOKEN__",
                // A JS string literal that cannot close the script tag.
                &json!(shared.token).to_string().replace('<', "\\u003c"),
            )
            .into_bytes(),
        )
        .with_header(header("Content-Type", "text/html; charset=utf-8"))
        .with_header(header("Cache-Control", "no-store")),
        (Method::Get, "/status") => ask(shared, |reply| Job::Status { reply }),
        (Method::Post, "/lua") => {
            let mut body = String::new();
            if let Err(e) = request
                .as_reader()
                .take(MAX_BODY as u64 + 1)
                .read_to_string(&mut body)
            {
                return json_response(400, &json!({"error": format!("body: {e}")}));
            }
            if body.len() > MAX_BODY {
                return json_response(413, &json!({"error": "chunk larger than 1 MiB"}));
            }
            let budget = param("budget").and_then(|b| b.parse().ok());
            ask(shared, |reply| Job::Lua {
                code: body,
                budget,
                reply,
            })
        }
        (Method::Get, "/screenshot") => {
            let scale = param("scale")
                .and_then(|s| s.parse::<f32>().ok())
                .filter(|s| *s > 0.0 && *s <= 1.0)
                .unwrap_or(1.0);
            let format = Format::parse(param("format").as_deref(), param("quality").as_deref());
            let (reply, answer) = crossbeam_channel::bounded(1);
            let job = Job::Screenshot {
                scale,
                format,
                reply,
            };
            if shared.jobs.send(job).is_err() {
                return json_response(503, &json!({"error": "game loop stopped"}));
            }
            match answer.recv_timeout(GAME_TIMEOUT) {
                Ok(Ok(bytes)) => Response::from_data(bytes)
                    .with_header(header("Content-Type", format.content_type())),
                Ok(Err(e)) => json_response(500, &json!({"error": e})),
                Err(_) => json_response(504, &json!({"error": "no frame in time"})),
            }
        }
        (Method::Get, "/events") => {
            // No `since` reads from now (only what arrives during `wait_ms`); `since=0` is the
            // whole retained buffer.
            let since = param("since").and_then(|s| s.parse().ok());
            let limit = param("limit")
                .and_then(|s| s.parse().ok())
                .unwrap_or(DEFAULT_EVENTS)
                .clamp(1, 10_000);
            let wait = param("wait_ms")
                .and_then(|s| s.parse().ok())
                .map(Duration::from_millis)
                .unwrap_or(Duration::ZERO)
                .min(MAX_WAIT);
            let filter = Filter::parse(
                param("names").map(|v| percent_decode(&v)).as_deref(),
                param("exclude").map(|v| percent_decode(&v)).as_deref(),
            );
            let read = shared.events.read(since, limit, wait, &filter);
            json_response(
                200,
                &json!({"events": read.events, "next": read.next, "missed": read.missed}),
            )
        }
        _ => json_response(
            404,
            &json!({"error": "unknown route", "routes": ["GET /status", "POST /lua", "GET /screenshot", "GET /events", "GET /view", "GET|PUT|DELETE /files/<module>"]}),
        ),
    }
}

/// A query value with its `%XX` escapes and `+` undone (a client's URL encoder writes the commas
/// of `names=A,B` as `%2C`). A broken escape is kept as written.
fn percent_decode(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = (bytes[i] == b'%')
            .then(|| value.get(i + 1..i + 3))
            .flatten()
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match hex {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(if bytes[i] == b'+' { b' ' } else { bytes[i] });
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The agent's modules: `GET /files` lists them, `GET|PUT|DELETE /files/<module>` reads, writes
/// (creating folders) or removes one. Module names are dotted words (`grind.loop`), so a path can
/// never leave the scripts folder. Writing does not reload: `AgentReload(name)` in Lua does.
fn files(shared: &Port, request: &mut HttpRequest, name: &str) -> Reply {
    let dir = &shared.scripts;
    if name.is_empty() {
        return match request.method() {
            Method::Get => json_response(
                200,
                &json!({"modules": crate::env::list(dir), "folder": dir.display().to_string()}),
            ),
            _ => json_response(405, &json!({"error": "GET /files lists modules"})),
        };
    }
    if !crate::env::valid_module(name) {
        return json_response(
            400,
            &json!({"error": "module names are dotted words: grind.loop"}),
        );
    }
    let path = crate::env::module_path(dir, name);
    match request.method() {
        Method::Get => match std::fs::read(&path) {
            Ok(source) => Response::from_data(source)
                .with_header(header("Content-Type", "text/plain; charset=utf-8")),
            Err(_) => json_response(404, &json!({"error": format!("no module {name}")})),
        },
        Method::Put => {
            let mut body = Vec::new();
            if let Err(e) = request
                .as_reader()
                .take(MAX_BODY as u64 + 1)
                .read_to_end(&mut body)
            {
                return json_response(400, &json!({"error": format!("body: {e}")}));
            }
            if body.len() > MAX_BODY {
                return json_response(413, &json!({"error": "module larger than 1 MiB"}));
            }
            let written = path
                .parent()
                .map_or(Ok(()), std::fs::create_dir_all)
                .and_then(|()| std::fs::write(&path, &body));
            match written {
                Ok(()) => json_response(200, &json!({"saved": name, "bytes": body.len()})),
                Err(e) => json_response(500, &json!({"error": e.to_string()})),
            }
        }
        Method::Delete => match std::fs::remove_file(&path) {
            Ok(()) => json_response(200, &json!({"deleted": name})),
            Err(_) => json_response(404, &json!({"error": format!("no module {name}")})),
        },
        _ => json_response(405, &json!({"error": "GET, PUT or DELETE"})),
    }
}

/// Queue a job for the game loop and wait for its JSON answer.
fn ask(shared: &Port, job: impl FnOnce(Sender<Value>) -> Job) -> Reply {
    let (reply, answer) = crossbeam_channel::bounded(1);
    if shared.jobs.send(job(reply)).is_err() {
        return json_response(503, &json!({"error": "game loop stopped"}));
    }
    match answer.recv_timeout(GAME_TIMEOUT) {
        Ok(value) => json_response(200, &value),
        Err(_) => json_response(504, &json!({"error": "the game did not answer in time"})),
    }
}

fn header(name: &str, value: &str) -> Header {
    Header::from_bytes(name.as_bytes(), value.as_bytes()).expect("static header")
}

fn json_response(status: u16, value: &Value) -> Reply {
    Response::from_data(value.to_string().into_bytes())
        .with_status_code(status)
        .with_header(header("Content-Type", "application/json"))
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
