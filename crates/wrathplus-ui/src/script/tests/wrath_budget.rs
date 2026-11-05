use crate::script::{UiScript, WRATH_INSTRUCTION_HOOK_STEP, WRATH_MEMORY_LIMIT};

fn wrath() -> UiScript {
    UiScript::new().unwrap()
}

fn assert_budget_error(error: mlua::Error) {
    let message = error.to_string();
    assert!(
        message.contains("instruction budget exhausted"),
        "expected the host budget sentinel, got: {message}"
    );
}

#[test]
fn wrath_budget_is_shared_with_coroutines_created_before_it_is_armed() {
    let s = wrath();
    s.run(
        "co = coroutine.create(function() \
         local n = 0; for i = 1, 1000000 do n = n + i end; return n end)",
    )
    .unwrap();

    s.set_instruction_budget(WRATH_INSTRUCTION_HOOK_STEP as u64 * 2);
    assert_budget_error(
        s.run("local ok, value = coroutine.resume(co); COROUTINE_CONTINUED = true")
            .expect_err("a pre-existing coroutine must inherit the global hook"),
    );
    assert!(
        !s.eval::<bool>("return COROUTINE_CONTINUED == true")
            .unwrap_or(false),
        "the resume wrapper must not continue after reporting the hook failure"
    );
}

#[test]
fn wrath_budget_sentinel_escapes_nested_pcall_xpcall_and_error_handlers() {
    for source in [
        "pcall(function() local n=0; for i=1,1000000 do n=n+i end end); CONTINUED=true",
        "pcall(function() pcall(function() local n=0; for i=1,1000000 do n=n+i end end) end); CONTINUED=true",
        "xpcall(function() local n=0; for i=1,1000000 do n=n+i end end, function(e) HANDLER_RAN=true; return e end); CONTINUED=true",
    ] {
        let s = wrath();
        s.set_instruction_budget(WRATH_INSTRUCTION_HOOK_STEP as u64 * 2);
        assert_budget_error(
            s.run(source)
                .expect_err("protected calls must not swallow the sticky host sentinel"),
        );
        assert!(!s.eval::<bool>("return CONTINUED == true").unwrap_or(false));
    }
}

/// The guards keep the stdlib's own argument errors: `luaL_argerror` names the verb its caller
/// called and `luaL_where(L, 1)` places the message at that caller, not inside a guard.
#[test]
fn the_guarded_verbs_raise_their_own_argument_errors_at_the_caller() {
    let s = wrath();
    for (call, want) in [
        ("pcall()", "bad argument #1 to 'pcall' (value expected)"),
        (
            "xpcall(print)",
            "bad argument #2 to 'xpcall' (value expected)",
        ),
        (
            "coroutine.resume(nil)",
            "bad argument #1 to 'resume' (coroutine expected)",
        ),
    ] {
        let got: String = s
            .eval(&format!("return select(2, pcall(function() {call} end))"))
            .unwrap();
        assert!(
            got.ends_with(&format!(":1: {want}")) && !got.contains("guards"),
            "{call}: {got}"
        );
    }
}

#[test]
fn clearing_a_wrath_budget_keeps_the_global_hook_dormant_and_reusable() {
    let s = wrath();
    s.set_instruction_budget(WRATH_INSTRUCTION_HOOK_STEP as u64);
    assert_budget_error(
        s.run("local n=0; for i=1,1000000 do n=n+i end")
            .expect_err("the first finite workload exceeds the small budget"),
    );

    s.clear_instruction_budget();
    assert_eq!(s.eval::<i64>("return 20 + 22").unwrap(), 42);

    s.set_instruction_budget(WRATH_INSTRUCTION_HOOK_STEP as u64);
    assert_budget_error(
        s.run("local n=0; for i=1,1000000 do n=n+i end")
            .expect_err("the global hook must be reusable after disarming"),
    );
}

#[test]
fn wrath_allocator_rejects_bounded_growth_past_its_ceiling() {
    let s = wrath();
    let error = s
        .run(&format!(
            "local chunks={{}}; for i=1,{} do chunks[i]=string.rep('x',1024*1024)..i end",
            WRATH_MEMORY_LIMIT / (1024 * 1024) + 8
        ))
        .expect_err("bounded allocations beyond the configured ceiling must fail");
    assert!(
        matches!(error, mlua::Error::MemoryError(_)) || error.to_string().contains("memory"),
        "expected allocator refusal, got: {error}"
    );
    assert!(s.lua().used_memory() <= WRATH_MEMORY_LIMIT);
}

#[test]
fn debugstack_hostile_counts_are_clamped_without_overflow() {
    let s = wrath();
    for args in ["1/0, 1/0, 1/0", "1e300, 1e300, 1e300", "-1, -1, -1"] {
        let trace = s
            .eval::<String>(&format!("return debugstack({args})"))
            .unwrap();
        assert!(
            trace.len() < 128 * 1024,
            "diagnostic output must remain bounded"
        );
    }
}
