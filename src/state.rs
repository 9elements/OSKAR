use crate::layouts::{KeyLayout, LAYOUT_1, LAYOUT_2, LAYOUT_3};
use embassy_rp::gpio::{Input, Level, Pull};
use embassy_sync::watch::Watch;
use embassy_sync::blocking_mutex::raw::CriticalSectionRawMutex;
use embassy_futures::select::select_array;
use smart_leds::RGB8;

/// System state combining device mode and keyboard layout
/// Each state maps to a layout configuration with keys and LED color
#[derive(Copy, Clone, PartialEq, Debug)]
pub enum State {
    KeyboardLayout1,
    KeyboardLayout2,
    KeyboardLayout3,
}

impl State {
    /// Returns the keyboard layout for this state
    pub fn layout(&self) -> &'static KeyLayout {
        match self {
            State::KeyboardLayout1 => &LAYOUT_1,
            State::KeyboardLayout2 => &LAYOUT_2,
            State::KeyboardLayout3 => &LAYOUT_3,
        }
    }

    /// Returns whether LEDs should be animating in this state
    pub fn leds_active(&self) -> bool {
        self.layout().active
    }

    /// Returns the mode indicator LED color for this state
    pub fn mode_color(&self) -> RGB8 {
        self.layout().led_color
    }
}

/// Global state channel - single source of truth for system state
pub static DEVICE_STATE: Watch<CriticalSectionRawMutex, State, 2> = Watch::new();

/// Determines state based on physical switch positions
fn select_state(switch1: Level, switch2: Level) -> State {
    match (switch1, switch2) {
        (_, Level::Low) => State::KeyboardLayout1,         
        (Level::High, Level::High) => State::KeyboardLayout2,  
        (Level::Low, Level::High) => State::KeyboardLayout3,   
    }
}

/// State manager task - monitors switches and broadcasts state changes
#[embassy_executor::task]
pub async fn state_manager_task(r: crate::SelectorResources) -> ! {
    let mut switch1 = Input::new(r.switch1, Pull::Up);
    let mut switch2 = Input::new(r.switch2, Pull::Up);

    // Set initial state
    let mut current_state = select_state(switch1.get_level(), switch2.get_level());
    DEVICE_STATE.sender().send(current_state);

    // Monitor switches for changes
    loop {
        // Wait for any switch to change
        select_array([
            switch1.wait_for_any_edge(),
            switch2.wait_for_any_edge(),
        ]).await;

        // Small debounce delay
        embassy_time::Timer::after(embassy_time::Duration::from_millis(50)).await;

        // Read new switch positions and determine state
        let new_state = select_state(switch1.get_level(), switch2.get_level());

        // Only broadcast if state actually changed
        if new_state != current_state {
            current_state = new_state;
            DEVICE_STATE.sender().send(new_state);
        }
    }
}
