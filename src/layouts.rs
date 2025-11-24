use crate::hid::KeyType;
use usbd_hid::descriptor::*;
use smart_leds::RGB8;

#[derive(PartialEq)]
pub struct KeyLayout {
    pub encoder_left: KeyType,
    pub encoder_right: KeyType,
    pub encoder_button: KeyType,
    pub key1: KeyType,
    pub key2: KeyType,
    pub key3: KeyType,
    pub led_color: RGB8,
    pub active: bool, // Whether keyboard input is active
}

// Dummy key type for inactive layouts
const NONE: KeyType = KeyType::Keycode(KeyboardUsage::KeyboardErrorRollOver);

pub const LAYOUT_1: KeyLayout = KeyLayout {
    encoder_left: NONE,
    encoder_right: NONE,
    encoder_button: NONE,
    key1: NONE,
    key2: NONE,
    key3: NONE,
    led_color: RGB8::new(0, 0, 0), // Off (black)
    active: false, // Keyboard OFF
};

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

pub const LAYOUT_3: KeyLayout = KeyLayout {
    encoder_left: KeyType::Media(MediaKey::VolumeDecrement),
    encoder_right: KeyType::Media(MediaKey::VolumeIncrement),
    encoder_button: KeyType::Media(MediaKey::Mute),
    key1: KeyType::Media(MediaKey::PrevTrack),
    key2: KeyType::Media(MediaKey::PlayPause),
    key3: KeyType::Media(MediaKey::NextTrack),
    led_color: RGB8::new(14, 4, 13), // Purple
    active: true,
};            