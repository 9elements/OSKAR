use crate::HidResources;
use defmt::unreachable;
use defmt_rtt as _;
use embassy_futures::select::select_array;
use embassy_rp::gpio::{Input, Level, Pull};
use embassy_rp::peripherals::USB;
use embassy_rp::usb::Driver;
use embassy_time::{Duration, with_timeout};
use embassy_usb::class::hid::HidReaderWriter;
use usbd_hid::descriptor::KeyboardReport;
type CustomHid = HidReaderWriter<'static, Driver<'static, USB>, 1, 8>;

enum Direction {
    Left,
    Right,
}

#[embassy_executor::task]
pub async fn hid_task(mut hid: CustomHid, r: HidResources) -> ! {
    let mut key1: Input<'_> = Input::new(r.key1, Pull::Up);
    key1.set_schmitt(true);

    let mut key2: Input<'_> = Input::new(r.key2, Pull::Up);
    key2.set_schmitt(true);

    let mut key3: Input<'_> = Input::new(r.key3, Pull::Up);
    key3.set_schmitt(true);

    let mut encoder_button: Input<'_> = Input::new(r.encoder_button, Pull::Up);
    encoder_button.set_schmitt(true);

    let mut encoder_left: Input<'_> = Input::new(r.encoder_left, Pull::Up);

    let mut encoder_right: Input<'_> = Input::new(r.encoder_right, Pull::Up);

    loop {
        defmt::info!("HID: waiting for trigger...");

        _ = with_timeout(Duration::from_millis(TIMEOUT), async {
            encoder_left.wait_for_high()
        })
        .await;

        let (_, index) = select_array([
            key1.wait_for_any_edge(),
            key2.wait_for_any_edge(),
            key3.wait_for_any_edge(),
            encoder_button.wait_for_any_edge(),
            encoder_right.wait_for_any_edge(),
        ])
        .await;

        let keycode = match index {
            0 => {
                defmt::info!("key1");
                match key1.get_level() {
                    Level::Low => 0x12,
                    Level::High => 0x0,
                }
            }
            1 => {
                defmt::info!("key2");
                match key2.get_level() {
                    Level::Low => 0x16,
                    Level::High => 0x0,
                }
            }
            2 => {
                defmt::info!("key3");
                match key3.get_level() {
                    Level::Low => 0x9,
                    Level::High => 0x0,
                }
            }
            3 => {
                defmt::info!("ecoder_button");
                match encoder_button.get_level() {
                    Level::Low => 0x7f,
                    Level::High => 0x0,
                }
            }
            4 => {
                let edge = match encoder_right.get_level() {
                    Level::Low => 0,
                    Level::High => 1,
                };

                let state = match encoder_left.get_level() {
                    Level::Low => 0,
                    Level::High => 1,
                };

                let direction = if edge == state {
                    Direction::Left
                } else {
                    Direction::Right
                };

                hid = handle_encoder_interaction(hid, direction).await;

                continue;
            }

            _ => unreachable!(),
        };

        let keycodes: [u8; 6] = [keycode, 0, 0, 0, 0, 0];
        send_report(&mut hid, keycodes).await;
    }
}

const TIMEOUT: u64 = 200;

async fn handle_encoder_interaction(mut hid: CustomHid, direction: Direction) -> CustomHid {
    let keycode = match direction {
        Direction::Right => 0x80,
        Direction::Left => 0x81,
    };

    send_report(&mut hid, [keycode, 0, 0, 0, 0, 0]).await;
    send_report(&mut hid, [0, 0, 0, 0, 0, 0]).await;
    return hid;
}

async fn send_report(hid: &mut CustomHid, keycodes: [u8; 6]) {
    let report = KeyboardReport {
        keycodes: keycodes,
        leds: 0,
        modifier: 0,
        reserved: 0,
    };

    // Send the report
    if let Err(e) = hid.write_serialize(&report).await {
        log::error!("Failed to send HID key press: {:?}", e);
    }
}
