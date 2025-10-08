use crate::hid::KeyType;

pub struct KeyLayout {
    pub encoder_left: KeyType,
    pub encoder_right: KeyType,
    pub encoder_button: KeyType,
    pub key1: KeyType,
    pub key2: KeyType,
    pub key3: KeyType,
}
