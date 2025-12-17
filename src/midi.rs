use crate::layouts::{MidiInputConfig, MidiLayout, MidiMessageType};
use crate::{ButtonResources, EncoderResources};
use defmt::unreachable;
use defmt_rtt as _;
use embassy_executor::{InterruptExecutor, Spawner};
use embassy_futures::select::select_array;
use embassy_rp::gpio::{Input, Level, Pull};
use embassy_rp::interrupt;
use embassy_rp::interrupt::{InterruptExt, Priority};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::pubsub::PubSubChannel;
use embassy_time::{Duration, Timer};
use embassy_usb::class::midi::{MidiClass, Sender};

static KEY_EVENT_QUEUE: PubSubChannel<CriticalSectionRawMutex, KeyEvent, 8, 2, 2> =
    PubSubChannel::new();

// Encoder value counters (0-127) for absolute mode - one per mode
// [Mode1/Keyboard, Mode2/Picoprog, Mode3/Universal]
static ENCODER_VALUES: Mutex<CriticalSectionRawMutex, [u8; 3]> = Mutex::new([64, 64, 64]); // Start at middle (64)

#[derive(Clone, PartialEq, Eq, Hash)]
enum Key {
    EncoderLeft,
    EncoderRight,
    EncoderButton,
    Key1,
    Key2,
    Key3,
}

#[derive(Clone, Copy, PartialEq)]
enum Event {
    Pressed,
    Released,
}

#[derive(Clone)]
struct KeyEvent {
    key: Key,
    event: Event,
}

/// MIDI Layout 1 - Position 1 (Keyboard mode selector - Teal LED)
/// Channel 15, Notes for keys (C1, C#1, D1), free CCs for encoder
const MIDI_LAYOUT_1: MidiLayout = MidiLayout {
    encoder_left: MidiInputConfig::cc(14, 102), // CC 102 (undefined/free)
    encoder_right: MidiInputConfig::cc(14, 102), // CC 102 (undefined/free)
    encoder_button: MidiInputConfig::cc(14, 103), // CC 103 (undefined/free)
    key1: MidiInputConfig::note(14, 36, 127),   // Note C1 (MIDI note 36)
    key2: MidiInputConfig::note(14, 37, 127),   // Note C#1 (MIDI note 37)
    key3: MidiInputConfig::note(14, 38, 127),   // Note D1 (MIDI note 38)
};

/// MIDI Layout 2 - Position 2 (Picoprog mode selector - Orange LED)
/// Channel 15, First set of CC values
const MIDI_LAYOUT_2: MidiLayout = MidiLayout {
    encoder_left: MidiInputConfig::cc(14, 104), // CC 104 (undefined/free)
    encoder_right: MidiInputConfig::cc(14, 104), // CC 104 (undefined/free)
    encoder_button: MidiInputConfig::cc(14, 105), // CC 105 (undefined/free)
    key1: MidiInputConfig::cc(14, 20),          // CC 20 (General Purpose 1)
    key2: MidiInputConfig::cc(14, 21),          // CC 21 (General Purpose 2)
    key3: MidiInputConfig::cc(14, 22),          // CC 22 (General Purpose 3)
};

/// MIDI Layout 3 - Position 3 (Universal/neutral mode selector - Pink LED)
/// Channel 15, Second set of CC values
const MIDI_LAYOUT_3: MidiLayout = MidiLayout {
    encoder_left: MidiInputConfig::cc(14, 106), // CC 106 (undefined/free)
    encoder_right: MidiInputConfig::cc(14, 106), // CC 106 (undefined/free)
    encoder_button: MidiInputConfig::cc(14, 107), // CC 107 (undefined/free)
    key1: MidiInputConfig::cc(14, 23),          // CC 23 (General Purpose 4)
    key2: MidiInputConfig::cc(14, 24),          // CC 24 (General Purpose 5)
    key3: MidiInputConfig::cc(14, 25),          // CC 25 (General Purpose 6)
};

