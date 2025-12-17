/// MIDI message type for each input
#[derive(Clone, Copy)]
pub enum MidiMessageType {
    /// Control Change (CC) message
    ControlChange { cc_number: u8 },
    /// Note On/Off message
    #[allow(dead_code)]
    Note { note_number: u8, velocity: u8 },
}

/// Configuration for a single input (button or encoder action)
#[derive(Clone, Copy)]
pub struct MidiInputConfig {
    pub message_type: MidiMessageType,
    pub channel: u8, // MIDI channel (0-15)
}

/// Complete layout configuration for all inputs
pub struct MidiLayout {
    pub encoder_left: MidiInputConfig,
    pub encoder_right: MidiInputConfig,
    pub encoder_button: MidiInputConfig,
    pub key1: MidiInputConfig,
    pub key2: MidiInputConfig,
    pub key3: MidiInputConfig,
}

impl MidiInputConfig {
    /// Create a Control Change configuration
    pub const fn cc(channel: u8, cc_number: u8) -> Self {
        Self {
            message_type: MidiMessageType::ControlChange { cc_number },
            channel,
        }
    }

    /// Create a Note configuration
    #[allow(dead_code)]
    pub const fn note(channel: u8, note_number: u8, velocity: u8) -> Self {
        Self {
            message_type: MidiMessageType::Note {
                note_number,
                velocity,
            },
            channel,
        }
    }
}
