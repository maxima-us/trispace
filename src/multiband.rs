//! Spectral (Multiband) Delay Bus
//! 
//! 8-band parallel delay with independent time/feedback/damping per band.
//! Uses SVF bandpass filters and power-normalized recombination.

use crate::dsp::{DcBlockerExact, OnePoleLP};
use crate::types::{soft_clip, BLOCK, SR};

const NUM_BANDS: usize = 8;

pub struct SpectralDelay {
    bands: [BandDelay; NUM_BANDS],
    band_freqs: [f32; NUM_BANDS],
}

struct BandDelay {
    // Filtering
    bp_filter: BandpassSvf,
    damper: OnePoleLP,
    dc_blocker: DcBlockerExact,
    
    // Delay line (stereo)
    buffer_l: Vec<f32>,
    buffer_r: Vec<f32>,
    mask: usize,
    w_idx: usize,
    
    // Parameters (smoothed at block rate)
    time_current: f32,
    time_target: f32,
    fb_current: f32,
    fb_target: f32,
    smooth_alpha: f32,
}

/// Zero-delay feedback SVF configured as bandpass
struct BandpassSvf {
    ic1eq: f32,  // Integrator state 1
    ic2eq: f32,  // Integrator state 2
    g: f32,      // tan(π*fc/fs)
    k: f32,      // 1/Q (resonance)
}

impl BandpassSvf {
    fn new(fc: f32, q: f32) -> Self {
        let g = (std::f32::consts::PI * fc / SR as f32).tan();
        let k = 1.0 / q;
        Self {
            ic1eq: 0.0,
            ic2eq: 0.0,
            g,
            k,
        }
    }
    
    fn set_freq_q(&mut self, fc: f32, q: f32) {
        self.g = (std::f32::consts::PI * fc / SR as f32).tan();
        self.k = 1.0 / q;
    }
    
    #[inline]
    fn process(&mut self, x: f32) -> f32 {
        // TPT SVF: solve for v1 (bandpass output)
        let v1 = (self.ic1eq + self.g * (x - self.ic2eq)) / (1.0 + self.g * (self.g + self.k));
        let v2 = self.ic2eq + self.g * v1;
        
        // Update integrator states
        self.ic1eq = 2.0 * v1 - self.ic1eq;
        self.ic2eq = 2.0 * v2 - self.ic2eq;
        
        v1 // Bandpass output
    }
    
    #[allow(dead_code)]
    fn reset(&mut self) {
        self.ic1eq = 0.0;
        self.ic2eq = 0.0;
    }
}

impl BandDelay {
    fn new(center_freq: f32, q: f32, max_delay_ms: f32) -> Self {
        // Round up to next power of 2 for efficient masking
        let max_samps = (max_delay_ms * 0.001 * SR as f32) as usize;
        let pow2 = (max_samps as f32).log2().ceil() as u32;
        let len = 1usize << pow2;
        
        Self {
            bp_filter: BandpassSvf::new(center_freq, q),
            damper: OnePoleLP::new(4000.0, SR as f32),
            dc_blocker: DcBlockerExact::new(10.0, SR as f32),
            buffer_l: vec![0.0; len],
            buffer_r: vec![0.0; len],
            mask: len - 1,
            w_idx: 0,
            time_current: 200.0 * 0.001 * SR as f32,
            time_target: 200.0 * 0.001 * SR as f32,
            fb_current: 0.5,
            fb_target: 0.5,
            smooth_alpha: 0.001,
        }
    }
    
    fn set_time_ms(&mut self, ms: f32) {
        self.time_target = (ms * 0.001 * SR as f32).clamp(1.0, self.mask as f32);
    }
    
    fn set_feedback(&mut self, fb: f32) {
        self.fb_target = fb.clamp(0.0, 0.65);
    }
    
    fn set_damping_hz(&mut self, fc: f32) {
        self.damper.set_cutoff(fc, SR as f32);
    }
    
    fn process_stereo(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        frames: usize,
    ) {
        for n in 0..frames {
            // Smooth parameters at audio rate
            self.time_current += self.smooth_alpha * (self.time_target - self.time_current);
            self.fb_current += self.smooth_alpha * (self.fb_target - self.fb_current);
            
            // Calculate read position with wrapping
            let delay_samps = self.time_current;
            let read_pos = (self.w_idx as f32 - delay_samps + (self.mask + 1) as f32)
                % ((self.mask + 1) as f32);
            
            // Linear interpolation read
            let i0 = read_pos as usize & self.mask;
            let i1 = (i0 + 1) & self.mask;
            let frac = read_pos - read_pos.floor();
            
            let delayed_l = self.buffer_l[i0] + frac * (self.buffer_l[i1] - self.buffer_l[i0]);
            let delayed_r = self.buffer_r[i0] + frac * (self.buffer_r[i1] - self.buffer_r[i0]);
            
            // Damping (HF roll-off)
            let damped_l = self.damper.process(delayed_l);
            let damped_r = self.damper.process(delayed_r);
            
            // DC blocking
            let clean_l = self.dc_blocker.process(damped_l);
            let clean_r = self.dc_blocker.process(damped_r);
            
            // Output
            out_l[n] = clean_l;
            out_r[n] = clean_r;
            
            // Feedback with soft clipping to prevent runaway
            let fb_l = soft_clip(in_l[n] + self.fb_current * clean_l, 0.9);
            let fb_r = soft_clip(in_r[n] + self.fb_current * clean_r, 0.9);
            
            self.buffer_l[self.w_idx] = fb_l;
            self.buffer_r[self.w_idx] = fb_r;
            
            self.w_idx = (self.w_idx + 1) & self.mask;
        }
    }
}

