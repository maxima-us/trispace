//! Tri-Space Ambient Engine - Entry Point

//! this implements #22, #25, #26, #28 from our feature list

mod types;
mod dsp;
mod processors;
mod routing;  // ← ADDED THIS
mod multiband;  // ← ADD THIS LINE
mod envelope;  // ← ADD THIS
mod engine;
mod ui;

use anyhow::Result;

fn main() -> Result<(), eframe::Error> {
    // Enable FTZ/DAZ for performance
    types::ftz_daz();
    
    // Set up panic handler for better error messages
    std::panic::set_hook(Box::new(|panic_info| {
        eprintln!("Application panicked: {}", panic_info);
    }));
    
    // Run GUI
    ui::run()
}