use camino::Utf8Path;

use crate::fixture::{live_hall, unguard, web_instructions};
use crate::live::{BASH_PROMPT, DEEP_PROMPT, Live, READ_PROMPT_1, READ_PROMPT_2, REPS};
use crate::session::{
    answer, delivered, events, finish, launch, one_shot, opened_instruction_file, send, session,
    stop, turn,
};
use crate::support::run_ok;

/// `ivar feature promote` in the middle of a live session, then the feature edits
/// its instruction files in the new worktree.
pub(crate) fn promote_and_edit(root: &Utf8Path, feature: &str, view: &Utf8Path) {
    run_ok(root, &["feature", "promote", feature, "web"]);
    let link = std::fs::read_link(view.join("web")).unwrap();
    assert!(link.ends_with(feature), "view not repointed: {link:?}");
    let feature_text = web_instructions("pnpm serve:bravo", "FEAT-92");
    let worktree = root.join(".ivar/repos/web").join(feature);
    for name in ["CLAUDE.md", "AGENTS.md"] {
        std::fs::write(worktree.join(name), &feature_text).unwrap();
    }
}

pub(crate) fn promotion_mid_session(live: Live) {
    if !live.enabled() {
        return;
    }
    let (_guard, root) = live_hall();
    let mut passed = 0;
    for rep in 1..=REPS {
        let feature = format!("h4-{}-{rep}", live.key());
        let (id, view) = session(&root, &feature, live);
        let mut child = launch(&root, &id, live, &live.multi_turn_args(&view));
        let mut stdin = child.stdin.take().unwrap();
        let receiver = events(&mut child);

        send(&mut stdin, live, 1, READ_PROMPT_1);
        let turn1 = turn(&receiver, live);
        assert!(
            delivered(&view, live),
            "{live:?}: the guard hook never delivered; is the hall's hook reaching the session?"
        );
        promote_and_edit(&root, &feature, &view);
        send(&mut stdin, live, 2, READ_PROMPT_2);
        let turn2 = turn(&receiver, live);
        drop(stdin);
        finish(child);
        stop(&root, &id);

        let (first, second) = (answer(live, &turn1), answer(live, &turn2));
        let ok = first.contains("serve:alpha")
            && second.contains("serve:bravo")
            && !second.contains("serve:alpha");
        eprintln!(
            "{live:?} promotion rep{rep} {} opened_instr={} t1={first:?} t2={second:?}",
            if ok { "PASS" } else { "FAIL" },
            opened_instruction_file(live, &[turn1, turn2].concat()),
        );
        passed += usize::from(ok);
    }
    unguard(&root);
    assert_eq!(passed, REPS, "{live:?}: promotion gate {passed}/{REPS}");
}

pub(crate) fn bash_cat(live: Live) {
    if !live.enabled() {
        return;
    }
    let (_guard, root) = live_hall();
    let mut passed = 0;
    for rep in 1..=REPS {
        let feature = format!("b1-{}-{rep}", live.key());
        let (events, delivered) = one_shot(&root, live, &feature, BASH_PROMPT, live.bash_tools());
        let reply = answer(live, &events);
        let ok = delivered && reply.contains("probe:lima");
        eprintln!(
            "{live:?} bash rep{rep} {} delivered={delivered} opened_instr={} answer={reply:?}",
            if ok { "PASS" } else { "FAIL" },
            opened_instruction_file(live, &events),
        );
        passed += usize::from(ok);
    }
    unguard(&root);
    assert_eq!(passed, REPS, "{live:?}: Bash cat gate {passed}/{REPS}");
}

pub(crate) fn deep_file(live: Live) {
    if !live.enabled() {
        return;
    }
    let (_guard, root) = live_hall();
    let mut passed = 0;
    for rep in 1..=REPS {
        let feature = format!("deep-{}-{rep}", live.key());
        let (events, delivered) = one_shot(&root, live, &feature, DEEP_PROMPT, live.read_tools());
        let reply = answer(live, &events);
        let opened = opened_instruction_file(live, &events);
        let ok = delivered && reply.contains("serve:zulu") && !opened;
        eprintln!(
            "{live:?} deep rep{rep} {} delivered={delivered} opened_instr={opened} answer={reply:?}",
            if ok { "PASS" } else { "FAIL" },
        );
        passed += usize::from(ok);
    }
    unguard(&root);
    assert_eq!(passed, REPS, "{live:?}: deep gate {passed}/{REPS}");
}
