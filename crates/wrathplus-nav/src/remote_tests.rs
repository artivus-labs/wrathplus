//! `remote`'s tests. The realm's side of each exchange is `mod-agent-nav`'s own code
//! (`csrc/realm_module_shim.cpp` compiles the module's headers), so these check the client's
//! encoder and decoder against what the realm really reads and writes, and the module's routes
//! against this crate's.

use super::*;
use std::ffi::{c_char, c_int, CString};

extern "C" {
    fn realm_parse(payload: *const c_char, out: *mut c_char, out_len: c_int) -> c_int;
    fn realm_check(
        payload: *const c_char,
        at: *const f32,
        max_distance: f32,
        max_start: f32,
        out: *mut c_char,
        out_len: c_int,
    ) -> c_int;
    fn realm_avoid(
        lines: *const c_char,
        max_avoid: c_int,
        take: c_int,
        out: *mut c_char,
        out_len: c_int,
    ) -> c_int;
    fn realm_hello_reply(out: *mut c_char, out_len: c_int) -> c_int;
    fn realm_error_reply(
        id: c_int,
        code: *const c_char,
        detail: *const c_char,
        out: *mut c_char,
        out_len: c_int,
    ) -> c_int;
    fn realm_route_reply(
        id: c_int,
        flags: c_int,
        points: *const f32,
        count: c_int,
        out: *mut c_char,
        out_len: c_int,
    ) -> c_int;
    fn realm_rate(
        times: *const u64,
        count: c_int,
        per_second: c_int,
        per_minute: c_int,
        verdicts: *mut u8,
    );
}

/// Call one of the shim's text-returning functions.
fn text(call: impl FnOnce(*mut c_char, c_int) -> c_int) -> String {
    let mut out = vec![0u8; 64 * 1024];
    let len = call(out.as_mut_ptr().cast(), out.len() as c_int);
    assert!(len >= 0 && (len as usize) < out.len());
    String::from_utf8(out[..len as usize].to_vec()).unwrap()
}

fn c(s: &str) -> CString {
    CString::new(s).unwrap()
}

/// What the module makes of a client line.
fn parsed(line: &str) -> String {
    let line = c(line);
    text(|out, len| unsafe { realm_parse(line.as_ptr(), out, len) })
}

fn request() -> Request {
    Request {
        id: 7,
        from: [-8907.23, -113.6, 81.94],
        to: [-8902.6, -162.64],
        to_z: Some(82.0),
        near: false,
        any_height: false,
        avoid: Vec::new(),
    }
}

#[test]
fn the_module_reads_a_request_as_the_client_wrote_it() {
    assert_eq!(
        encode(&request()),
        ["P 7 - -8907.2 -113.6 81.9 -8902.6 -162.6 82.0"]
    );
    assert_eq!(
        parsed(&encode(&request())[0]),
        "path 7 exact -8907.200 -113.600 81.900 -8902.600 -162.600 82.000 z"
    );
    // A leg with no height: the realm starts from ours.
    let leg = Request {
        to_z: None,
        near: true,
        from: [-0.04, 12.0, 3.0],
        ..request()
    };
    assert_eq!(encode(&leg), ["P 7 n 0.0 12.0 3.0 -8902.6 -162.6 -"]);
    assert_eq!(
        parsed(&encode(&leg)[0]),
        "path 7 near 0.000 12.000 3.000 -8902.600 -162.600 3.000 noz"
    );
    // A flat point: the height is the player's guess, and the realm may look on other levels.
    let flat = Request {
        any_height: true,
        ..request()
    };
    assert_eq!(
        encode(&flat),
        ["P 7 h -8907.2 -113.6 81.9 -8902.6 -162.6 82.0"]
    );
    assert_eq!(
        parsed(&encode(&flat)[0]),
        "path 7 exact anyz -8907.200 -113.600 81.900 -8902.600 -162.600 82.000 z"
    );
    assert_eq!(
        parsed("P 7 nh 1 2 3 4 5 -"),
        "path 7 near anyz 1.000 2.000 3.000 4.000 5.000 3.000 noz"
    );
    assert_eq!(parsed(&hello()), "hello 1");
}

#[test]
fn circles_go_ahead_of_the_request_in_as_many_lines_as_they_need() {
    let avoid: Vec<[f32; 3]> = (0..64)
        .map(|i| [-9460.0 - i as f32 * 3.3, 64.0 + i as f32, 23.0])
        .collect();
    let lines = encode(&Request {
        avoid: avoid.clone(),
        ..request()
    });
    assert!(lines.len() > 3 && lines.iter().all(|l| l.len() <= MAX_PAYLOAD));
    assert!(lines.last().unwrap().starts_with("P 7 "));
    // The module reads every circle back, and holds all 64 for request 7 only.
    let mut read = Vec::new();
    for line in &lines[..lines.len() - 1] {
        let dump = parsed(line);
        let mut fields = dump.split(' ');
        assert_eq!((fields.next(), fields.next()), (Some("avoid"), Some("7")));
        let numbers: Vec<f32> = fields.map(|f| f.parse().unwrap()).collect();
        read.extend(numbers.as_chunks::<3>().0.iter().copied());
    }
    assert_eq!(read.len(), 64);
    for (got, want) in read.iter().zip(&avoid) {
        assert!((0..3).all(|i| (got[i] - want[i]).abs() < 0.051));
    }
    let sent = c(&lines[..lines.len() - 1].join("\n"));
    let held = |max: c_int, take: c_int| {
        text(|out, len| unsafe { realm_avoid(sent.as_ptr(), max, take, out, len) })
    };
    assert_eq!(held(64, 7), "64");
    assert_eq!(held(64, 8), "0");
    assert_eq!(held(63, 7), "!circles: at most 63");
}

