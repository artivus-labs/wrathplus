use std::collections::HashMap;

use crate::framexml;
use crate::loader;
use crate::script::{ScriptValue, SeededCvar, UiScript, UnitGuids, UnitState};

#[test]
fn wrath_keeps_native_lua51_select_length_and_table_semantics() {
    let s = UiScript::new().unwrap();
    assert_eq!(s.eval::<i64>("return select('#', 1, nil, 3)").unwrap(), 3);
    assert_eq!(s.eval::<i64>("return #{10,20,30}").unwrap(), 3);
    assert!(s
        .eval::<bool>("return table.maxn ~= nil and not pcall(table.setn, {}, 10)")
        .unwrap());
}

fn seeded_cvar(name: &str, value: &str) -> SeededCvar {
    SeededCvar {
        name: name.into(),
        value: value.into(),
        default: "host-default-not-a-wrath-claim".into(),
        latched: false,
    }
}

#[test]
fn a_seeded_cvar_reads_back_and_a_host_write_records_no_change() {
    let mut s = UiScript::new().unwrap();
    s.seed_cvars([seeded_cvar("realmName", "Seeded Realm")]);

    assert_eq!(
        s.eval::<(String, String)>("return GetCVar('REALMNAME'), GetCVarDefault('realmName')")
            .unwrap(),
        (
            "Seeded Realm".into(),
            "host-default-not-a-wrath-claim".into()
        )
    );
    assert!(s
        .eval::<bool>("return SetCVar ~= nil and RegisterCVar ~= nil and ConsoleExec ~= nil")
        .unwrap());
    s.set_cvar_host("realmName", "Later Realm");
    assert_eq!(
        s.eval::<String>("return GetCVar('realmName')").unwrap(),
        "Later Realm"
    );
    assert!(s.take_cvar_changes().is_empty());
}

fn observed_unit(exists: bool, guid: u64) -> UnitState {
    UnitState {
        exists,
        guid,
        ..Default::default()
    }
}

