//! 3-Band Envelope Follower for Dynamic Space Control
//! 
//! Tracks energy in low/mid/high bands and generates modulation signals
//! for making the reverb/delay "breathe" with the input.

use crate::types::SR;

const NUM_BANDS: usize = 3;

/// 3-band envelope follower with independent attack/release per band
pub struct EnvelopeFollower {
    // Band splitting (simple 2-pole crossover)
    crossover: SimpleCrossover3,
    
    // RMS detectors per band
    rms_states: [RmsDetector; NUM_BANDS],
    
    // Envelope outputs (0.0 to 1.0+)
    envelopes: [f32; NUM_BANDS],
    
    // Combined envelope (weighted sum)
    combined: f32,
}

/// Simple 3-band crossover using 1-pole filters
struct SimpleCrossover3 {
    // Low/mid split at ~250Hz
    lp1_z: f32,
    lp1_coeff: f32,
    
    // Mid/high split at ~2500Hz  
    lp2_z: f32,
    lp2_coeff: f32,
}

impl SimpleCrossover3 {
    fn new() -> Self {
        // One-pole LP coefficient: a = exp(-2π * fc / fs)
        let lp1_coeff = (-2.0 * std::f32::consts::PI * 250.0 / SR as f32).exp();
        let lp2_coeff = (-2.0 * std::f32::consts::PI * 2500.0 / SR as f32).exp();
        
        Self {
            lp1_z: 0.0,
            lp1_coeff,
            lp2_z: 0.0,
            lp2_coeff,
        }
    }
    
    /// Split mono input into 3 bands [low, mid, high]
    #[inline]
    fn process(&mut self, x: f32) -> [f32; 3] {
        // Low band: LP at 250Hz
        self.lp1_z = self.lp1_coeff * self.lp1_z + (1.0 - self.lp1_coeff) * x;
        let low = self.lp1_z;
        
        // Mid+High: complement of low
        let mid_high = x - low;
        
        // Mid band: LP the mid+high at 2500Hz
        self.lp2_z = self.lp2_coeff * self.lp2_z + (1.0 - self.lp2_coeff) * mid_high;
        let mid = self.lp2_z;
        
        // High band: complement of mid (within mid+high)
        let high = mid_high - mid;
        
        [low, mid, high]
    }
}

/// RMS detector with attack/release (two-pole peak/RMS hybrid)
struct RmsDetector {
    rms_state: f32,
    attack_coeff: f32,
    release_coeff: f32,
}

impl RmsDetector {
    fn new(attack_ms: f32, release_ms: f32) -> Self {
        let attack_coeff = Self::time_to_coeff(attack_ms);
        let release_coeff = Self::time_to_coeff(release_ms);
        
        Self {
            rms_state: 0.0,
            attack_coeff,
            release_coeff,
        }
    }
    
    /// Converts milliseconds to 1-pole coefficient using T60 standard.
    /// Previous implementation used tau=time, which is correct for 'tau' 
    /// but misleading for UI labels (result was ~7x slower than expected).
    /// T60 (decay to -60dB) implies coeff = exp(-ln(1000) / samples).
    fn time_to_coeff(time_ms: f32) -> f32 {
        let time_samps = time_ms * 0.001 * SR as f32;
        // -ln(1000) ≈ -6.907755
        (-6.907755 / time_samps.max(1.0)).exp()
    }
    
    fn set_attack(&mut self, attack_ms: f32) {
        self.attack_coeff = Self::time_to_coeff(attack_ms);
    }
    
    fn set_release(&mut self, release_ms: f32) {
        self.release_coeff = Self::time_to_coeff(release_ms);
    }
    
    /// Process one sample: r ← α*r + (1-α)*x², e = √(r + ε)
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        let x_sq = x * x;
        
        // Two-pole hybrid: fast attack, slow release
        let coeff = if x_sq > self.rms_state {
            self.attack_coeff
        } else {
            self.release_coeff
        };
        
        // Update RMS² state
        self.rms_state = coeff * self.rms_state + (1.0 - coeff) * x_sq;
        
        // Return envelope with epsilon
        (self.rms_state + 1e-12).sqrt()
    }
}

impl EnvelopeFollower {
    pub fn new() -> Self {
        Self {
            crossover: SimpleCrossover3::new(),
            rms_states: [
                RmsDetector::new(10.0, 100.0),  // Low
                RmsDetector::new(5.0, 50.0),    // Mid
                RmsDetector::new(2.0, 25.0),    // High
            ],
            envelopes: [0.0; NUM_BANDS],
            combined: 0.0,
        }
    }
    
    /// Process one sample, update internal state
    #[inline]
    pub fn process_sample(&mut self, x: f32) {
        let bands = self.crossover.process(x);
        
        for i in 0..NUM_BANDS {
            self.envelopes[i] = self.rms_states[i].process(bands[i]);
        }
        
        // Weighted combination (favor mids for musical response)
        self.combined = 0.2 * self.envelopes[0]
                      + 0.5 * self.envelopes[1]
                      + 0.3 * self.envelopes[2];
    }
    
    pub fn get_combined(&self) -> f32 {
        self.combined
    }
    
    #[allow(dead_code)]
    pub fn get_bands(&self) -> [f32; NUM_BANDS] {
        self.envelopes
    }
    
    pub fn set_band_attack(&mut self, band: usize, attack_ms: f32) {
        if band < NUM_BANDS {
            self.rms_states[band].set_attack(attack_ms);
        }
    }
    
    pub fn set_band_release(&mut self, band: usize, release_ms: f32) {
        if band < NUM_BANDS {
            self.rms_states[band].set_release(release_ms);
        }
    }
}

/// Mapping curve: envelope → modulation with minimum and curve shaping
#[inline]
pub fn envelope_to_modulation(envelope: f32, depth: f32, minimum: f32, curve: f32) -> f32 {
    let scaled = envelope * depth;
    
    let shaped = if curve < -0.01 {
        // Exponential: more sensitive to quiet signals
        scaled.powf(1.0 + curve.abs())
    } else if curve > 0.01 {
        // Compressed: less sensitive to loud signals
        1.0 - (1.0 - scaled.min(1.0)).powf(1.0 + curve)
    } else {
        scaled
    };
    
    minimum + shaped * (1.0 - minimum)
}