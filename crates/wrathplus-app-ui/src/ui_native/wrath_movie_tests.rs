use super::*;

fn app() -> (App, crossbeam_channel::Receiver<ClientCommand>) {
    let mut app = App::new();
    let (tx, rx) = crossbeam_channel::unbounded();
    app.insert_resource(NetCommands(tx))
        .init_resource::<NativeWorldObjectsPackets>()
        .init_resource::<PendingMovie>()
        .insert_resource(Movies(
            std::collections::HashMap::from([(
                2,
                wrathplus_formats::Movie {
                    file: "Interface\\Cinematics\\WOW_Intro.avi".into(),
                    volume: 100,
                },
            )])
            .into(),
        ))
        .add_systems(Update, feed_movie);
    let script = UiScript::new().unwrap();
    script
        .run(
            r#"
            played = {}
            local f = CreateFrame("Frame")
            f:RegisterEvent("PLAY_MOVIE")
            f:SetScript("OnEvent", function(_, _, name, volume)
                played[#played + 1] = name .. "@" .. volume
                -- The stock MovieFrame_PlayMovie: the movie cannot start, so it finishes.
                GameMovieFinished()
            end)
        "#,
        )
        .unwrap();
    app.world_mut().insert_non_send_resource(script);
    (app, rx)
}

fn trigger(app: &mut App, movie: u32) {
    app.world_mut()
        .resource_mut::<NativeWorldObjectsPackets>()
        .0
        .push_back(Packet::TriggerMovie { movie });
}

fn completes(rx: &crossbeam_channel::Receiver<ClientCommand>) -> usize {
    rx.try_iter()
        .filter(|c| matches!(c, ClientCommand::NativeWorldObjects(Command::CompleteMovie)))
        .count()
}

#[test]
fn a_known_movie_plays_and_its_finish_completes_it_once() {
    let (mut app, rx) = app();
    trigger(&mut app, 2);
    app.update();
    let played: Vec<String> = app
        .world()
        .non_send_resource::<UiScript>()
        .eval("return played")
        .unwrap();
    assert_eq!(played, ["Interface\\Cinematics\\WOW_Intro@100"]);
    assert_eq!(completes(&rx), 0, "the finish is read on the next feed");
    app.update();
    assert_eq!(completes(&rx), 1);
    assert!(app.world().resource::<PendingMovie>().id.is_none());
    app.update();
    assert_eq!(completes(&rx), 0, "nothing pending, nothing sent");
}

#[test]
fn an_unknown_movie_finishes_without_playing() {
    let (mut app, rx) = app();
    trigger(&mut app, 99);
    app.update();
    let played: Vec<String> = app
        .world()
        .non_send_resource::<UiScript>()
        .eval("return played")
        .unwrap();
    assert!(played.is_empty());
    assert_eq!(completes(&rx), 1);
}

/// The Wrathgate (14) and the Fall of the Lich King (16) on the install's own tables and stock
/// `MovieFrame.lua`. Their `Movie.dbc` rows carry no file name: build 12340 reaches the files
/// through `MovieVariation.dbc` and `FileData.dbc` (`WOW_WRATHGATE_*.AVI`, `WOW_FOTLK_*.AVI`, at
/// 800 and 1024), which ship loose under `Data/<locale>/Interface/Cinematics` in Blizzard's own
/// `digm` video codec. Labelled: WrathPlus has no decoder, so `StartMovie` answers nil and the stock
/// frame takes its fallback, hiding and calling `GameMovieFinished`; each movie ends with one
/// `CMSG_COMPLETE_MOVIE`, as the reference does for a movie it cannot open.
#[test]
fn the_wrath_movies_take_the_stock_fallback_and_complete_once() {
    let data = wrathplus_formats::wow_data_or_skip!();
    let mut chain = wrathplus_formats::open_chain(&data).expect("open chain");
    let movies = wrathplus_formats::load_movies(&mut chain).expect("Movie.dbc");
    for id in [14, 16] {
        assert_eq!(
            movies.get(id).map(|m| m.file.as_str()),
            Some(""),
            "movie {id}"
        );
    }
    let src = chain
        .read_file("Interface\\FrameXML\\MovieFrame.lua")
        .expect("MovieFrame.lua in the chain");

    let mut app = App::new();
    let (tx, rx) = crossbeam_channel::unbounded();
    app.insert_resource(NetCommands(tx))
        .init_resource::<NativeWorldObjectsPackets>()
        .init_resource::<PendingMovie>()
        .insert_resource(Movies(movies))
        .add_systems(Update, feed_movie);
    let script = UiScript::new().unwrap();
    script.run(&String::from_utf8_lossy(&src)).unwrap();
    script
        .run(
            r#"
            local f = CreateFrame("MovieFrame", "MovieFrame")
            f:Hide()
            f:SetScript("OnEvent", MovieFrame_OnEvent)
            MovieFrame_OnLoad(f)
        "#,
        )
        .unwrap();
    app.world_mut().insert_non_send_resource(script);

    for id in [14, 16] {
        trigger(&mut app, id);
        app.update();
        app.update();
        assert_eq!(completes(&rx), 1, "movie {id} completes once");
        let script = app.world().non_send_resource::<UiScript>();
        assert!(
            !script.eval::<bool>("return MovieFrame:IsShown()").unwrap(),
            "movie {id}: the stock fallback hides the frame"
        );
        assert!(script.errors().is_empty(), "{:?}", script.errors());
        assert!(app.world().resource::<PendingMovie>().id.is_none());
    }
}
