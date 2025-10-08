use crate::{DeviceMode, LedResources};
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::PIO1;
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::pio_programs::ws2812::{PioWs2812, PioWs2812Program};
use embassy_time::{Duration, Ticker};
use smart_leds::RGB8;

bind_interrupts!(struct Irqs {
    PIO1_IRQ_0 => InterruptHandler<PIO1>;
});

const COLOR_RED: RGB8 = RGB8 { r: 10, g: 0, b: 0 };
const COLOR_GREEN: RGB8 = RGB8 { r: 10, g: 10, b: 0 };
const COLOR_PURPLE: RGB8 = RGB8 { r: 14, g: 4, b: 13 };

#[embassy_executor::task]
pub async fn led_task(r: LedResources, mode: DeviceMode) -> ! {
    let Pio {
        mut common, sm0, ..
    } = Pio::new(r.peripheral, Irqs);

    const NUM_LEDS: usize = 4;
    let mut data = [RGB8::default(); NUM_LEDS];
    data[3] = match mode {
        DeviceMode::Keyboard => COLOR_RED,
        DeviceMode::Universal => COLOR_PURPLE,
        DeviceMode::Picoprog => COLOR_GREEN,
    };

    let program = PioWs2812Program::new(&mut common);
    let mut ws2812 = PioWs2812::new(&mut common, sm0, r.led_dma, r.led_gpio, &program);

    let mut ticker = Ticker::every(Duration::from_millis(10));
    loop {
        for j in 0..(256 * 5) {
            for i in 0..NUM_LEDS - 1 {
                data[i] =
                    wheel((((i * 256) as u16 / (NUM_LEDS - 1) as u16 + j as u16) & 255) as u8);
            }
            ws2812.write(&data).await;

            ticker.next().await;
        }
    }
}

fn wheel(mut wheel_pos: u8) -> RGB8 {
    wheel_pos = 255 - wheel_pos;
    if wheel_pos < 85 {
        return (255 - wheel_pos * 3, 0, wheel_pos * 3).into();
    }
    if wheel_pos < 170 {
        wheel_pos -= 85;
        return (0, wheel_pos * 3, 255 - wheel_pos * 3).into();
    }
    wheel_pos -= 170;
    (wheel_pos * 3, 255 - wheel_pos * 3, 0).into()
}
