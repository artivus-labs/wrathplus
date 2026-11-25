use super::*;

fn w(v: u32) -> Vec<u8> {
    v.to_le_bytes().to_vec()
}

fn fixtures() -> Vec<(u16, Vec<u8>, SessionEvent)> {
    let c = |packet| SessionEvent::WrathSessionMeta { packet };
    let level_up: Vec<u8> = (1..=14u32).flat_map(u32::to_le_bytes).collect();
    vec![
        (
            0x0fd,
            (0..32).collect(),
            SessionEvent::TutorialFlags((0..32).collect()),
        ),
        (
            0x0fa,
            w(41),
            SessionEvent::CinematicTriggered { cinematic_id: 41 },
        ),
        (
            0x1cd,
            [w(3600), w(60), vec![1]].concat(),
            SessionEvent::PlayedTime {
                total: 3600,
                level: 60,
            },
        ),
        (
            0x1d4,
            level_up,
            SessionEvent::LevelUp(LevelUpInfo {
                level: 1,
                health: 2,
                powers: [3, 4, 5, 6, 7],
                stats: [10, 11, 12, 13, 14],
            }),
        ),
        (
            0x2f4,
            [w(4), 0.5f32.to_le_bytes().to_vec(), vec![1]].concat(),
            SessionEvent::Weather {
                weather_type: 1,
                grade: 0.5,
                sound_id: 0,
                instant: true,
            },
        ),
        (
            0x209,
            [w(1_700_000_000), vec![1], w(0x15), w(1), w(2), w(3)].concat(),
            c(Packet::AccountDataTimes {
                server_time: 1_700_000_000,
                raw_byte: 1,
                mask: 0x15,
                times: vec![1, 2, 3],
            }),
        ),
        (
            0x20c,
            [
                9u64.to_le_bytes().to_vec(),
                w(4),
                w(5),
                w(6),
                vec![0x78, 0x9c],
            ]
            .concat(),
            c(Packet::AccountData {
                player: 9,
                kind: 4,
                time: 5,
                decompressed_size: 6,
                compressed: vec![0x78, 0x9c],
            }),
        ),
        (
            0x463,
            [w(4), w(0)].concat(),
            c(Packet::AccountDataSaved {
                kind: 4,
                raw_word: 0,
            }),
        ),
    ]
}

#[test]
fn every_fixture_decodes_and_rejects_bad_extent() {
    let fixtures = fixtures();
    let covered: std::collections::BTreeSet<_> = fixtures.iter().map(|f| f.0).collect();
    assert_eq!(covered, OPCODES.into_iter().collect());
    let mut adapter = crate::wrath_events::WrathEventAdapter::new();
    for (opcode, body, expected) in fixtures {
        // `SessionEvent` has no `PartialEq`; its `Debug` form is total over the fields.
        assert_eq!(
            format!("{:?}", decode(opcode, &body).unwrap()),
            format!("{expected:?}"),
            "{opcode:#x}"
        );
        // The account-data blob runs to the end, so only its header can be short.
        let floor = if opcode == 0x20c { 20 } else { body.len() };
        for end in 0..floor {
            assert!(
                decode(opcode, &body[..end]).is_err(),
                "{opcode:#x} truncated at {end}"
            );
        }
        if opcode != 0x20c {
            assert!(
                decode(opcode, &[body.as_slice(), &[0]].concat()).is_err(),
                "{opcode:#x} trailing"
            );
        }
        match adapter.dispatch(opcode, &body, 0).unwrap() {
            crate::wrath_events::WrathDispatch::Handled { session_events, .. } => {
                assert_eq!(
                    format!("{session_events:?}"),
                    format!("{:?}", vec![expected])
                )
            }
            other => panic!("{opcode:#x}: {other:?}"),
        }
    }
    assert!(
        decode(0x209, &[w(0), vec![1], w(0x100)].concat()).is_err(),
        "only eight data types"
    );
}

#[test]
fn commands_encode_ac_read_order() {
    for (command, opcode, body) in [
        (Command::TutorialFlag { id: 37 }, 0x0fe, w(37)),
        (Command::TutorialClear, 0x0ff, vec![]),
        (Command::TutorialReset, 0x100, vec![]),
        (Command::PlayedTime { trigger: 1 }, 0x1cc, vec![1]),
        (Command::NextCinematicCamera, 0x0fb, vec![]),
        (Command::CompleteCinematic, 0x0fc, vec![]),
        (Command::AreaTrigger { trigger_id: 45 }, 0x0b4, w(45)),
        (Command::ZoneUpdate { zone: 12 }, 0x1f4, w(12)),
        (
            Command::ActionBarToggles { toggles: 0x0f },
            0x2bf,
            vec![0x0f],
        ),
        (Command::RequestAccountData { kind: 4 }, 0x20a, w(4)),
        (
            Command::UpdateAccountData {
                kind: 4,
                time: 5,
                decompressed_size: 6,
                compressed: vec![0x78, 0x9c],
            },
            0x20b,
            [w(4), w(5), w(6), vec![0x78, 0x9c]].concat(),
        ),
        (Command::ReadyForAccountDataTimes, 0x4ff, vec![]),
        (
            Command::SetTitle { title: 42 },
            0x374,
            42i32.to_le_bytes().to_vec(),
        ),
        (Command::SetTitle { title: -1 }, 0x374, vec![0xff; 4]),
    ] {
        assert_eq!(encode(&command), (opcode, body), "{command:?}");
    }
}

#[test]
fn weather_state_maps_to_the_shared_kinds() {
    // AzerothCore `WeatherState` -> 1.12 type: rain 3..5, snow 6..8, sand 22/41/42, the rest clear.
    for (state, kind) in [
        (0, 0),
        (1, 0),
        (3, 1),
        (5, 1),
        (6, 2),
        (8, 2),
        (22, 3),
        (42, 3),
        (86, 1),
        (90, 1),
        (106, 2),
        (2, 0),
    ] {
        assert_eq!(weather_kind(state), kind, "state {state}");
    }
}
