#![no_std]
#![no_main]
#![allow(async_fn_in_trait)]
#![allow(incomplete_features)]
#![feature(impl_trait_in_assoc_type)]
#![feature(type_alias_impl_trait)]

use assign_resources::assign_resources;
use core::panic::PanicInfo;
use cortex_m::peripheral::SCB;
use defmt_rtt as _;
use embassy_executor::Spawner;
use embassy_rp::bind_interrupts;
use embassy_rp::flash::{Async, Flash};
use embassy_rp::gpio::{Input, Level, Pull};
use embassy_rp::peripherals::{self, PIO0, USB};
use embassy_rp::pio::InterruptHandler as PIOInterruptHandler;
use embassy_rp::usb::{Driver, InterruptHandler as USBInterruptHandler};
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_sync::mutex::Mutex;
use embassy_sync::signal::Signal;
use embassy_usb::class::midi::MidiClass;
use embassy_usb::{Config as UsbConfig, UsbDevice};
use heapless::String;
use static_cell::StaticCell;
use ufmt::uwrite;

// Global mutex to share current mode between tasks
pub static CURRENT_MODE: Mutex<CriticalSectionRawMutex, DeviceMode> =
    Mutex::new(DeviceMode::Keyboard);

// Signal to notify when mode changes
pub static MODE_CHANGED: Signal<CriticalSectionRawMutex, ()> = Signal::new();

mod layouts;
mod led;
mod midi;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => USBInterruptHandler<USB>;
    PIO0_IRQ_0 => PIOInterruptHandler<PIO0>;
});

assign_resources! {
    hid: ButtonResources{
        key1: PIN_19,
        key2: PIN_20,
        key3: PIN_21,
        encoder_button: PIN_13,
    }

    encoder: EncoderResources{
        encoder_right: PIN_14,
        encoder_left: PIN_12,
    }

    led: LedResources{
        peripheral: PIO1,
        led_gpio: PIN_18,
        led_dma: DMA_CH0,
    }

    selector_switch: ModeSwitchRessources{
        selector_kb: PIN_16,
        selector_picocprog: PIN_17,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DeviceMode {
    Keyboard,
    Picoprog,
    Universal,
}

// According to Serial Flasher Protocol Specification - version 1
const FLASH_SIZE: usize = 2 * 1024 * 1024;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p: embassy_rp::Peripherals = embassy_rp::init(Default::default());
    let r: AssignedResources = split_resources!(p);
    let driver = Driver::new(p.USB, Irqs);

    let selector_keyboard: Input<'static> = Input::new(r.selector_switch.selector_kb, Pull::Up);
    let selector_picoprog: Input<'static> =
        Input::new(r.selector_switch.selector_picocprog, Pull::Up);

    let mode: DeviceMode = if selector_keyboard.get_level() == Level::Low {
        defmt::info!("keyboard mode");
        DeviceMode::Keyboard
    } else if selector_picoprog.get_level() == Level::Low {
        defmt::info!("picoprog mode");
        DeviceMode::Picoprog
    } else {
        defmt::info!("neutral mode");
        DeviceMode::Universal
    };

    // Initialize the shared mode mutex
    {
        let mut current_mode = CURRENT_MODE.lock().await;
        *current_mode = mode;
    }

    let mut flash = Flash::<_, Async, FLASH_SIZE>::new(p.FLASH, p.DMA_CH4);
    let mut uid: [u8; 8] = [0; 8];
    flash.blocking_unique_id(&mut uid).unwrap_or_default();

    static UID_STR: StaticCell<String<16>> = StaticCell::new();
    let uid_str = UID_STR.init(String::<16>::new());
    for byte in uid.iter() {
        uwrite!(uid_str, "{:02X}", *byte).unwrap_or_default();
    }

    let config = {
        let mut config = UsbConfig::new(0x1ced, 0xc0fe);
        config.manufacturer = Some("9elements");
        config.product = Some("oskar");
        config.serial_number = Some(uid_str.as_str());
        config.max_power = 100;
        config.max_packet_size_0 = 64;

        // Required for windows compatibility.
        // https://developer.nordicsemi.com/nRF_Connect_SDK/doc/1.9.1/kconfig/CONFIG_CDC_ACM_IAD.html#help
        config.device_class = 0xEF;
        config.device_sub_class = 0x02;
        config.device_protocol = 0x01;
        config.composite_with_iads = true;
        config
    };

    let mut builder: embassy_usb::Builder<'_, Driver<'_, USB>> = {
        static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();
        static MSOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();

        let builder = embassy_usb::Builder::new(
            driver,
            config,
            CONFIG_DESCRIPTOR.init([0; 256]),
            BOS_DESCRIPTOR.init([0; 256]),
            MSOS_DESCRIPTOR.init([0; 256]), // no msos descriptors
            CONTROL_BUF.init([0; 64]),
        );
        builder
    };

    spawner.spawn(led::led_task(r.led, mode)).unwrap();

    // Create MIDI class with 1 input jack, 1 output jack, and 64-byte packet size
    let midi_class = MidiClass::new(&mut builder, 1, 1, 64);

    spawner
        .spawn(midi::midi_task(
            spawner,
            midi_class,
            r.hid,
            r.encoder,
            mode,
            selector_keyboard,
            selector_picoprog,
        ))
        .unwrap();

    let usb = builder.build();
    // We can't really recover here so just unwrap
    spawner.spawn(usb_task(usb)).unwrap();

    loop {
        embassy_time::Timer::after(embassy_time::Duration::from_secs(1)).await;
    }
}

type CustomUsbDriver = Driver<'static, USB>;
type CustomUsbDevice = UsbDevice<'static, CustomUsbDriver>;

#[embassy_executor::task]
async fn usb_task(mut usb: CustomUsbDevice) -> ! {
    usb.run().await
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Print out the panic info
    log::error!("Panic occurred: {:?}", info);

    // Reboot the system
    SCB::sys_reset();
}
