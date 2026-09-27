//! MIDI input ports (`midir`): every port is connected; short messages go to the engine
//! (live MIDI) and, tagged with their port id, to the controller ([`ControlQueue`], drained by
//! `EngineBridge::poll_midi_input` for MIDI learn / mappings).

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use ether_core::protocol::midi_map::MidiInputEvent;
use ether_core::protocol::recording::MidiPort;
use midir::{Ignore, MidiInput, MidiInputConnection};

const CLIENT: &str = "Ethereal";

/// Most messages kept for the controller between two ticks; older ones are dropped first
/// (a controller that stops ticking must not grow memory).
pub(super) const CONTROL_QUEUE_CAPACITY: usize = 1024;

/// Incoming messages for the controller (MIDI learn / mappings). Written by midir callback
/// threads (non-RT), drained by the controller thread; the lock is held for one push/drain.
#[derive(Debug, Default)]
pub(super) struct ControlQueue {
    events: Mutex<VecDeque<MidiInputEvent>>,
}

impl ControlQueue {
    pub fn push(&self, port: &str, data: [u8; 3]) {
        let time_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs_f64() * 1000.0)
            .unwrap_or(0.0);
        let mut q = self.events.lock().unwrap_or_else(|p| p.into_inner());
        if q.len() >= CONTROL_QUEUE_CAPACITY {
            q.pop_front();
        }
        q.push_back(MidiInputEvent {
            port: port.to_string(),
            data,
            time_ms,
        });
    }

    pub fn drain(&self, out: &mut Vec<MidiInputEvent>) {
        let mut q = self.events.lock().unwrap_or_else(|p| p.into_inner());
        out.extend(q.drain(..));
    }
}

/// Connected MIDI input ports (by name).
#[derive(Default)]
pub(super) struct MidiInputs {
    connections: Vec<(String, MidiInputConnection<()>)>,
}

impl MidiInputs {
    /// Connect ports that appeared since the last call (and forget vanished ones); returns
    /// every available port and the connections of vanished ports (to close). `on_message`
    /// receives the port id (`MidiPort::id`) and short messages (status + up to 2 data bytes)
    /// on midir's thread.
    pub fn refresh(
        &mut self,
        on_message: impl Fn(&str, [u8; 3]) + Send + Sync + 'static,
    ) -> (Vec<MidiPort>, Vec<MidiInputConnection<()>>) {
        let Ok(probe) = MidiInput::new(CLIENT) else {
            return (Vec::new(), Vec::new());
        };
        let ports: Vec<(String, midir::MidiInputPort)> = probe
            .ports()
            .into_iter()
            .filter_map(|p| Some((probe.port_name(&p).ok()?, p)))
            .collect();
        // Vanished ports: returned to the caller, which closes them outside any lock.
        let (keep, gone): (Vec<_>, Vec<_>) = std::mem::take(&mut self.connections)
            .into_iter()
            .partition(|(name, _)| ports.iter().any(|(n, _)| n == name));
        self.connections = keep;
        let gone = gone.into_iter().map(|(_, c)| c).collect();
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
            let id = name.clone();
            match input.connect(
                port,
                "ethereal-in",
                move |_stamp, bytes, _| {
                    if let Some(msg) = short_message(bytes) {
                        cb(&id, msg);
                    }
                },
                (),
            ) {
                Ok(conn) => self.connections.push((name.clone(), conn)),
                Err(e) => tracing::warn!(port = %name, error = %e, "could not open MIDI input"),
            }
        }
        let ports = ports
            .into_iter()
            .map(|(name, _)| MidiPort {
                id: name.clone(),
                name,
            })
            .collect();
        (ports, gone)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_queue_is_bounded_and_drains_in_order() {
        let q = ControlQueue::default();
        for i in 0..CONTROL_QUEUE_CAPACITY + 10 {
            q.push("Port A", [0xb0, 1, (i % 128) as u8]);
        }
        let mut out = Vec::new();
        q.drain(&mut out);
        assert_eq!(out.len(), CONTROL_QUEUE_CAPACITY);
        // The oldest messages were dropped.
        assert_eq!(out[0].data, [0xb0, 1, 10]);
        assert_eq!(out[0].port, "Port A");
        assert!(out[0].time_ms > 0.0);
        let mut again = Vec::new();
        q.drain(&mut again);
        assert!(again.is_empty());
    }
}