/// `0x60c3d0` compares the resolver's guids, held or not (`0x60c1c0`): neither 0 nor the
/// sentinel a recognised token naming nobody leaves.
#[test]
fn wrath_unit_is_unit_compares_the_resolved_guids() {
    let mut s = UiScript::new().unwrap();
    let g = UnitGuids {
        player: 0x10,
        target: 0x10,
        party: [0x20, 0, 0, 0],
        ..Default::default()
    };
    s.set_unit_guids(&g);
    assert_eq!(
        s.eval::<String>(r#"return type(UnitIsUnit("player", "target"))"#)
            .unwrap(),
        "number"
    );
    assert!(s
        .eval::<bool>(r#"return UnitIsUnit("player", "party1") == nil"#)
        .unwrap());
    s.set_unit_guids(&UnitGuids { target: 0, ..g });
    assert!(s
        .eval::<bool>(r#"return UnitIsUnit("target", "party2") == nil"#)
        .unwrap());
    assert_eq!(
        s.eval::<i64>(r#"return UnitIsUnit("player", "PLAYER")"#)
            .unwrap(),
        1,
        "equal text answers before the resolver, case-insensitively"
    );
}

#[test]
fn wrath_unit_is_unit_uses_the_existing_target_chain_resolver() {
    const TARGET: u64 = 0xF130_0000_0000_0001;
    const TARGET_TARGET: u64 = 0x21;

    let mut s = UiScript::new().unwrap();
    s.set_unit_guids(&UnitGuids {
        target: TARGET,
        party: [TARGET_TARGET, 0, 0, 0],
        held: HashMap::from([(TARGET, TARGET_TARGET)]),
        ..Default::default()
    });
    s.set_unit_by_guid(TARGET_TARGET, Some(observed_unit(true, TARGET_TARGET)));

    assert_eq!(
        s.eval::<i64>(r#"return UnitIsUnit("targettarget", "party1")"#)
            .unwrap(),
        1
    );
}

#[test]
fn wrath_setscript_and_xml_handlers_receive_arguments_and_restore_the_legacy_globals() {
    let mut s = UiScript::new().unwrap();
    s.run(
        "this,event,arg1='sentinel-this','sentinel-event','sentinel-arg'; \
         f=CreateFrame('Button','WrathButton'); f:RegisterEvent('E'); \
         f:SetScript('OnEvent', function(self, event, ...) EVENT={self,event,...} end); \
         f:SetScript('OnUpdate', function(self, elapsed) UPDATE={self,elapsed} end); \
         f:SetScript('OnClick', function(self, button, down) CLICK={self,button,down} end)",
    )
    .unwrap();
    s.fire_event("E", vec![ScriptValue::Str("payload".into())]);
    s.tick(0.25);
    s.run("WrathButton:Click('RightButton')").unwrap();
    assert!(s
        .eval::<bool>("return EVENT[1]==f and EVENT[2]=='E' and EVENT[3]=='payload'")
        .unwrap());
    assert!(s
        .eval::<bool>("return UPDATE[1]==f and UPDATE[2]==0.25")
        .unwrap());
    assert!(s
        .eval::<bool>("return CLICK[1]==f and CLICK[2]=='RightButton'")
        .unwrap());
    assert_eq!(
        s.eval::<(String, String, String)>("return this,event,arg1")
            .unwrap(),
        (
            "sentinel-this".into(),
            "sentinel-event".into(),
            "sentinel-arg".into()
        )
    );

    let doc = framexml::parse(
        r#"<Ui><Frame name="XmlWrath"><Scripts><OnLoad>
        XML_SELF=self; XML_THIS=this
        </OnLoad></Scripts></Frame></Ui>"#,
    )
    .unwrap();
    let report = loader::load(&s, &doc, &|_| None);
    assert!(report.errors.is_empty(), "{:?}", report.errors);
    assert!(s
        .eval::<bool>("return XML_SELF==XmlWrath and XML_THIS==XmlWrath and this=='sentinel-this'")
        .unwrap());
}

#[test]
fn wrath_nested_handlers_restore_the_legacy_globals() {
    let mut s = UiScript::new().unwrap();
    s.run(
        "outer=CreateFrame('Frame'); inner=CreateFrame('Frame'); \
         inner:SetScript('OnShow',function(self) INNER=self end); \
         outer:RegisterEvent('E'); outer:SetScript('OnEvent',function(self,event,...) \
           inner:Hide(); inner:Show(); OUTER={self,event,...,this,arg1} end)",
    )
    .unwrap();
    s.fire_event("E", vec![ScriptValue::Int(7)]);
    assert!(s
        .eval::<bool>("return INNER==inner and OUTER[1]==outer and OUTER[2]=='E' and OUTER[3]==7")
        .unwrap());
    assert!(s
        .eval::<bool>("return this==nil and event==nil and arg1==nil")
        .unwrap());
}

/// A 2.x-era handler reads `this`, `event` and `argN` as build 12340 sets them around every
/// dispatch (`0x819ea0`): the frame, the event name on an event, and the values after it.
#[test]
fn wrath_this_style_addon_handlers_read_the_legacy_globals() {
    let mut s = UiScript::new().unwrap();
    s.run(
        r#"
        local f = CreateFrame("Button", "OldAddonFrame")
        f:RegisterEvent("UNIT_HEALTH")
        f:SetScript("OnEvent", function() EV = { this, event, arg1, arg2 } end)
        f:SetScript("OnUpdate", function() UP = { this, arg1 } end)
        f:SetScript("OnClick", function() CL = { this, arg1 } end)
        local inner = CreateFrame("Frame", "OldInner")
        inner:Hide()
        inner:SetScript("OnShow", function() INNER_THIS = this end)
        local outer = CreateFrame("Frame", "OldOuter")
        outer:RegisterEvent("OUTER")
        outer:SetScript("OnEvent", function()
            OldInner:Show()
            AFTER = { this, event, arg1 }
        end)
        "#,
    )
    .unwrap();
    s.fire_event(
        "UNIT_HEALTH",
        vec![ScriptValue::Str("player".into()), ScriptValue::Int(42)],
    );
    s.tick(0.5);
    s.run("OldAddonFrame:Click('LeftButton')").unwrap();
    s.fire_event("OUTER", vec![ScriptValue::Int(9)]);
    assert!(s.errors().is_empty(), "{:?}", s.errors());
    assert!(s
        .eval::<bool>(
            "return EV[1] == OldAddonFrame and EV[2] == 'UNIT_HEALTH' and EV[3] == 'player' \
             and EV[4] == 42"
        )
        .unwrap());
    assert!(s
        .eval::<bool>("return UP[1] == OldAddonFrame and UP[2] == 0.5")
        .unwrap());
    assert!(s
        .eval::<bool>("return CL[1] == OldAddonFrame and CL[2] == 'LeftButton'")
        .unwrap());
    assert!(
        s.eval::<bool>(
            "return INNER_THIS == OldInner and AFTER[1] == OldOuter and AFTER[2] == 'OUTER' \
             and AFTER[3] == 9"
        )
        .unwrap(),
        "the inner dispatch restores the outer handler's globals"
    );
    assert!(s
        .eval::<bool>("return this == nil and event == nil and arg1 == nil")
        .unwrap());
}

#[test]
fn wrath_native_pattern_backtracking_and_recursion_are_bounded() {
    let s = UiScript::new().unwrap();
    for source in [
        "return string.find(string.rep('a', 32), 'a*a*a*a*a*a*a*a*a*a*b')",
        "return string.match(string.rep('a', 32), 'a*a*a*a*a*a*a*a*a*a*b')",
        "return string.gsub(string.rep('a', 32), 'a*a*a*a*a*a*a*a*a*a*b', '')",
        "local it=string.gmatch(string.rep('a', 32), 'a*a*a*a*a*a*a*a*a*a*b'); return it()",
    ] {
        let error = s
            .run(source)
            .expect_err("pathological pattern must stop inside the native matcher");
        assert!(error.to_string().contains("pattern matching"), "{error}");
    }
    assert_eq!(
        s.eval::<String>("return string.match('player-123', '%d+')")
            .unwrap(),
        "123"
    );
}

#[test]
fn wrath_addon_chunks_share_private_table_per_addon() {
    let s = UiScript::new().unwrap();
    s.run_addon_chunk_named(
        "Probe",
        b"local name,p=...; p.value=name",
        "@Probe\\one.lua",
    )
    .unwrap();
    s.run_addon_chunk_named(
        "Probe",
        b"local name,p=...; ADDON={name,p.value}",
        "@Probe\\two.lua",
    )
    .unwrap();
    assert_eq!(
        s.eval::<(String, String)>("return ADDON[1],ADDON[2]")
            .unwrap(),
        ("Probe".into(), "Probe".into())
    );
}

#[test]
fn wrath_pointer_focus_tracks_real_hit_testing_visibility_and_button_state() {
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(800.0, 600.0);
    s.run(
        r#"
        log = {}
        local function record(self, suffix)
            table.insert(log, self:GetName()..suffix)
        end
        local function frame(name, x, y, level)
            local f = CreateFrame("Button", name)
            f:SetPoint("BOTTOMLEFT", x, y)
            f:SetWidth(200); f:SetHeight(200)
            f:SetFrameLevel(level); f:EnableMouse(true)
            f:SetScript("OnEnter", function(self, motion)
                assert(motion == true); record(self, ":enter")
            end)
            f:SetScript("OnLeave", function(self, motion)
                assert(motion == true); record(self, ":leave")
            end)
            return f
        end
        Back = frame("WrathBack", 100, 100, 1)
        Front = frame("WrathFront", 150, 150, 2)
        Disabled = frame("WrathDisabled", 400, 100, 3)
        Disabled:SetWidth(100); Disabled:SetHeight(100); Disabled:Disable()
        "#,
    )
    .unwrap();
    s.resolve();

    s.mouse_move(110.0, 110.0);
    assert!(s.eval::<bool>("return GetMouseFocus() == Back").unwrap());

    s.mouse_move(175.0, 175.0);
    assert!(s.eval::<bool>("return GetMouseFocus() == Front").unwrap());
    s.run("Front:Hide()").unwrap();
    assert!(
        s.eval::<bool>("return GetMouseFocus() == nil").unwrap(),
        "hiding the hovered frame clears focus before the deferred re-pick"
    );

    s.mouse_move(175.0, 175.0);
    assert!(s.eval::<bool>("return GetMouseFocus() == Back").unwrap());
    s.mouse_move(425.0, 125.0);
    assert!(
        s.eval::<bool>("return GetMouseFocus() == Disabled")
            .unwrap(),
        "a disabled button remains the actual hover frame"
    );
    s.mouse_move(700.0, 500.0);
    assert!(s.eval::<bool>("return GetMouseFocus() == nil").unwrap());
    assert_eq!(
        s.eval::<String>("return table.concat(log, ',')").unwrap(),
        "WrathBack:enter,WrathBack:leave,WrathFront:enter,WrathFront:leave,WrathBack:enter,WrathBack:leave",
        "overlap follows frame order; disabled buttons suppress enter and leave handlers"
    );
    assert!(s.errors().is_empty(), "{:?}", s.errors());
}

#[test]
fn wrath_cursor_position_is_screen_space_and_callers_apply_effective_scale() {
    let mut s = UiScript::new().unwrap();
    s.set_screen_size(800.0, 600.0);
    s.run(
        "Scaled = CreateFrame('Frame'); Scaled:SetScale(0.5); \
         function ReadCursorForScaledFrame() \
           local x,y = GetCursorPosition(); local scale = Scaled:GetEffectiveScale(); \
           return x,y,x/scale,y/scale \
         end",
    )
    .unwrap();

    s.mouse_move(240.0, 135.0);
    assert_eq!(
        s.eval::<(f32, f32, f32, f32)>("return ReadCursorForScaledFrame()")
            .unwrap(),
        (240.0, 135.0, 480.0, 270.0),
        "GetCursorPosition stays in screen UI units; frame-local math divides by effective scale"
    );
}
