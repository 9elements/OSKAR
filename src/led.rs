use crate::{DeviceMode, LedResources};
use embassy_rp::bind_interrupts;
use embassy_rp::peripherals::PIO1;
use embassy_rp::pio::{InterruptHandler, Pio};
use embassy_rp::pio_programs::ws2812::{PioWs2812, PioWs2812Program};
use smart_leds::RGB8;

bind_interrupts!(struct Irqs {
    PIO1_IRQ_0 => InterruptHandler<PIO1>;
});

#[embassy_executor::task]
pub async fn led_task(r: LedResources, _initial_mode: DeviceMode) -> ! {
    let Pio {
        mut common, sm0, ..
    } = Pio::new(r.peripheral, Irqs);

    const NUM_LEDS: usize = 4;
    let mut data = [RGB8::default(); NUM_LEDS];

    let program = PioWs2812Program::new(&mut common);
    let mut ws2812 = PioWs2812::new(&mut common, sm0, r.led_dma, r.led_gpio, &program);

    loop {
        // Read current mode from shared mutex
        let current_mode = {
            let mode = crate::CURRENT_MODE.lock().await;
            *mode
        };

        // Set colors based on mode: Position 1=Teal, Position 2=Orange, Position 3=Pink
        let color = match current_mode {
            DeviceMode::Keyboard => RGB8 { r: 0, g: 10, b: 8 }, // Teal
            DeviceMode::Picoprog => RGB8 { r: 10, g: 3, b: 0 }, // Orange
            DeviceMode::Universal => RGB8 { r: 10, g: 0, b: 5 }, // Pink
        };

        // Set all 4 LEDs to the same mode color
        data[0] = color;
        data[1] = color;
        data[2] = color;
        data[3] = color;

        // Write the updated colors
        ws2812.write(&data).await;

        // Wait for mode change signal
        crate::MODE_CHANGED.wait().await;
    }
}
