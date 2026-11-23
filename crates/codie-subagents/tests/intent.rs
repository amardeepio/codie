//! `AGENT-028` and `AGENT-029`: delegation modes and prompt detection.

use codie_subagents::{
    DelegationIntent, DelegationMode, FORCED_NUDGE, NudgeState, detect_intent, effective_mode,
    mode_directive,
};
use codie_testkit::spec;

#[test]
#[spec("AGENT-029")]
fn spec_agent_029_prompt_intent_detection() {
    for prompt in [
        "Please use subagents to make it faster",
        "use subagents",
        "delegate this to a child",
        "run it in parallel",
        "parallelize the task",
        "spawn agents for the audit",
        "split this into agents",
        "use multiple agents",
    ] {
        assert_eq!(
            detect_intent(prompt),
            DelegationIntent::Requested,
            "expected a request: {prompt}"
        );
    }

    for prompt in [
        "don't use subagents",
        "Do not delegate this",
        "no agents, just do it",
        "without subagents",
        "avoid subagents",
    ] {
        assert_eq!(
            detect_intent(prompt),
            DelegationIntent::Suppressed,
            "expected suppression: {prompt}"
        );
    }

    for prompt in [
        "make it faster",
        "refactor the parser",
        "use the agents config file",
        "review src/main.rs",
    ] {
        assert_eq!(detect_intent(prompt), DelegationIntent::None, "{prompt}");
    }

    // Fenced code blocks are ignored.
    assert_eq!(
        detect_intent("```\nuse subagents to make it faster\n```"),
        DelegationIntent::None
    );

    // Only the first 500 characters are inspected.
    let mut late = "x".repeat(600);
    late.push_str(" use subagents");
    assert_eq!(detect_intent(&late), DelegationIntent::None);
}

#[test]
#[spec("AGENT-028")]
fn spec_agent_028_delegation_mode_precedence() {
    // A disabled configuration always wins.
    assert_eq!(
        effective_mode(
            false,
            DelegationMode::Force,
            true,
            DelegationIntent::Requested
        )
        .mode,
        DelegationMode::Off
    );

    // Prompt suppression beats a forced session.
    assert_eq!(
        effective_mode(
            true,
            DelegationMode::Force,
            false,
            DelegationIntent::Suppressed
        )
        .mode,
        DelegationMode::Off
    );

    // An explicit request beats a session that is off.
    assert_eq!(
        effective_mode(
            true,
            DelegationMode::Off,
            false,
            DelegationIntent::Requested
        )
        .mode,
        DelegationMode::Force
    );

    // Session off otherwise stays off.
    assert_eq!(
        effective_mode(true, DelegationMode::Off, false, DelegationIntent::None).mode,
        DelegationMode::Off
    );

    // Sticky force and one-shot force both produce force.
    assert_eq!(
        effective_mode(true, DelegationMode::Force, false, DelegationIntent::None).mode,
        DelegationMode::Force
    );
    assert_eq!(
        effective_mode(true, DelegationMode::Auto, true, DelegationIntent::None).mode,
        DelegationMode::Force
    );

    // Default is auto.
    assert_eq!(
        effective_mode(true, DelegationMode::Auto, false, DelegationIntent::None).mode,
        DelegationMode::Auto
    );

    assert!(mode_directive(DelegationMode::Force).contains("delegation"));
    assert_eq!(DelegationMode::parse("on"), Some(DelegationMode::Auto));
    assert_eq!(DelegationMode::parse("nope"), None);
}

#[test]
#[spec("AGENT-030")]
fn spec_agent_030_forced_nudge_once() {
    let mut state = NudgeState::default();
    assert_eq!(state.after_turn(true, false), Some(FORCED_NUDGE));
    assert_eq!(
        state.after_turn(true, false),
        None,
        "a second turn is not forced"
    );

    state.reset();
    assert_eq!(
        state.after_turn(true, true),
        None,
        "delegation needs no nudge"
    );
    assert_eq!(state.after_turn(false, false), None, "auto is not nudged");
    assert!(FORCED_NUDGE.contains("task"));
}
