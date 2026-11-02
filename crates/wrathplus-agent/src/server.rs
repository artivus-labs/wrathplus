//! The HTTP side of the control port: 127.0.0.1 only, a bearer token on every request, one thread
//! per request. Requests that need the game (Lua, status, screenshots) are queued to the game loop
//! and answered from there; event reads are served straight from the shared log.

use crate::events::EventLog;
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
        reply: Sender<Result<Vec<u8>, String>>,
    },
}

pub(crate) struct Port {
    pub token: String,
    pub jobs: Sender<Job>,
    pub events: Arc<EventLog>,
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
            let (reply, answer) = crossbeam_channel::bounded(1);
            if shared.jobs.send(Job::Screenshot { scale, reply }).is_err() {
                return json_response(503, &json!({"error": "game loop stopped"}));
            }
            match answer.recv_timeout(GAME_TIMEOUT) {
                Ok(Ok(png)) => {
                    Response::from_data(png).with_header(header("Content-Type", "image/png"))
                }
                Ok(Err(e)) => json_response(500, &json!({"error": e})),
                Err(_) => json_response(504, &json!({"error": "no frame in time"})),
            }
        }
        (Method::Get, "/events") => {
            let since = param("since").and_then(|s| s.parse().ok()).unwrap_or(0);
            let limit = param("limit")
                .and_then(|s| s.parse().ok())
                .unwrap_or(1_000usize)
                .clamp(1, 10_000);
            let wait = param("wait_ms")
                .and_then(|s| s.parse().ok())
                .map(Duration::from_millis)
                .unwrap_or(Duration::ZERO)
                .min(MAX_WAIT);
            let read = shared.events.read(since, limit, wait);
            json_response(
                200,
                &json!({"events": read.events, "next": read.next, "missed": read.missed}),
            )
        }
        _ => json_response(
            404,
            &json!({"error": "unknown route", "routes": ["GET /status", "POST /lua", "GET /screenshot", "GET /events", "GET /view"]}),
        ),
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
mod tests {
    use super::*;
    use std::io::Write;

    /// A raw HTTP/1.1 exchange, so the tests need no client crate.
    fn call(address: std::net::SocketAddr, request: &str) -> (u16, String) {
        let mut stream = std::net::TcpStream::connect(address).unwrap();
        stream.write_all(request.as_bytes()).unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let status = response[9..12].parse().unwrap();
        let body = response.split_once("\r\n\r\n").unwrap().1.to_owned();
        (status, body)
    }

    fn get(address: std::net::SocketAddr, path: &str, token: &str) -> (u16, String) {
        call(
            address,
            &format!("GET {path} HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {token}\r\nConnection: close\r\n\r\n"),
        )
    }

    const TOKEN: &str = "test-token-0123456789";

    /// A port with a stand-in game loop that answers Lua with the code it was sent.
    fn port() -> (std::net::SocketAddr, Arc<EventLog>) {
        let (jobs, queue) = crossbeam_channel::unbounded::<Job>();
        let events = Arc::new(EventLog::new());
        let address = start(
            0,
            Port {
                token: TOKEN.into(),
                jobs,
                events: events.clone(),
            },
        )
        .unwrap();
        std::thread::spawn(move || {
            for job in queue {
                match job {
                    Job::Lua {
                        code,
                        budget,
                        reply,
                    } => {
                        let _ = reply.send(json!({"ok": true, "values": [code], "budget": budget}));
                    }
                    Job::Status { reply } => {
                        let _ = reply.send(json!({"state": "Login"}));
                    }
                    Job::Screenshot { reply, .. } => {
                        let _ = reply.send(Err("no window in tests".into()));
                    }
                }
            }
        });
        (address, events)
    }

    #[test]
    fn every_route_needs_the_token() {
        let (address, _) = port();
        assert_eq!(get(address, "/status", "wrong-token").0, 401);
        let (status, _) = call(
            address,
            "GET /status HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        );
        assert_eq!(status, 401);
        assert_eq!(
            get(address, "/status", TOKEN),
            (200, r#"{"state":"Login"}"#.into())
        );
    }

    #[test]
    fn lua_goes_to_the_game_loop_with_its_budget() {
        let (address, _) = port();
        let code = "return UnitName('player')";
        let (status, body) = call(
            address,
            &format!(
                "POST /lua?budget=5000 HTTP/1.1\r\nHost: x\r\nAuthorization: Bearer {TOKEN}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{code}",
                code.len()
            ),
        );
        assert_eq!(status, 200);
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["values"][0], code);
        assert_eq!(body["budget"], 5000);
    }

    #[test]
    fn events_are_read_by_cursor_and_unknown_routes_list_the_real_ones() {
        let (address, events) = port();
        events.push([json!({"event": "PLAYER_LOGIN", "args": []})]);
        let (status, body) = get(address, "/events?since=0&wait_ms=10", TOKEN);
        assert_eq!(status, 200);
        let body: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(body["events"][0]["event"], "PLAYER_LOGIN");
        assert_eq!(body["next"], 2);
        let (status, body) = get(address, "/nope", TOKEN);
        assert_eq!(status, 404);
        assert!(body.contains("POST /lua"));
        assert_eq!(get(address, "/screenshot", TOKEN).0, 500);
    }

    #[test]
    fn the_view_takes_its_token_from_the_address_and_carries_it() {
        let (address, _) = port();
        let (status, page) = call(
            address,
            &format!("GET /view?token={TOKEN} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n"),
        );
        assert_eq!(status, 200);
        assert!(page.contains(&format!("const TOKEN = \"{TOKEN}\";")));
        let wrong = "GET /view?token=nope HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n";
        assert_eq!(call(address, wrong).0, 401);
        // The address-bar token opens the page only, not the API.
        let api =
            format!("GET /status?token={TOKEN} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
        assert_eq!(call(address, &api).0, 401);
    }
}