#[test]
fn the_module_refuses_what_it_cannot_read() {
    for (line, why) in [
        ("", "!length 0"),
        ("X 1", "!verb 0"),
        ("H", "!hello 0"),
        ("H 0", "!hello 0"),
        ("P 0 - 1 2 3 4 5 6", "!id 0"),
        ("P 70000 - 1 2 3 4 5 6", "!id 0"),
        ("P 9 - 1 2 3 4 5", "!fields 9"),
        ("P 9 x 1 2 3 4 5 6", "!flags 9"),
        ("P 9 nn 1 2 3 4 5 6", "!flags 9"),
        ("P 9 hnh 1 2 3 4 5 6", "!flags 9"),
        ("P 9 - 1e3 2 3 4 5 6", "!from 9"),
        ("P 9 - nan 2 3 4 5 6", "!from 9"),
        ("P 9 - 1 2 3 30000 5 6", "!to 9"),
        ("P 9 - 1 2 3 4 5 0x10", "!to 9"),
        ("P 9 - 1 2 3 4 5 6.", "!to 9"),
        ("P 9 -  1 2 3 4 5", "!from 9"),
        ("A 9 1 2", "!circles 9"),
        ("A 9 1 2 0", "!circle 9"),
        ("A 9 1 2 500", "!circle 9"),
        ("P 9 - 1 2 3 4 5 6\u{7}", "!characters 0"),
    ] {
        assert_eq!(parsed(line), why, "{line:?}");
    }
    assert_eq!(parsed(&"P 9 - 1 2 3 4 5 6".repeat(20)), "!length 0");
}

#[test]
fn the_probe_answer_carries_the_realm_limits() {
    let line = text(|out, len| unsafe { realm_hello_reply(out, len) });
    assert_eq!(line, "h 1 1500 256 64 5 240");
    assert_eq!(
        decode(&line),
        Ok(Reply::Hello(Limits {
            max_distance: 1500.0,
            max_points: 256,
            max_avoid: 64,
            per_second: 5,
            per_minute: 240,
        }))
    );
    // A realm without the module delivers our own whisper back.
    assert_eq!(decode(&hello()), Ok(Reply::Echo));
    assert_eq!(decode(&encode(&request())[0]), Ok(Reply::Echo));
    assert!(decode("h 1 1500").is_err());
    assert!(decode("z").is_err());
}

/// A long winding route, as the module's reply lines.
fn reply(points: &[[f32; 3]], flags: c_int) -> Vec<String> {
    let flat: Vec<f32> = points.iter().flatten().copied().collect();
    let joined = text(|out, len| unsafe {
        realm_route_reply(41, flags, flat.as_ptr(), points.len() as c_int, out, len)
    });
    joined.split('\n').map(str::to_string).collect()
}

fn winding(count: usize) -> Vec<[f32; 3]> {
    (0..count)
        .map(|i| {
            let t = i as f32;
            [
                -9460.37 + t * 7.13 + (t * 0.7).sin() * 40.0,
                64.21 - t * 3.31 + (t * 1.3).cos() * 55.0,
                56.0 + (t * 0.4).sin() * 12.5,
            ]
        })
        .collect()
}

#[test]
fn a_long_route_comes_back_in_parts_and_goes_together_again() {
    let points = winding(256);
    let lines = reply(&points, 1 | 2);
    assert!(lines.len() > 8, "{} parts", lines.len());
    assert!(lines.iter().all(|l| l.len() <= MAX_PAYLOAD));
    let mut assembler = Assembler::default();
    let mut route = None;
    for (i, line) in lines.iter().enumerate() {
        let Ok(Reply::Part(part)) = decode(line) else {
            panic!("{line}");
        };
        assert_eq!(
            (part.id, part.part as usize, part.parts as usize),
            (41, i + 1, lines.len())
        );
        route = assembler.push(&part).unwrap();
        assert_eq!(route.is_some(), i + 1 == lines.len());
    }
    let route = route.unwrap();
    assert!(route.complete && route.steep);
    assert_eq!(route.points.len(), points.len());
    for (got, want) in route.points.iter().zip(&points) {
        assert!(
            (0..3).all(|i| (got[i] - want[i]).abs() < 0.051),
            "{got:?} {want:?}"
        );
    }
    // Cut at the corner limit: not a complete route, whatever else it says.
    let Ok(Reply::Part(cut)) = decode(&reply(&points[..3], 1 | 4)[0]) else {
        panic!();
    };
    let route = Assembler::default().push(&cut).unwrap().unwrap();
    assert!(!route.complete && !route.steep && route.points.len() == 3);
    assert_eq!(reply(&points[..1], 0)[0], "r 41 1 1 - -94604,1192,560");
}

