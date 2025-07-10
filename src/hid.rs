use defmt_rtt as _;
use defmt::unreachable;
use embassy_futures::select::{select_array};
use embassy_rp::gpio::{Input, Level, Pull};
use embassy_rp::peripherals::{USB};
use embassy_rp::usb::{Driver};
use embassy_time::{Duration, with_timeout};
use embassy_usb::class::hid::{HidReaderWriter};
use usbd_hid::descriptor::{KeyboardReport};
use crate::HidResources;
type CustomHid = HidReaderWriter<'static, Driver<'static, USB>, 1, 8>;

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
    encoder_button.set_schmitt(true);

    loop {
        defmt::info!("HID: waiting for trigger...");

        let (_, index) = select_array([
            key1.wait_for_any_edge(),
            key2.wait_for_any_edge(),
            key3.wait_for_any_edge(),
            encoder_button.wait_for_any_edge(),
            encoder_left.wait_for_any_edge(),
            encoder_right.wait_for_any_edge(),
        ])
        .await;

        let keycode = match index {
            0 => {
                defmt::info!("key1");
                match key1.get_level() { Level::Low => {0x12} Level::High => {0x0}}
            }
            1 => {
                defmt::info!("key2");
                match key2.get_level() { Level::Low => {0x16} Level::High => {0x0}}
            }
            2 => {
                defmt::info!("key3");
                match key3.get_level() { Level::Low => {0x9} Level::High => {0x0}}
            }
            3 => {
                defmt::info!("ecoder_button");
                match encoder_button.get_level() { Level::Low => {0x7f} Level::High => {0x0}}
            }
            4 => {
                hid = handle_encoder_interaction(&mut encoder_left, &mut encoder_right, hid, 0x81).await;
                continue;
            }
            5 => {
                hid = handle_encoder_interaction(&mut encoder_right, &mut encoder_left, hid, 0x80).await;
                continue;
            }
            _ => unreachable!(),
        };

        let keycodes:[u8; 6] = [keycode, 0 ,0 ,0 ,0 ,0];
        send_report(&mut hid, keycodes).await;

    }

}


const TIMEOUT: u64 = 200;

async fn handle_encoder_interaction(encoder_a: &mut Input<'_>,encoder_b:&mut Input<'_>,mut hid: CustomHid, keycode: u8) -> CustomHid{
    if encoder_a.get_level() == Level::High || encoder_b.get_level() == Level::Low {
        return hid
    }

    let result = with_timeout(Duration::from_millis(TIMEOUT), async {
        encoder_b.wait_for_falling_edge().await;
        encoder_a.wait_for_rising_edge().await;
        encoder_b.wait_for_rising_edge().await;
    })
    .await;

    match result {
        Ok(_) => {
            send_report(&mut hid, [keycode, 0, 0, 0, 0, 0]).await;
            send_report(&mut hid, [0, 0, 0, 0, 0, 0]).await;
            return hid
        }
        Err(_) => {
            return hid
        }
    }
}

async fn send_report(hid: &mut CustomHid, keycodes:[u8;6]) {
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
