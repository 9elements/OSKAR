use crate::{EncoderResources, ButtonResources};
use crate::hid_codes::{Keycode, KeyLayout};
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
use usbd_hid::descriptor::KeyboardReport;
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

const KEYLAYOUT:KeyLayout = KeyLayout {
    encoder_left: Keycode::VolumeDown,
    encoder_right: Keycode::VolumeUp,
    encoder_button: Keycode::Mute,
    key1: Keycode::O,
    key2: Keycode::S,
    key3: Keycode::F,
};

#[embassy_executor::task]
pub async fn hid_task(spawner: Spawner, mut hid: CustomHid, button_resources: ButtonResources, encoder_resources: EncoderResources) -> ! {

    interrupt::SWI_IRQ_0.set_priority(Priority::P2);
    let spawner_encoder: embassy_executor::SendSpawner = EXECUTOR_ENCODER.start(interrupt::SWI_IRQ_0);
    spawner_encoder.spawn(encoder_task(encoder_resources)).unwrap();

    spawner.spawn(button_task(button_resources)).unwrap();

    let mut sub = KEY_EVENT_QUEUE.subscriber().unwrap();


    loop {
        let key_event: KeyEvent = sub.next_message_pure().await;

        match key_event.key {
            Key::EncoderLeft => {
                hid = handle_encoder_interaction(hid, KEYLAYOUT.encoder_left).await;
            },
            Key::EncoderRight => {
                hid = handle_encoder_interaction(hid, KEYLAYOUT.encoder_right).await;
            },
            Key::EncoderButton => {
                hid = send_keycode(hid, KEYLAYOUT.encoder_button, key_event.event).await;
            },
            Key::Key1 => {
                hid = send_keycode(hid, KEYLAYOUT.key1, key_event.event).await;
            },
            Key::Key2 => {
                hid = send_keycode(hid, KEYLAYOUT.key2, key_event.event).await;
            },
            Key::Key3 => {
                hid = send_keycode(hid, KEYLAYOUT.key3,key_event.event).await;
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


async fn handle_encoder_interaction(mut hid: CustomHid, keycode: Keycode) -> CustomHid {

    send_report(&mut hid, [keycode as u8, 0, 0, 0, 0, 0]).await;
    send_report(&mut hid, [0, 0, 0, 0, 0, 0]).await;

    return hid;
}

async fn send_keycode(mut hid: CustomHid, keycode: Keycode, event: Event) -> CustomHid {

    let keycodes: [u8; 6] = if event == Event::Pressed {
        [keycode as u8, 0, 0, 0, 0, 0]
    } else {
        [0, 0, 0, 0, 0, 0]
    };

    send_report(&mut hid, keycodes).await;
    return hid;
}

async fn send_report(hid: &mut CustomHid, keycodes: [u8; 6]) {
    let report = KeyboardReport {
        keycodes: keycodes,
        leds: 0,
        modifier: 0,
        reserved: 0,
    };

    if let Err(e) = hid.write_serialize(&report).await {
        log::error!("Failed to send HID key press: {:?}", e);
    }
}