/// Encode a MIDI message into a USB-MIDI packet (4 bytes)
///
/// USB-MIDI packet format:
/// - Byte 0: Cable Number (4 bits) + Code Index Number (4 bits)
/// - Byte 1-3: MIDI message bytes
///
/// For our single virtual cable: Cable Number = 0
fn encode_midi_packet(config: &MidiInputConfig, value: u8) -> [u8; 4] {
    match config.message_type {
        MidiMessageType::ControlChange { cc_number } => {
            // CIN 0x0B = Control Change (3-byte message)
            // Status byte: 0xB0 + channel
            [
                0x0B,                  // CIN for Control Change
                0xB0 | config.channel, // Status: Control Change + channel
                cc_number,             // CC number
                value,                 // CC value
            ]
        }
        MidiMessageType::Note {
            note_number,
            velocity,
        } => {
            if value > 0 {
                // Note On: CIN 0x09
                // Status byte: 0x90 + channel
                [
                    0x09,                  // CIN for Note On
                    0x90 | config.channel, // Status: Note On + channel
                    note_number,           // Note number
                    velocity,              // Velocity
                ]
            } else {
                // Note Off: CIN 0x08
                // Status byte: 0x80 + channel
                [
                    0x08,                  // CIN for Note Off
                    0x80 | config.channel, // Status: Note Off + channel
                    note_number,           // Note number
                    0x00,                  // Velocity (0)
                ]
            }
        }
    }
}

// Mode monitoring task that continuously checks for mode switch changes
#[embassy_executor::task]
async fn mode_monitor_task(
    selector_keyboard: Input<'static>,
    selector_picoprog: Input<'static>,
) -> ! {
    let get_current_mode = || -> crate::DeviceMode {
        use embassy_rp::gpio::Level;
        if selector_keyboard.get_level() == Level::Low {
            crate::DeviceMode::Keyboard
        } else if selector_picoprog.get_level() == Level::Low {
            crate::DeviceMode::Picoprog
        } else {
            crate::DeviceMode::Universal
        }
    };

    let mut last_mode = get_current_mode();

    loop {
        Timer::after(Duration::from_millis(50)).await; // Check every 50ms

        let current_mode = get_current_mode();
        if current_mode != last_mode {
            // Update the shared mode
            {
                let mut mode = crate::CURRENT_MODE.lock().await;
                *mode = current_mode;
            }
            // Signal that mode changed
            crate::MODE_CHANGED.signal(());
            last_mode = current_mode;
        }
    }
}