#[test]
fn parts_out_of_turn_are_an_error_and_the_next_reply_still_reads() {
    let lines = reply(&winding(60), 1);
    let parts: Vec<Part> = lines
        .iter()
        .map(|l| match decode(l) {
            Ok(Reply::Part(p)) => p,
            other => panic!("{other:?}"),
        })
        .collect();
    assert!(parts.len() >= 3);
    let mut assembler = Assembler::default();
    assert_eq!(assembler.push(&parts[0]), Ok(None));
    assert!(assembler
        .push(&parts[2])
        .unwrap_err()
        .contains("came apart"));
    // Starting over from part 1 works.
    for part in &parts[..parts.len() - 1] {
        assert_eq!(assembler.push(part), Ok(None));
    }
    assert!(assembler.push(parts.last().unwrap()).unwrap().is_some());
    assert!(decode("r 41 1 1 c 1,2").is_err());
    assert!(decode("r 41 1 1 q 1,2,3").is_err());
    assert!(decode("r 41 1 1 c").is_ok_and(|r| match r {
        Reply::Part(p) => Assembler::default().push(&p).is_err(),
        _ => false,
    }));
}

#[test]
fn refusals_are_read_and_put_in_words() {
    let refused = |code: &str, detail: &str| {
        let (code, detail) = (c(code), c(detail));
        let line = text(|out, len| unsafe {
            realm_error_reply(12, code.as_ptr(), detail.as_ptr(), out, len)
        });
        match decode(&line) {
            Ok(Reply::Refused {
                id: 12,
                refusal,
                detail,
            }) => (refusal, refusal.explain(&detail)),
            other => panic!("{line}: {other:?}"),
        }
    };
    assert_eq!(
        refused("rate", "5 a second"),
        (
            Refusal::Rate,
            "the realm refused the route: too many requests (it allows 5 a second per character); wait a moment".into()
        )
    );
    assert_eq!(
        refused("far", "1500"),
        (
            Refusal::Far,
            "the realm routes at most 1500 yd at a time".into()
        )
    );
    assert_eq!(
        refused("nomesh", "this realm has no navmesh for this map").0,
        Refusal::NoMesh
    );
    assert_eq!(refused("noroute", "x").0, Refusal::NoRoute);
    assert_eq!(refused("busy", "x").0, Refusal::Busy);
    assert_eq!(refused("bad", "from").0, Refusal::Bad);
    assert_eq!(refused("start", "x").0, Refusal::Start);
    assert_eq!(
        refused("later", "why not"),
        (
            Refusal::Other,
            "the realm refused the route (later why not)".into()
        )
    );
}

#[test]
fn the_module_checks_distance_and_where_the_route_starts() {
    let line = c(&encode(&request())[0]);
    let check = |at: [f32; 3], max_distance: f32, max_start: f32| {
        text(|out, len| unsafe {
            realm_check(
                line.as_ptr(),
                at.as_ptr(),
                max_distance,
                max_start,
                out,
                len,
            )
        })
    };
    let here = [-8907.0, -113.0, 82.0];
    assert_eq!(check(here, 1500.0, 100.0), "");
    assert_eq!(check(here, 40.0, 100.0), "e 7 far 40");
    assert_eq!(check(here, 0.0, 0.0), "");
    let elsewhere = [-8700.0, -113.0, 82.0];
    assert_eq!(
        check(elsewhere, 1500.0, 100.0),
        "e 7 start routes start within 100 yd of the character"
    );
    assert_eq!(check(elsewhere, 1500.0, 0.0), "");
}

#[test]
fn the_rate_limit_counts_a_second_and_a_minute_and_forgets() {
    let run = |times: &[u64], per_second: c_int, per_minute: c_int| {
        let mut verdicts = vec![9u8; times.len()];
        unsafe {
            realm_rate(
                times.as_ptr(),
                times.len() as c_int,
                per_second,
                per_minute,
                verdicts.as_mut_ptr(),
            )
        };
        verdicts
    };
    // Three a second: the fourth within the second is refused, and let in once it has passed.
    assert_eq!(
        run(&[0, 100, 200, 300, 999, 1000, 1100], 3, 0),
        [0, 0, 0, 1, 1, 0, 0]
    );
    // Four a minute: refused until the first is a minute old. A refusal is not counted.
    assert_eq!(
        run(
            &[0, 5_000, 10_000, 15_000, 20_000, 59_999, 60_000, 60_001],
            0,
            4
        ),
        [0, 0, 0, 0, 2, 2, 0, 2]
    );
    assert_eq!(run(&[0, 1, 2, 3, 4, 5], 0, 0), [0; 6]);
}