impl SpectralDelay {
    pub fn new() -> Self {
        // Logarithmically spaced bands covering musical spectrum
        let band_freqs = [
            100.0,    // Sub/low bass
            200.0,    // Bass
            400.0,    // Low mids
            800.0,    // Mids
            1600.0,   // Upper mids
            3200.0,   // Presence
            6400.0,   // Brilliance
            12000.0,  // Air
        ];
        
        let q = 1.5; // Moderate overlap between bands
        let max_delay_ms = 2000.0;
        
        let bands = std::array::from_fn(|i| {
            BandDelay::new(band_freqs[i], q, max_delay_ms)
        });
        
        Self {
            bands,
            band_freqs,
        }
    }
    
    pub fn process(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        frames: usize,
    ) {
        // Clear output
        for i in 0..frames {
            out_l[i] = 0.0;
            out_r[i] = 0.0;
        }
        
        // Temporary buffers for per-band processing
        let mut filtered_l = [0.0f32; BLOCK];
        let mut filtered_r = [0.0f32; BLOCK];
        let mut band_out_l = [0.0f32; BLOCK];
        let mut band_out_r = [0.0f32; BLOCK];
        
        // Power-normalized gain (maintains RMS when summing)
        // Previously 1.0/sqrt(8) ~= 0.35, which resulted in significant volume drop
        // because band outputs are partially correlated at crossover points.
        // 0.60 is a safer heuristic for 8 overlapped bands to maintain perceived loudness.
        let gain = 0.60;
        
        for band in &mut self.bands {
            // Split input into this frequency band
            for n in 0..frames {
                filtered_l[n] = band.bp_filter.process(in_l[n]);
                filtered_r[n] = band.bp_filter.process(in_r[n]);
            }
            
            // Process band delay
            band.process_stereo(
                &filtered_l,
                &filtered_r,
                &mut band_out_l,
                &mut band_out_r,
                frames,
            );
            
            // Sum to output with power normalization
            for n in 0..frames {
                out_l[n] += band_out_l[n] * gain;
                out_r[n] += band_out_r[n] * gain;
            }
        }
    }
    
    // === Public parameter setters ===
    
    pub fn set_band_time(&mut self, band_idx: usize, ms: f32) {
        if band_idx < NUM_BANDS {
            self.bands[band_idx].set_time_ms(ms);
        }
    }
    
    pub fn set_band_feedback(&mut self, band_idx: usize, fb: f32) {
        if band_idx < NUM_BANDS {
            self.bands[band_idx].set_feedback(fb);
        }
    }
    
    pub fn set_band_damping(&mut self, band_idx: usize, fc: f32) {
        if band_idx < NUM_BANDS {
            self.bands[band_idx].set_damping_hz(fc);
        }
    }

    /// Set all band times from external array (called from routing with macro-shaped values)
    pub fn set_all_band_times(&mut self, times_ms: &[f32; 8]) {
        for (i, &time) in times_ms.iter().enumerate() {
            self.bands[i].set_time_ms(time);
        }
    }
    
    /// Set all band feedbacks from external array
    pub fn set_all_band_feedbacks(&mut self, feedbacks: &[f32; 8]) {
        for (i, &fb) in feedbacks.iter().enumerate() {
            self.bands[i].set_feedback(fb);
        }
    }
    
    /// Legacy: Set all band times proportionally (kept for backward compatibility)
    pub fn set_global_time_scale(&mut self, scale: f32) {
        let base_times = [150.0, 200.0, 250.0, 300.0, 350.0, 400.0, 450.0, 500.0];
        for (i, &base_ms) in base_times.iter().enumerate() {
            self.bands[i].set_time_ms(base_ms * scale);
        }
    }
    
    /// Legacy: Set all band feedbacks to same value
    pub fn set_global_feedback(&mut self, fb: f32) {
        for band in &mut self.bands {
            band.set_feedback(fb);
        }
    }

    // === Getters for UI ===
    
    pub fn get_band_count() -> usize {
        NUM_BANDS
    }
    
    pub fn get_band_freq(&self, band_idx: usize) -> f32 {
        if band_idx < NUM_BANDS {
            self.band_freqs[band_idx]
        } else {
            0.0
        }
    }
}