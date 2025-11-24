# Open Source Keyboard Appliance powered by Rust

OSKAR is a firmware for the Raspberry Pi Pico that combines the 9elements picoprog project and additional HID code into a firmware for our multi-feature Macro-Keypad.

## Prerequisites

Before you can compile and use OSKAR, you need to install the following dependencies:

- Rust and Cargo: Follow the instructions on the [official Rust website](https://www.rust-lang.org/tools/install) to install Rust and Cargo.
- Install flip-link and elf2uf2

```sh
# Only Linux
sudo apt install build-essential libudev-dev pkg-config

# All systems
cargo install flip-link elf2uf2-rs
```

## Compiling the Firmware

To compile the firmware, follow these steps:

1. Clone the repository:

```sh
git clone https://github.com/9elements/OSKAR.git
cd OSKAR
```

2. Build the firmware:

```sh
cargo run --release
```

3. The compiled binary will be located in the `target/thumbv6m-none-eabi/release` directory.

## Flashing the Firmware

To flash the firmware onto the Raspberry Pi Pico, follow these steps:

1. Connect the Raspberry Pi Pico to your computer while holding the BOOTSEL button. This will put the Pico into USB mass storage mode.

2. Copy the UF2 file to the Pico:

```sh
# Linux
cp target/thumbv6m-none-eabi/release/oskar.uf2 /path/to/pi/volume

# macOS
cp target/thumbv6m-none-eabi/release/oskar.uf2 /Volumes/RPI-RP2/
```

3. The Pico will automatically reboot and start running the new firmware.

## Usage

### Layout Selection

OSKAR features a state machine that manages three different keyboard layouts. The device uses two selector switches (PIN_16 and PIN_17) to cycle between layouts. Each layout has its own LED color indicator and key configuration.

#### Pre-Configured Layouts

The firmware ships with three pre-configured layouts (fully customizable in `src/layouts.rs`):

**Layout 1 - OFF (Black LED)**

- Keyboard input disabled
- All keys inactive
- LED off (RGB: 0, 0, 0)

**Layout 2 - Navigation (Red LED)**

- Encoder: Volume control (rotate) + Mute (press)
- Key 1: Left Arrow
- Key 2: Spacebar
- Key 3: Right Arrow
- LED color: Red (RGB: 10, 0, 0)
- Ideal for presentations or video playback

**Layout 3 - Media Control (Purple LED)**

- Encoder: Volume control (rotate) + Mute (press)
- Key 1: Previous Track
- Key 2: Play/Pause
- Key 3: Next Track
- LED color: Purple (RGB: 14, 4, 13)
- Perfect for music and video control

### Customizing Layouts

Layouts are defined in `src/layouts.rs`. Each layout is a constant struct of type `KeyLayout`:

```rust
pub const LAYOUT_2: KeyLayout = KeyLayout {
    encoder_left: KeyType::Media(MediaKey::VolumeDecrement),
    encoder_right: KeyType::Media(MediaKey::VolumeIncrement),
    encoder_button: KeyType::Media(MediaKey::Mute),
    key1: KeyType::Keycode(KeyboardUsage::KeyboardLeftArrow),
    key2: KeyType::Keycode(KeyboardUsage::KeyboardSpacebar),
    key3: KeyType::Keycode(KeyboardUsage::KeyboardRightArrow),
    led_color: RGB8::new(10, 0, 0), // Red
    active: true,
};
```

To customize a layout:
1. Open `src/layouts.rs`
2. Modify any of the three `LAYOUT_X` constants
3. Change key assignments using `KeyType::Keycode()` or `KeyType::Media()`
4. Adjust LED colors with `RGB8::new(red, green, blue)` (values 0-255)
5. Rebuild and flash the firmware

**Example - Function Key Layout:**
```rust
pub const LAYOUT_2: KeyLayout = KeyLayout {
    encoder_left: KeyType::Media(MediaKey::VolumeDecrement),
    encoder_right: KeyType::Media(MediaKey::VolumeIncrement),
    encoder_button: KeyType::Media(MediaKey::Mute),
    key1: KeyType::Keycode(KeyboardUsage::KeyboardF10),
    key2: KeyType::Keycode(KeyboardUsage::KeyboardF11),
    key3: KeyType::Keycode(KeyboardUsage::KeyboardF12),
    led_color: RGB8::new(0, 10, 0), // Green
    active: true,
};
```

Available key types can be found in the `KeyboardUsage` enum (for regular keys) and `MediaKey` enum (for media controls) from the `usbd_hid` crate.

### Serial (picocom or combined mode)

Once the firmware is running, you can use any terminal program to communicate with the UART and SPI peripherals via USB. The device will appear as a USB CDC (Communications Device Class) device. Currently `/dev/ttyACM0` (macOS: `/dev/tty.usbmodemOSFC20241`) is a debug console that prints information about the Pico's current operation.

### UART Communication (picocom or combined mode)

To communicate with the UART peripheral, open the corresponding serial port (e.g., `/dev/ttyACM1` on Linux, `/dev/tty.usbmodemOSFC20243` on macOS) with your terminal program. For now the Baud is fixed at 115200 but can be changed in code. Dynamic reconfiguration is still planned.

### Using Flashrom or Flashprog (picocom or combined mode)

To interact with the Raspberry Pi Pico for reading and writing SPI flash chips, you can use tools like `flashrom` or `flashprog`. These tools support the `serprog` protocol, which allows communication over a serial interface.

1. Install `flashrom` or `flashprog` e.g.:

```sh
# Debian or Debian-based Linux distributions
sudo apt-get install flashrom

# macOS with Homebrew
brew install flashrom
```

2. Use the following command to read from the SPI flash chip:

```sh
flashrom -p serprog:dev=/dev/ttyACM2 -r backup.bin
```

This command reads the contents of the SPI flash chip and saves it to `backup.bin`.

3. Use the following command to write to the SPI flash chip:

```sh
flashrom -p serprog:dev=/dev/ttyACM2 -w firmware.bin
```

This command writes the contents of `firmware.bin` to the SPI flash chip.

Make sure to replace `/dev/ttyACM2` with the correct serial port if your device is connected to a different port or if you're on a different operating system (on macOS it will be `/dev/tty.usbmodemOSFC20245`).


## License

This project is licensed under the Apache 2.0 License. See the [LICENSE](LICENSE) file for details.
