// SPDX-FileCopyrightText: © 2026 Julian Andrews
// SPDX-License-Identifier: 0BSD

//! Window rules end to end: they are read from the pending window's
//! app_id/title when it is mapped, so the events carrying them have to arrive
//! before the window is added to a workspace.

use super::*;
use crate::types::WindowRule;

fn rule(app_id: Option<&str>, title: Option<&str>, floating: bool) -> WindowRule {
    WindowRule {
        app_id: app_id.map(Into::into),
        title: title.map(Into::into),
        floating,
    }
}

/// One output, one pending window, and the given rules.
fn scene(rules: Vec<WindowRule>) -> (Session, MiniServer) {
    let (mut s, mut sv) = build();
    s.add_seat(&mut sv);
    s.manage(&mut sv);
    s.add_output(&mut sv, "A", (0, 0), (1920, 1080));
    s.manage(&mut sv);
    s.config_mut().window_rules = rules;
    (s, sv)
}

fn send_str(s: &mut Session, sv: &mut MiniServer, win: &ObjectId, opcode: u16, value: &str) {
    s.send(
        sv,
        win.clone(),
        opcode,
        vec![Argument::Str(Some(Box::new(CString::new(value).unwrap())))],
    );
}

/// The single window of the scene, asserting there is exactly one.
fn only_window(s: &Session) -> &crate::types::Window {
    let ws = &s.state.wm.outputs[0].workspace_list[0];
    assert_eq!(ws.window_list.len(), 1, "one window expected");
    &ws.window_list[0]
}

#[test]
fn a_matching_app_id_floats_the_window() {
    let (mut s, mut sv) = scene(vec![rule(Some("foot"), None, true)]);
    let win = s.add_pending_window(&mut sv);
    send_str(&mut s, &mut sv, &win, EVT_WIN_APP_ID, "foot");
    s.map_window(&mut sv, &win);
    s.manage(&mut sv);
    assert!(only_window(&s).geom.is_floating);
}

#[test]
fn a_matching_title_glob_floats_the_window() {
    let (mut s, mut sv) = scene(vec![rule(None, Some("*Picture-in-Picture*"), true)]);
    let win = s.add_pending_window(&mut sv);
    send_str(
        &mut s,
        &mut sv,
        &win,
        EVT_WIN_TITLE,
        "Picture-in-Picture (mpv)",
    );
    s.map_window(&mut sv, &win);
    s.manage(&mut sv);
    assert!(only_window(&s).geom.is_floating);
}

#[test]
fn an_unmatched_app_id_leaves_the_window_tiled() {
    let (mut s, mut sv) = scene(vec![rule(Some("foot"), None, true)]);
    let win = s.add_pending_window(&mut sv);
    send_str(&mut s, &mut sv, &win, EVT_WIN_APP_ID, "something-else");
    s.map_window(&mut sv, &win);
    s.manage(&mut sv);
    assert!(!only_window(&s).geom.is_floating);
}

/// `floating = false` (the default) never floats even on a match, and a rule
/// with no app_id and no title cannot match anything.
#[test]
fn rules_need_floating_and_a_matcher() {
    let (mut s, mut sv) = scene(vec![
        rule(Some("foot"), None, false),
        rule(None, None, true),
    ]);
    let win = s.add_pending_window(&mut sv);
    send_str(&mut s, &mut sv, &win, EVT_WIN_APP_ID, "foot");
    s.map_window(&mut sv, &win);
    s.manage(&mut sv);
    assert!(!only_window(&s).geom.is_floating);
}

/// Rules are evaluated when the window is mapped: an app_id arriving after
/// that does not retroactively float the window. river sends the app_id
/// before the first manage sequence, so in practice rules see it.
#[test]
fn a_late_app_id_does_not_apply_the_rule() {
    let (mut s, mut sv) = scene(vec![rule(Some("foot"), None, true)]);
    let win = s.add_window(&mut sv); // dimensions first: mapped with no app_id
    s.manage(&mut sv);
    send_str(&mut s, &mut sv, &win, EVT_WIN_APP_ID, "foot");
    s.manage(&mut sv);
    assert!(!only_window(&s).geom.is_floating);
}
