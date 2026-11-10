//! The control port over a real socket, with a stand-in game loop: the token, the routes, the
//! event reads and the module files.

use super::*;
use std::io::Write;

#[test]
fn screenshots_are_png_unless_jpeg_is_asked_for() {
    assert_eq!(Format::parse(None, None), Format::Png);
    assert_eq!(Format::parse(Some("png"), Some("50")), Format::Png);
    assert_eq!(Format::parse(Some("jpeg"), None), Format::Jpeg(85));
    assert_eq!(Format::parse(Some("jpg"), Some("90")), Format::Jpeg(90));
    assert_eq!(Format::parse(Some("jpeg"), Some("0")), Format::Jpeg(1));
    assert_eq!(Format::parse(Some("jpeg"), Some("250")), Format::Jpeg(100));
    assert_eq!(Format::parse(Some("jpeg"), Some("high")), Format::Jpeg(85));
    assert_eq!(Format::Jpeg(90).content_type(), "image/jpeg");
}

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
            scripts: std::env::temp_dir()
                .join(format!("wp-agent-port-test-{}", std::process::id())),
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

fn read_events(address: std::net::SocketAddr, query: &str) -> (Vec<String>, u64, u64) {
    let (status, body) = get(address, &format!("/events{query}"), TOKEN);
    assert_eq!(status, 200);
    let body: Value = serde_json::from_str(&body).unwrap();
    let names = body["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["event"].as_str().unwrap().to_owned())
        .collect();
    (
        names,
        body["next"].as_u64().unwrap(),
        body["missed"].as_u64().unwrap(),
    )
}

#[test]
fn events_default_to_from_now_and_a_hundred_and_filter_by_name() {
    let (address, events) = port();
    events.push((0..250).map(|i| {
        let event = match i % 5 {
            0 => "AGENT_WALK_DONE",
            1 => "CHAT_MSG_SAY",
            2 => "UNIT_HEALTH",
            _ => "COMBAT_LOG_EVENT_UNFILTERED",
        };
        json!({"event": event, "args": []})
    }));
    // No cursor: nothing old, only where to read on from.
    assert_eq!(read_events(address, ""), (vec![], 251, 0));
    assert_eq!(read_events(address, "?wait_ms=20"), (vec![], 251, 0));
    // `since=0` is the whole buffer, a hundred events at a time.
    let (first, next, missed) = read_events(address, "?since=0");
    assert_eq!((first.len(), next, missed), (100, 101, 0));
    let (all, next, _) = read_events(address, "?since=0&limit=5000");
    assert_eq!((all.len(), next), (250, 251));
    // By name or prefix, with a client's encoded commas, less what is excluded.
    let (named, next, _) = read_events(
        address,
        "?since=0&limit=3&names=AGENT_%2CCHAT_MSG_*&exclude=CHAT_MSG_SAY",
    );
    assert_eq!(named, ["AGENT_WALK_DONE"; 3]);
    assert_eq!(next, 12, "after the third match, to carry on from");
    let (rest, next, _) = read_events(address, "?since=12&names=AGENT_,CHAT_MSG_SAY");
    assert_eq!((rest.len(), next), (95, 251));
    let (quiet, next, _) = read_events(
        address,
        "?since=0&exclude=AGENT_,CHAT_MSG_,UNIT_,COMBAT_LOG*",
    );
    assert_eq!((quiet.len(), next), (0, 251));
}

#[test]
fn a_wait_with_names_ends_only_on_a_matching_event() {
    let (address, events) = port();
    let writer = events.clone();
    let handle = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(40));
        writer.push([json!({"event": "UNIT_AURA", "args": []})]);
        std::thread::sleep(Duration::from_millis(150));
        writer.push([json!({"event": "AGENT_COMBAT_DONE", "args": ["0x1", "killed"]})]);
    });
    let started = std::time::Instant::now();
    let (got, next, _) = read_events(address, "?wait_ms=5000&names=AGENT_");
    handle.join().unwrap();
    assert!(started.elapsed() >= Duration::from_millis(150));
    assert_eq!((got, next), (vec!["AGENT_COMBAT_DONE".to_owned()], 3));
}

#[test]
fn query_values_are_percent_decoded() {
    assert_eq!(percent_decode("AGENT_%2CCHAT_MSG_%2a"), "AGENT_,CHAT_MSG_*");
    assert_eq!(percent_decode("a+b%"), "a b%");
    assert_eq!(percent_decode("%zz%4"), "%zz%4");
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
    let api = format!("GET /status?token={TOKEN} HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n");
    assert_eq!(call(address, &api).0, 401);
}

#[test]
fn modules_are_saved_listed_read_and_deleted_inside_the_scripts_folder() {
    let (address, _) = port();
    let auth = format!("Authorization: Bearer {TOKEN}\r\nConnection: close\r\n");
    let source = "return { hello = true }";
    let put = format!(
        "PUT /files/grind.loop HTTP/1.1\r\nHost: x\r\n{auth}Content-Length: {}\r\n\r\n{source}",
        source.len()
    );
    assert_eq!(call(address, &put).0, 200);
    let (status, body) = get(address, "/files", TOKEN);
    assert_eq!(status, 200);
    assert!(body.contains("\"grind.loop\""));
    assert_eq!(
        get(address, "/files/grind.loop", TOKEN),
        (200, source.to_string())
    );
    assert_eq!(get(address, "/files/..%2Fetc", TOKEN).0, 400);
    let delete = format!("DELETE /files/grind.loop HTTP/1.1\r\nHost: x\r\n{auth}\r\n");
    assert_eq!(call(address, &delete).0, 200);
    assert_eq!(get(address, "/files/grind.loop", TOKEN).0, 404);
}
