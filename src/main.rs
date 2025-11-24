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
use embassy_rp::gpio::{Level, Output};
use embassy_rp::peripherals::{self, PIO0, SPI0, USB};
use embassy_rp::pio::InterruptHandler as PIOInterruptHandler;
use embassy_rp::spi::{Config as SpiConfig, Spi};
use embassy_rp::usb::{Driver, InterruptHandler as USBInterruptHandler};
use embassy_rp::watchdog::Watchdog;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State as CdcAcmState};
use embassy_usb::class::hid::{HidReaderWriter, State as HidState};
use embassy_usb::{Config as UsbConfig, UsbDevice};
use usbd_hid::descriptor::{KeyboardReport, MediaKeyboardReport, SerializedDescriptor};
use heapless::String;
use static_cell::StaticCell;
use ufmt::uwrite;

mod hid;
mod layouts;
mod led;
mod state;
mod uart;

// USB Configuration Constants
const USB_MAX_PACKET_SIZE: u16 = 64;
const HID_POLL_INTERVAL_MS: u8 = 60;
const CDC_MAX_PACKET_SIZE: u16 = 64;

bind_interrupts!(struct Irqs {
    USBCTRL_IRQ => USBInterruptHandler<USB>;
    PIO0_IRQ_0 => PIOInterruptHandler<PIO0>;
});

assign_resources! {
    uart: UartResources{
        peripheral: PIO0,
        tx: PIN_0,
        rx: PIN_1,
    }
    spi: SpiResources{
        peripheral: SPI0,
        clk: PIN_2,
        mosi: PIN_3,
        mosi_dma: DMA_CH2,
        miso: PIN_4,
        miso_dma: DMA_CH3,
        cs: PIN_5,
        led: PIN_25,
    }

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

    selector: SelectorResources{
        switch1: PIN_16,
        switch2: PIN_17,
    }
}

// According to Serial Flasher Protocol Specification - version 1
const FLASH_SIZE: usize = 2 * 1024 * 1024;



#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p: embassy_rp::Peripherals = embassy_rp::init(Default::default());
    let r: AssignedResources = split_resources!(p);
    let driver = Driver::new(p.USB, Irqs);
    
    let _watchdog = Watchdog::new(p.WATCHDOG);

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
        static MSOS_DESCRIPTOR: StaticCell <[u8; 256]> = StaticCell::new();

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

    spawner.spawn(state::state_manager_task(r.selector)).unwrap();
    spawner.spawn(led::led_task(r.led)).unwrap();

    // Create all USB classes from builder
    let uart_class = {
        static STATE: StaticCell<CdcAcmState> = StaticCell::new();
        let state = STATE.init(CdcAcmState::new());
        CdcAcmClass::new(&mut builder, state, CDC_MAX_PACKET_SIZE)
    };

    let serprog_class = {
        static STATE: StaticCell<CdcAcmState> = StaticCell::new();
        let state = STATE.init(CdcAcmState::new());
        CdcAcmClass::new(&mut builder, state, CDC_MAX_PACKET_SIZE)
    };

    let keyboard_class = {
        static STATE: StaticCell<HidState> = StaticCell::new();
        let state = STATE.init(HidState::new());

        let config = embassy_usb::class::hid::Config {
            report_descriptor: KeyboardReport::desc(),
            request_handler: None,
            poll_ms: HID_POLL_INTERVAL_MS,
            max_packet_size: USB_MAX_PACKET_SIZE,
        };

        HidReaderWriter::<'_, Driver<'_, USB>, 1, 8>::new(&mut builder, state, config)
    };

    let multimedia_class = {
        static STATE: StaticCell<HidState> = StaticCell::new();
        let state = STATE.init(HidState::new());

        let config = embassy_usb::class::hid::Config {
            report_descriptor: MediaKeyboardReport::desc(),
            request_handler: None,
            poll_ms: HID_POLL_INTERVAL_MS,
            max_packet_size: USB_MAX_PACKET_SIZE,
        };

        HidReaderWriter::<'_, Driver<'_, USB>, 1, 8>::new(&mut builder, state, config)
    };

    let usb = builder.build();

    spawner.spawn(usb_task(usb)).unwrap();
    spawner.spawn(uart::uart_task(uart_class, r.uart)).unwrap();
    spawner.spawn(serprog_task(serprog_class, r.spi)).unwrap();
    spawner.spawn(hid::hid_task(spawner, keyboard_class, multimedia_class, r.hid, r.encoder)).unwrap();

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

#[embassy_executor::task]
async fn serprog_task(class: CdcAcmClass<'static, CustomUsbDriver>, r: SpiResources) -> ! {
    let mut config = SpiConfig::default();
    config.frequency = 12_000_000; // 12 MHz

    let spi = Spi::new(
        r.peripheral,
        r.clk,
        r.mosi,
        r.miso,
        r.mosi_dma,
        r.miso_dma,
        config,
    );
    let cs = Output::new(r.cs, Level::High);
    let led = Output::new(r.led, Level::Low);

    let set_freq_cb = move |spi: &mut Spi<'_, SPI0, embassy_rp::spi::Async>, freq| {
        spi.set_frequency(freq);
    };

    let serprog = serprog::Serprog::new(spi, cs, led, class, Some(set_freq_cb));
    serprog.run_loop().await
}

#[panic_handler]
fn panic(info: &PanicInfo) -> ! {
    // Print out the panic info
    log::error!("Panic occurred: {:?}", info);

    // Reboot the system
    SCB::sys_reset();
}
