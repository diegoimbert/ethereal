//! An extension's document commands (`ext::set_ext_doc_commands`): applied in a transaction,
//! one undo step like the app's own. Its own test binary: the registry is process-wide.

mod common;

use common::*;
use ether_controller::Controller;
use ether_controller::ext::{DocCtx, ExtDocCommands, set_ext_doc_commands};
use ether_core::protocol::ext::ExtCommand;
use ether_core::protocol::model::SettingsChange;
use ether_core::protocol::project::EditCommand;
use ether_core::protocol::{Command, ReplyValue};

fn is_metronome(c: &Command) -> bool {
    matches!(c, Command::Ext(e) if e.domain == "Metronome")
}

fn apply(ctx: &mut DocCtx, c: &Command) -> Option<ether_controller::ext::api::CmdResult<()>> {
    is_metronome(c).then(|| ctx.tx.settings(SettingsChange::Metronome(true)))
}

#[test]
fn an_extension_document_command_is_one_undo_step() {
    set_ext_doc_commands(ExtDocCommands {
        is_document: is_metronome,
        apply,
    });
    let mut h = Harness::with_project();
    let before = h.ctl.project().unwrap().settings.metronome;
    let cmd = Command::Ext(ExtCommand {
        domain: "Metronome".into(),
        command: serde_json::json!({ "type": "On" }),
    });
    assert_eq!(ok(&h.send(cmd)), ReplyValue::Unit);
    assert!(h.ctl.project().unwrap().settings.metronome);
    ok(&h.send(Command::Edit(EditCommand::Undo)));
    assert_eq!(h.ctl.project().unwrap().settings.metronome, before);
}
