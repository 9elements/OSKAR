use crate::{EncoderResources, ButtonResources};
use crate::state::DEVICE_STATE;
use defmt::unreachable;
use defmt_rtt as _;
use embassy_executor::{InterruptExecutor, Spawner};
use embassy_futures::select::select_array;
use embassy_rp::gpio::{Input, Level, Pull};
use embassy_rp::interrupt;
use embassy_rp::interrupt::{InterruptExt, Priority};
use embassy_rp::peripherals::USB;
use embassy_rp::usb::Driver;
use embassy_usb::class::hid::HidReaderWriter;
use usbd_hid::descriptor::*;
use embassy_sync::pubsub::PubSubChannel;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;

type CustomHid = HidReaderWriter<'static, Driver<'static, USB>, 1, 8>;

/// USB HID specification allows up to 6 simultaneous key presses (6-key rollover)
const MAX_SIMULTANEOUS_KEYS: usize = 6;

static KEY_EVENT_QUEUE: PubSubChannel::<CriticalSectionRawMutex, KeyEvent, 12, 2, 2> = PubSubChannel::new();

#[derive(Clone)]
#[derive(PartialEq)]
enum Key {
    EncoderLeft,
    EncoderRight,
    EncoderButton,
    Key1,
    Key2,
    Key3,
}

#[derive(Clone)]
#[derive(PartialEq)]
enum Event {
    Pressed,
    Released,
}
#[derive(Clone)]
struct KeyEvent {
    key: Key,
    event: Event,
}

#[derive(Copy, Clone, PartialEq)]
pub enum KeyType {
    Media(MediaKey),
    Keycode(KeyboardUsage),
}