#[embassy_executor::task]
pub async fn midi_task(
    spawner: Spawner,
    midi_class: MidiClass<'static, embassy_rp::usb::Driver<'static, embassy_rp::peripherals::USB>>,
    button_resources: ButtonResources,
    encoder_resources: EncoderResources,
    _initial_mode: crate::DeviceMode,
    selector_keyboard: Input<'static>,
    selector_picoprog: Input<'static>,
) -> ! {
    // Spawn mode monitor task to continuously check for mode changes
    spawner
        .spawn(mode_monitor_task(selector_keyboard, selector_picoprog))
        .unwrap();

    // Split MIDI class into sender and receiver
    let (mut sender, _) = midi_class.split();

    interrupt::SWI_IRQ_0.set_priority(Priority::P2);
    let spawner_encoder: embassy_executor::SendSpawner =
        EXECUTOR_ENCODER.start(interrupt::SWI_IRQ_0);
    spawner_encoder
        .spawn(encoder_task(encoder_resources))
        .unwrap();

    spawner.spawn(button_task(button_resources)).unwrap();

    let mut sub = KEY_EVENT_QUEUE.subscriber().unwrap();

    // Track which layout each key was pressed with to ensure matching release
    let mut pressed_configs: heapless::FnvIndexMap<Key, MidiInputConfig, 4> =
        heapless::FnvIndexMap::new();

    loop {
        let key_event: KeyEvent = sub.next_message_pure().await;

        // Read current mode from shared mutex
        let current_mode = {
            let mode = crate::CURRENT_MODE.lock().await;
            *mode
        };

        let layout = match current_mode {
            crate::DeviceMode::Keyboard => &MIDI_LAYOUT_1,
            crate::DeviceMode::Picoprog => &MIDI_LAYOUT_2,
            crate::DeviceMode::Universal => &MIDI_LAYOUT_3,
        };

        match key_event.key {
            Key::EncoderLeft => {
                sender =
                    handle_encoder_interaction(sender, &layout.encoder_left, false, current_mode)
                        .await;
            }
            Key::EncoderRight => {
                sender =
                    handle_encoder_interaction(sender, &layout.encoder_right, true, current_mode)
                        .await;
            }
            Key::EncoderButton => {
                let config = match key_event.event {
                    Event::Pressed => {
                        // Store the config for this press
                        let _ =
                            pressed_configs.insert(key_event.key.clone(), layout.encoder_button);
                        &layout.encoder_button
                    }
                    Event::Released => {
                        // Use the stored config from when it was pressed
                        pressed_configs
                            .get(&key_event.key)
                            .unwrap_or(&layout.encoder_button)
                    }
                };
                sender = send_midi_message(sender, config, key_event.event).await;
                if key_event.event == Event::Released {
                    pressed_configs.remove(&key_event.key);
                }
            }
            Key::Key1 => {
                let config = match key_event.event {
                    Event::Pressed => {
                        let _ = pressed_configs.insert(key_event.key.clone(), layout.key1);
                        &layout.key1
                    }
                    Event::Released => pressed_configs.get(&key_event.key).unwrap_or(&layout.key1),
                };
                sender = send_midi_message(sender, config, key_event.event).await;
                if key_event.event == Event::Released {
                    pressed_configs.remove(&key_event.key);
                }
            }
            Key::Key2 => {
                let config = match key_event.event {
                    Event::Pressed => {
                        let _ = pressed_configs.insert(key_event.key.clone(), layout.key2);
                        &layout.key2
                    }
                    Event::Released => pressed_configs.get(&key_event.key).unwrap_or(&layout.key2),
                };
                sender = send_midi_message(sender, config, key_event.event).await;
                if key_event.event == Event::Released {
                    pressed_configs.remove(&key_event.key);
                }
            }
            Key::Key3 => {
                let config = match key_event.event {
                    Event::Pressed => {
                        let _ = pressed_configs.insert(key_event.key.clone(), layout.key3);
                        &layout.key3
                    }
                    Event::Released => pressed_configs.get(&key_event.key).unwrap_or(&layout.key3),
                };
                sender = send_midi_message(sender, config, key_event.event).await;
                if key_event.event == Event::Released {
                    pressed_configs.remove(&key_event.key);
                }
            }
        }
    }
}

static EXECUTOR_ENCODER: InterruptExecutor = InterruptExecutor::new();

#[interrupt]
unsafe fn SWI_IRQ_0() {
    unsafe { EXECUTOR_ENCODER.on_interrupt() }
}

#[embassy_executor::task]
pub async fn encoder_task(r: EncoderResources) -> ! {
    let encoder_left: Input<'_> = Input::new(r.encoder_left, Pull::None);
    let mut encoder_right: Input<'_> = Input::new(r.encoder_right, Pull::None);

    let publisher = KEY_EVENT_QUEUE.publisher().unwrap();

    loop {
        encoder_right.wait_for_falling_edge().await;

        if encoder_left.get_level() == Level::Low {
            publisher.publish_immediate(KeyEvent {
                key: Key::EncoderLeft,
                event: Event::Pressed,
            });
        } else {
            publisher.publish_immediate(KeyEvent {
                key: Key::EncoderRight,
                event: Event::Pressed,
            });
        };

        encoder_right.wait_for_rising_edge().await;
    }
}

