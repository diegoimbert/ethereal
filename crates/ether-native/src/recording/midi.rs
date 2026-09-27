//! MIDI input ports (`midir`): every port is connected; short messages go to the engine.

use std::sync::Arc;

use ether_core::protocol::recording::MidiPort;
use midir::{Ignore, MidiInput, MidiInputConnection};

const CLIENT: &str = "Ethereal";

/// Connected MIDI input ports (by name).
#[derive(Default)]
pub(super) struct MidiInputs {
    connections: Vec<(String, MidiInputConnection<()>)>,
}

impl MidiInputs {
    /// Connect ports that appeared since the last call (and forget vanished ones); returns
    /// every available port. `on_message` receives short messages (status + up to 2 data
    /// bytes) on midir's thread.
    pub fn refresh(
        &mut self,
        on_message: impl Fn([u8; 3]) + Send + Sync + 'static,
    ) -> Vec<MidiPort> {
        let Ok(probe) = MidiInput::new(CLIENT) else {
            return Vec::new();
        };
        let ports: Vec<(String, midir::MidiInputPort)> = probe
            .ports()
            .into_iter()
            .filter_map(|p| Some((probe.port_name(&p).ok()?, p)))
            .collect();
        self.connections
            .retain(|(name, _)| ports.iter().any(|(n, _)| n == name));
        let on_message = Arc::new(on_message);
        for (name, port) in &ports {
            if self.connections.iter().any(|(n, _)| n == name) {
                continue;
            }
            let Ok(mut input) = MidiInput::new(CLIENT) else {
                continue;
            };
            input.ignore(Ignore::All);
            let cb = on_message.clone();
            match input.connect(
                port,
                "ethereal-in",
                move |_stamp, bytes, _| {
                    if let Some(msg) = short_message(bytes) {
                        cb(msg);
                    }
                },
                (),
            ) {
                Ok(conn) => self.connections.push((name.clone(), conn)),
                Err(e) => tracing::warn!(port = %name, error = %e, "could not open MIDI input"),
            }
        }
        ports
            .into_iter()
            .map(|(name, _)| MidiPort {
                id: name.clone(),
                name,
            })
            .collect()
    }
}

/// A channel voice message as 3 bytes (`None` for system/realtime/SysEx data).
pub(super) fn short_message(bytes: &[u8]) -> Option<[u8; 3]> {
    let status = *bytes.first()?;
    if !(0x80..0xf0).contains(&status) {
        return None;
    }
    Some([
        status,
        bytes.get(1).copied().unwrap_or(0),
        bytes.get(2).copied().unwrap_or(0),
    ])
}
