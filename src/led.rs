use crate::LedResources;
use crate::state::DEVICE_STATE;
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::PIO1;
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::pio_programs::ws2812::{PioWs2812, PioWs2812Program};
use embassy_time::{Duration, Ticker};
use smart_leds::RGB8;

bind_interrupts!(struct Irqs {
    PIO1_IRQ_0 => InterruptHandler<PIO1>;
});

#[embassy_executor::task]
pub async fn led_task(r: LedResources) -> ! {
    let Pio {
        mut common, sm0, ..
    } = Pio::new(r.peripheral, Irqs);

    const NUM_LEDS: usize = 4;
    let mut data = [RGB8::default(); NUM_LEDS];

    let program = PioWs2812Program::new(&mut common);
    let mut ws2812 = PioWs2812::new(&mut common, sm0, r.led_dma, r.led_gpio, &program);

    let mut ticker = Ticker::every(Duration::from_millis(10));
    let mut receiver = DEVICE_STATE.receiver().unwrap();

    // Wait for initial state
    let mut current_state = receiver.changed().await;

    // Set initial LED state based on state
    if !current_state.leds_active() {
        data = [RGB8::default(); NUM_LEDS];
        ws2812.write(&data).await;
    } else {
        data[3] = current_state.mode_color();
        ws2812.write(&data).await;  // Write immediately to show mode color
    }

    loop {
        // Check if state changed
        if let Some(new_state) = receiver.try_changed() {
            current_state = new_state;
            if !current_state.leds_active() {
                // Turn off all LEDs immediately
                data = [RGB8::default(); NUM_LEDS];
                ws2812.write(&data).await;
            } else {
                // Update mode color based on new state
                data[3] = current_state.mode_color();
            }
        }

        // Only animate LEDs if state is active
        if current_state.leds_active() {
            for j in 0..(256 * 5) {
                for i in 0..NUM_LEDS - 1 {
                    data[i] =
                        wheel((((i * 256) as u16 / (NUM_LEDS - 1) as u16 + j as u16) & 255) as u8);
                }
                ws2812.write(&data).await;

                ticker.next().await;

                // Check for state change during animation
                if let Some(new_state) = receiver.try_changed() {
                    current_state = new_state;
                    if !current_state.leds_active() {
                        // Turn off all LEDs before exiting animation
                        data = [RGB8::default(); NUM_LEDS];
                        ws2812.write(&data).await;
                        break;
                    } else {
                        // Update mode color if state changed during animation
                        data[3] = current_state.mode_color();
                    }
                }
            }
        } else {
            // When OFF, just wait
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