#[embassy_executor::task]
pub async fn hid_task(spawner: Spawner, mut keyboard_class: CustomHid, mut multimedia_class: CustomHid, button_resources: ButtonResources, encoder_resources: EncoderResources) -> ! {

    interrupt::SWI_IRQ_0.set_priority(Priority::P2);
    let spawner_encoder: embassy_executor::SendSpawner = EXECUTOR_ENCODER.start(interrupt::SWI_IRQ_0);
    spawner_encoder.spawn(encoder_task(encoder_resources)).unwrap();

    spawner.spawn(button_task(button_resources)).unwrap();

    let mut sub = KEY_EVENT_QUEUE.subscriber().unwrap();
    let mut state_receiver = DEVICE_STATE.receiver().unwrap();

    // Wait for initial state
    let mut current_state = state_receiver.changed().await;

    // Track currently pressed regular keys
    let mut pressed_keys: [u8; MAX_SIMULTANEOUS_KEYS] = [0; MAX_SIMULTANEOUS_KEYS];

    loop {
        let key_event: KeyEvent = sub.next_message_pure().await;

        // Check for state updates (non-blocking)
        if let Some(new_state) = state_receiver.try_changed() {
            // Clear pressed keys when state changes to prevent drift
            if current_state != new_state {
                pressed_keys = [0; MAX_SIMULTANEOUS_KEYS];

                // Send empty keyboard report to release all keys
                let empty_report = KeyboardReport {
                    keycodes: [0; MAX_SIMULTANEOUS_KEYS],
                    leds: 0,
                    modifier: 0,
                    reserved: 0,
                };
                let _ = keyboard_class.write_serialize(&empty_report).await;
            }

            current_state = new_state;
        }

        // Only process keys if keyboard is active
        let layout = current_state.layout();
        if !layout.active {
            continue; // Skip processing when keyboard is off (Layout 1)
        }

        match key_event.key {
            Key::EncoderLeft => {
                handle_encoder_interaction(&mut keyboard_class, &mut multimedia_class, layout.encoder_left).await;
            },
            Key::EncoderRight => {
                handle_encoder_interaction(&mut keyboard_class, &mut multimedia_class, layout.encoder_right).await;
            },
            Key::EncoderButton => {
                send_code(&mut keyboard_class, &mut multimedia_class, layout.encoder_button, key_event.event, &mut pressed_keys).await;
            },
            Key::Key1 => {
                send_code(&mut keyboard_class, &mut multimedia_class, layout.key1, key_event.event, &mut pressed_keys).await;
            },
            Key::Key2 => {
                send_code(&mut keyboard_class, &mut multimedia_class, layout.key2, key_event.event, &mut pressed_keys).await;
            },
            Key::Key3 => {
                send_code(&mut keyboard_class, &mut multimedia_class, layout.key3, key_event.event, &mut pressed_keys).await;
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
            publisher.publish_immediate(KeyEvent {key: Key::EncoderLeft, event: Event::Pressed});
        } else {
            publisher.publish_immediate(KeyEvent {key: Key::EncoderRight, event: Event::Pressed});
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

    // Track previous state of all buttons (High = not pressed, Low = pressed)
    let mut prev_states: [Level; 4] = [Level::High; 4];

    loop {
        // Wait for any button to change state
        select_array([
            key1.wait_for_any_edge(),
            key2.wait_for_any_edge(),
            key3.wait_for_any_edge(),
            encoder_button.wait_for_any_edge(),
        ])
        .await;

        // Debounce delay
        embassy_time::Timer::after(embassy_time::Duration::from_millis(5)).await;

        // Poll ALL button states (more reliable than edge detection)
        let current_states = [
            key1.get_level(),
            key2.get_level(),
            key3.get_level(),
            encoder_button.get_level(),
        ];

        // Compare with previous state and publish changes
        for (i, (&prev, &curr)) in prev_states.iter().zip(current_states.iter()).enumerate() {
            if prev != curr {
                let key = match i {
                    0 => Key::Key1,
                    1 => Key::Key2,
                    2 => Key::Key3,
                    3 => Key::EncoderButton,
                    _ => unreachable!(),
                };

                let event = match curr {
                    Level::Low => Event::Pressed,
                    Level::High => Event::Released,
                };

                publisher.publish_immediate(KeyEvent { key, event });
            }
        }

        // Update previous state
        prev_states = current_states;
    }
}


async fn handle_encoder_interaction(keyboard_class: &mut CustomHid, media_class: &mut CustomHid, code: KeyType) {
    match code {
        KeyType::Media(media_key) => {
            let mut report = MediaKeyboardReport {
                usage_id: media_key as u16,
            };

            if let Err(e) = media_class.write_serialize(&report).await {
                log::error!("Failed to send HID key press: {:?}", e);
            }

            report = MediaKeyboardReport {
                usage_id: 0x00 as u16,
            };

            if let Err(e) = media_class.write_serialize(&report).await {
                log::error!("Failed to send HID key press: {:?}", e);
            }
        },

        KeyType::Keycode(keyboard_usage) => {
            let mut keycodes = [0; MAX_SIMULTANEOUS_KEYS];
            keycodes[0] = keyboard_usage as u8;

            let mut report: KeyboardReport = KeyboardReport {
                keycodes,
                leds: 0,
                modifier: 0,
                reserved: 0,
            };

            if let Err(e) = keyboard_class.write_serialize(&report).await {
                log::error!("Failed to send HID key press: {:?}", e);
            }

            report.keycodes = [0; MAX_SIMULTANEOUS_KEYS];

            if let Err(e) = keyboard_class.write_serialize(&report).await {
                log::error!("Failed to send HID key press: {:?}", e);
            }
        },
    }
}

async fn send_code(keyboard_class: &mut CustomHid, media_class: &mut CustomHid, code: KeyType, event: Event, pressed_keys: &mut [u8; MAX_SIMULTANEOUS_KEYS]) {
    match code {
        KeyType::Media(media_key) => {
            // Media keys don't support multiple simultaneous presses in standard HID
            let code = match event {
                Event::Pressed => media_key as u16,
                Event::Released => 0x00 as u16,
            };

            let report = MediaKeyboardReport {
                usage_id: code,
            };

            if let Err(e) = media_class.write_serialize(&report).await {
                log::error!("Failed to send HID media key: {:?}", e);
            }
        },

        KeyType::Keycode(keyboard_usage) => {
            let key_code = keyboard_usage as u8;

            // Update pressed keys array
            match event {
                Event::Pressed => {
                    // Add key if not   present and there's space
                    if !pressed_keys.contains(&key_code) {
                        for slot in pressed_keys.iter_mut() {
                            if *slot == 0 {
                                *slot = key_code;
                                break;
                            }
                        }
                    }
                },
                Event::Released => {
                    // Remove key from pressed keys (first occurrence only)
                    let mut found_index = None;
                    for (i, slot) in pressed_keys.iter().enumerate() {
                        if *slot == key_code {
                            found_index = Some(i);
                            break;
                        }
                    }

                    // If found, shift all subsequent keys left to fill the gap
                    if let Some(index) = found_index {
                        for i in index..5 {
                            pressed_keys[i] = pressed_keys[i + 1];
                        }
                        pressed_keys[5] = 0;
                    }
                },
            }

            // Send report with all currently pressed keys
            let report: KeyboardReport = KeyboardReport {
                keycodes: *pressed_keys,
                leds: 0,
                modifier: 0,
                reserved: 0,
            };

            if let Err(e) = keyboard_class.write_serialize(&report).await {
                log::error!("Failed to send HID keyboard report: {:?}", e);
            }
        },
    }
}