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
use embassy_executor::{InterruptExecutor, Spawner};
use embassy_futures::select::select_array;
use embassy_rp::bind_interrupts;
use embassy_rp::flash::{Async, Flash};
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::interrupt;
use embassy_rp::interrupt::{InterruptExt, Priority};
use embassy_rp::peripherals::{self, PIO0, SPI0, USB};
use embassy_rp::pio::InterruptHandler as PIOInterruptHandler;
use embassy_rp::spi::{Config as SpiConfig, Spi};
use embassy_rp::usb::{Driver, InterruptHandler as USBInterruptHandler};
use embassy_rp::watchdog::Watchdog;
use embassy_usb::class::cdc_acm::{CdcAcmClass, State as CdcAcmState};
use embassy_usb::class::hid::{HidReaderWriter, State as Hid_State};
use usbd_hid::descriptor::{KeyboardReport, SerializedDescriptor};

// use embassy_usb::driver::EndpointError;
use embassy_usb::{Config as UsbConfig, UsbDevice};
use heapless::String;
use static_cell::StaticCell;
use ufmt::uwrite;

mod hid;
mod led;
mod uart;
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

    hid: HidResources{
        key1: PIN_19,
        key2: PIN_20,
        key3: PIN_21,
        encoder_button: PIN_13,
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

#[derive(Clone, Copy, Debug)]
pub enum DeviceMode {
    Keyboard,
    Picoprog,
    Universal,
}

// According to Serial Flasher Protocol Specification - version 1
const FLASH_SIZE: usize = 2 * 1024 * 1024;

static EXECUTOR_HIGH: InterruptExecutor = InterruptExecutor::new();

#[interrupt]
unsafe fn SWI_IRQ_1() {
    unsafe { EXECUTOR_HIGH.on_interrupt() }
}

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    let p: embassy_rp::Peripherals = embassy_rp::init(Default::default());
    let r: AssignedResources = split_resources!(p);
    let driver = Driver::new(p.USB, Irqs);

    let selector_keyboard: Input<'_> = Input::new(r.selector_switch.selector_kb, Pull::None);
    let selector_picoprog: Input<'_> = Input::new(r.selector_switch.selector_picocprog, Pull::None);
    let watchdog = Watchdog::new(p.WATCHDOG);

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

    let mut builder = {
        static CONFIG_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static BOS_DESCRIPTOR: StaticCell<[u8; 256]> = StaticCell::new();
        static CONTROL_BUF: StaticCell<[u8; 64]> = StaticCell::new();

        let builder = embassy_usb::Builder::new(
            driver,
            config,
            CONFIG_DESCRIPTOR.init([0; 256]),
            BOS_DESCRIPTOR.init([0; 256]),
            &mut [], // no msos descriptors
            CONTROL_BUF.init([0; 64]),
        );
        builder
    };

    spawner.spawn(led::led_task(r.led, mode)).unwrap();

    if !(matches!(mode, DeviceMode::Keyboard)) {
        let uart_class = {
            static STATE: StaticCell<CdcAcmState> = StaticCell::new();
            let state = STATE.init(CdcAcmState::new());
            CdcAcmClass::new(&mut builder, state, 64)
        };

        let serprog_class = {
            static STATE: StaticCell<CdcAcmState> = StaticCell::new();
            let state = STATE.init(CdcAcmState::new());
            CdcAcmClass::new(&mut builder, state, 64)
        };

        spawner.spawn(uart::uart_task(uart_class, r.uart)).unwrap();
        spawner.spawn(serprog_task(serprog_class, r.spi)).unwrap();
    }

    if !(matches!(mode, DeviceMode::Picoprog)) {
        let hid_class: HidReaderWriter<'_, Driver<'_, USB>, 1, 8> = {
            static STATE: StaticCell<Hid_State> = StaticCell::new();
            let state = STATE.init(Hid_State::new());

            let config = embassy_usb::class::hid::Config {
                report_descriptor: KeyboardReport::desc(),
                request_handler: None,
                poll_ms: 60,
                max_packet_size: 64,
            };

            HidReaderWriter::new(&mut builder, state, config)
        };

        interrupt::SWI_IRQ_1.set_priority(Priority::P1);
        let spawner_high = EXECUTOR_HIGH.start(interrupt::SWI_IRQ_1);
        spawner_high.spawn(hid::hid_task(hid_class, r.hid)).unwrap();
    }

    let usb = builder.build();
    // We can't really recover here so just unwrap
    spawner.spawn(usb_task(usb)).unwrap();
    spawner
        .spawn(selector_watchdog_task(
            watchdog,
            selector_keyboard,
            selector_picoprog,
        ))
        .unwrap();

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
async fn selector_watchdog_task(
    mut watchdog: Watchdog,
    mut selector_keyboard: Input<'static>,
    mut selector_picoprog: Input<'static>,
) {
    let (_, _) = select_array([
        selector_keyboard.wait_for_any_edge(),
        selector_picoprog.wait_for_any_edge(),
    ])
    .await;

    watchdog.trigger_reset();
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