#[embassy_executor::task]
pub async fn button_task(r: ButtonResources) -> ! {
    let mut key1: Input<'_> = Input::new(r.key1, Pull::Up);
    key1.set_schmitt(true);

    let mut key2: Input<'_> = Input::new(r.key2, Pull::Up);
    key2.set_schmitt(true);

    let mut key3: Input<'_> = Input::new(r.key3, Pull::Up);
    key3.set_schmitt(true);

    let mut encoder_button: Input<'_> = Input::new(r.encoder_button, Pull::Up);
    encoder_button.set_schmitt(true);

    let publisher = KEY_EVENT_QUEUE.publisher().unwrap();

    loop {
        let (_, index) = select_array([
            key1.wait_for_any_edge(),
            key2.wait_for_any_edge(),
            key3.wait_for_any_edge(),
            encoder_button.wait_for_any_edge(),
        ])
        .await;

        // Small debounce delay to avoid reading bounce
        Timer::after(Duration::from_millis(5)).await;

        match index {
            0 => match key1.get_level() {
                Level::Low => publisher.publish_immediate(KeyEvent {
                    key: Key::Key1,
                    event: Event::Pressed,
                }),
                Level::High => publisher.publish_immediate(KeyEvent {
                    key: Key::Key1,
                    event: Event::Released,
                }),
            },
            1 => match key2.get_level() {
                Level::Low => publisher.publish_immediate(KeyEvent {
                    key: Key::Key2,
                    event: Event::Pressed,
                }),
                Level::High => publisher.publish_immediate(KeyEvent {
                    key: Key::Key2,
                    event: Event::Released,
                }),
            },
            2 => match key3.get_level() {
                Level::Low => publisher.publish_immediate(KeyEvent {
                    key: Key::Key3,
                    event: Event::Pressed,
                }),
                Level::High => publisher.publish_immediate(KeyEvent {
                    key: Key::Key3,
                    event: Event::Released,
                }),
            },
            3 => match encoder_button.get_level() {
                Level::Low => publisher.publish_immediate(KeyEvent {
                    key: Key::EncoderButton,
                    event: Event::Pressed,
                }),
                Level::High => publisher.publish_immediate(KeyEvent {
                    key: Key::EncoderButton,
                    event: Event::Released,
                }),
            },
            _ => unreachable!(),
        };
    }
}

/// Handle encoder rotation - sends appropriate MIDI value based on direction
async fn handle_encoder_interaction(
    mut sender: Sender<'static, embassy_rp::usb::Driver<'static, embassy_rp::peripherals::USB>>,
    config: &MidiInputConfig,
    increment: bool,
    mode: crate::DeviceMode,
) -> Sender<'static, embassy_rp::usb::Driver<'static, embassy_rp::peripherals::USB>> {
    // Step size for encoder (higher = faster, adjust to taste: 2, 4, 8, etc.)
    const ENCODER_STEP: u8 = 4;

    // Get mode index for the encoder value array
    let mode_index = match mode {
        crate::DeviceMode::Keyboard => 0,
        crate::DeviceMode::Picoprog => 1,
        crate::DeviceMode::Universal => 2,
    };

    // Update the internal counter (0-127) for this mode with saturation at boundaries
    let value = {
        let mut counters = ENCODER_VALUES.lock().await;
        if increment {
            counters[mode_index] = counters[mode_index].saturating_add(ENCODER_STEP).min(127);
        } else {
            counters[mode_index] = counters[mode_index].saturating_sub(ENCODER_STEP);
        }
        counters[mode_index]
    };

    let packet = encode_midi_packet(config, value);

    if let Err(e) = sender.write_packet(&packet).await {
        log::error!("Failed to send MIDI packet: {:?}", e);
    }

    sender
}

/// Send MIDI message for button press/release
async fn send_midi_message(
    mut sender: Sender<'static, embassy_rp::usb::Driver<'static, embassy_rp::peripherals::USB>>,
    config: &MidiInputConfig,
    event: Event,
) -> Sender<'static, embassy_rp::usb::Driver<'static, embassy_rp::peripherals::USB>> {
    // Map button press/release to MIDI values
    // For CC: 127 = pressed, 0 = released
    // For Notes: 127 = Note On (pressed), 0 = Note Off (released)
    let value = match event {
        Event::Pressed => 127,
        Event::Released => 0,
    };

    let packet = encode_midi_packet(config, value);

    if let Err(e) = sender.write_packet(&packet).await {
        log::error!("Failed to send MIDI packet: {:?}", e);
    }

    sender
}
