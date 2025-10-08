use crate::{EncoderResources, ButtonResources};
use crate::layouts::{KeyLayout};
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
static KEY_EVENT_QUEUE: PubSubChannel::<CriticalSectionRawMutex, KeyEvent, 2, 2, 2> = PubSubChannel::new();

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

pub enum KeyType {
    Media(MediaKey),
    Keycode(KeyboardUsage),
}

const KEYLAYOUT:KeyLayout = KeyLayout {
    encoder_left: KeyType::Media(MediaKey::VolumeDecrement),
    encoder_right: KeyType::Media(MediaKey::VolumeIncrement),
    encoder_button: KeyType::Media(MediaKey::Mute),
    key1: KeyType::Keycode(KeyboardUsage::KeyboardOo),
    key2: KeyType::Keycode(KeyboardUsage::KeyboardSs),
    key3: KeyType::Keycode(KeyboardUsage::KeyboardFf),
};

#[embassy_executor::task]
pub async fn hid_task(spawner: Spawner, mut keyboard_class: CustomHid, mut multimedia_class: CustomHid, button_resources: ButtonResources, encoder_resources: EncoderResources) -> ! {

    interrupt::SWI_IRQ_0.set_priority(Priority::P2);
    let spawner_encoder: embassy_executor::SendSpawner = EXECUTOR_ENCODER.start(interrupt::SWI_IRQ_0);
    spawner_encoder.spawn(encoder_task(encoder_resources)).unwrap();

    spawner.spawn(button_task(button_resources)).unwrap();

    let mut sub = KEY_EVENT_QUEUE.subscriber().unwrap();

    loop {
        let key_event: KeyEvent = sub.next_message_pure().await;

        match key_event.key {
            Key::EncoderLeft => {
                (keyboard_class, multimedia_class) = handle_encoder_interaction(keyboard_class, multimedia_class, KEYLAYOUT.encoder_left).await;
            },
            Key::EncoderRight => {
                (keyboard_class, multimedia_class) = handle_encoder_interaction(keyboard_class, multimedia_class, KEYLAYOUT.encoder_right).await;
            },
            Key::EncoderButton => {
                (keyboard_class, multimedia_class) = send_code(keyboard_class, multimedia_class, KEYLAYOUT.encoder_button, key_event.event).await;
            },
            Key::Key1 => {
                (keyboard_class, multimedia_class) = send_code(keyboard_class, multimedia_class, KEYLAYOUT.key1, key_event.event).await;
            },
            Key::Key2 => {
                (keyboard_class, multimedia_class) = send_code(keyboard_class, multimedia_class, KEYLAYOUT.key2, key_event.event).await;
            },
            Key::Key3 => {
                (keyboard_class, multimedia_class) = send_code(keyboard_class, multimedia_class, KEYLAYOUT.key3,key_event.event).await;
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

    loop {

        let (_, index) = select_array([
            key1.wait_for_any_edge(),
            key2.wait_for_any_edge(),
            key3.wait_for_any_edge(),
            encoder_button.wait_for_any_edge(),
        ])
        .await;

        match index {
            0 => {
                match key1.get_level() {
                    Level::Low => publisher.publish_immediate(KeyEvent {key: Key::Key1, event: Event::Pressed}),
                    Level::High => publisher.publish_immediate(KeyEvent {key: Key::Key1, event: Event::Released}),
                }
            }
            1 => {
                match key2.get_level() {
                    Level::Low => publisher.publish_immediate(KeyEvent {key: Key::Key2, event: Event::Pressed}),
                    Level::High => publisher.publish_immediate(KeyEvent {key: Key::Key2, event: Event::Released}),
                }
            }
            2 => {
                match key3.get_level() {
                    Level::Low => publisher.publish_immediate(KeyEvent {key: Key::Key3, event: Event::Pressed}),
                    Level::High => publisher.publish_immediate(KeyEvent {key: Key::Key3, event: Event::Released}),
                }
            }
            3 => {
                match encoder_button.get_level() {
                    Level::Low => publisher.publish_immediate(KeyEvent {key: Key::EncoderButton, event: Event::Pressed}),
                    Level::High => publisher.publish_immediate(KeyEvent {key: Key::EncoderButton, event: Event::Released}),
                }
            }
            _ => unreachable!(),
        };
    }
}


async fn handle_encoder_interaction(mut keyboard_class: CustomHid, mut media_class: CustomHid, code: KeyType) -> (CustomHid, CustomHid) {

    match code {
        KeyType::Media(media_key) =>    {

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
            let keycodes: [u8; 6] = [keyboard_usage as u8, 0, 0, 0, 0, 0];

            let mut report: KeyboardReport = KeyboardReport {
                keycodes: keycodes,
                leds: 0,
                modifier: 0,
                reserved: 0,
            };

            if let Err(e) = keyboard_class.write_serialize(&report).await {
                log::error!("Failed to send HID key press: {:?}", e);
            }

            report.keycodes = [0,0,0,0,0,0];

            if let Err(e) = keyboard_class.write_serialize(&report).await {
                log::error!("Failed to send HID key press: {:?}", e);
            }
        },
    };



    return (keyboard_class, media_class)
}

async fn send_code(mut keyboard_class: CustomHid, mut media_class: CustomHid , code: KeyType, event: Event) -> (CustomHid, CustomHid) {

    match code {
        KeyType::Media(media_key) =>    {

            let code = match event {
                Event::Pressed => media_key as u16,
                Event::Released => 0x00 as u16,
            };

            let report = MediaKeyboardReport {
                usage_id: code,
            };

            if let Err(e) = media_class.write_serialize(&report).await {
                log::error!("Failed to send HID key press: {:?}", e);
            }
        },

        KeyType::Keycode(keyboard_usage) => {
            let keycodes: [u8; 6] = if event == Event::Pressed {
                [keyboard_usage as u8, 0, 0, 0, 0, 0]
            } else {
                [0, 0, 0, 0, 0, 0]
            };

            let report: KeyboardReport = KeyboardReport {
                keycodes: keycodes,
                leds: 0,
                modifier: 0,
                reserved: 0,
            };

            if let Err(e) = keyboard_class.write_serialize(&report).await {
                log::error!("Failed to send HID key press: {:?}", e);
            }
        },
    };

    return (keyboard_class, media_class);
}