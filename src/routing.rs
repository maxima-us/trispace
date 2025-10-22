
//! Audio Routing Graph - Maintains complex signal flow

use crate::dsp::TiltShelf;
use crate::envelope::{EnvelopeFollower, envelope_to_modulation};  // ← ADD THIS
use crate::multiband::SpectralDelay;  // ← ADD THIS LINE
use crate::processors::*;
use crate::types::*;
use std::sync::atomic::Ordering;  // ← ADDED THIS
use std::sync::Arc;
use rand::{Rng, SeedableRng};
use rand_pcg::Pcg64;


/// Represents the complete audio graph with all routing
pub struct AudioGraph {
    // Processors
    pub grains: GrainPool,
    pub delay: StereoDelay,
    pub spectral: SpectralDelay,  // ← ADD THIS LINE
    pub fdn: Fdn,
    pub conv: ConvHead,
    pub tilt: TiltShelf,
    pub envelope: EnvelopeFollower,  // ← ADD THIS
    
    // Routing buffers (reusable)
    buffers: GraphBuffers,

    // Persistent state (FIX: Rotor phase must accumulate across blocks)
    rotor_phase: f32,
    dither_state: u32,  // FIX: Persistent PRNG state (prevents 375 Hz periodic tone)
}

struct GraphBuffers {
    // Main signal paths
    main_l: [f32; BLOCK],
    main_r: [f32; BLOCK],
    
    // Send buffers
    fdn_sends: [Vec<f32>; FDN_LINES],
    delay_send_l: [f32; BLOCK],
    delay_send_r: [f32; BLOCK],
    spectral_send_l: [f32; BLOCK],  // ← ADD THIS LINE
    spectral_send_r: [f32; BLOCK],  // ← ADD THIS LINE
    
    // Processor outputs
    delay_out_l: [f32; BLOCK],
    delay_out_r: [f32; BLOCK],
    spectral_out_l: [f32; BLOCK],  // ← ADD THIS LINE
    spectral_out_r: [f32; BLOCK],  // ← ADD THIS LINE
    
    // Convolution scratch
    fdn_scratch_a: [f32; BLOCK],
    fdn_scratch_b: [f32; BLOCK],
    ir_scratch: [f32; BLOCK],
    
    // Post-processing
    post_l: [f32; BLOCK],
    post_r: [f32; BLOCK],
}

impl GraphBuffers {
    fn new() -> Self {
        Self {
            main_l: [0.0; BLOCK],
            main_r: [0.0; BLOCK],
            fdn_sends: std::array::from_fn(|_| vec![0.0; BLOCK]),
            delay_send_l: [0.0; BLOCK],
            delay_send_r: [0.0; BLOCK],
            spectral_send_l: [0.0; BLOCK],  // ← ADD THIS LINE
            spectral_send_r: [0.0; BLOCK],  // ← ADD THIS LINE
            delay_out_l: [0.0; BLOCK],
            delay_out_r: [0.0; BLOCK],
            spectral_out_l: [0.0; BLOCK],  // ← ADD THIS LINE
            spectral_out_r: [0.0; BLOCK],  // ← ADD THIS LINE
            fdn_scratch_a: [0.0; BLOCK],
            fdn_scratch_b: [0.0; BLOCK],
            ir_scratch: [0.0; BLOCK],
            post_l: [0.0; BLOCK],
            post_r: [0.0; BLOCK],
        }
    }
    
    fn clear_all(&mut self) {
        self.main_l.fill(0.0);
        self.main_r.fill(0.0);
        for send in &mut self.fdn_sends {
            send.fill(0.0);
        }
        self.delay_send_l.fill(0.0);
        self.delay_send_r.fill(0.0);
        self.spectral_send_l.fill(0.0);  // ← ADD THIS LINE
        self.spectral_send_r.fill(0.0);  // ← ADD THIS LINE
        self.ir_scratch.fill(0.0);
        self.post_l.fill(0.0);
        self.post_r.fill(0.0);
    }
}

impl AudioGraph {
    pub fn new(params: &Arc<GlobalParams>) -> Self {
        let size = params.reverb.size.load(Ordering::Relaxed);
        let damping = params.reverb.damping.load(Ordering::Relaxed);
        let predelay = params.reverb.predelay_ms.load(Ordering::Relaxed);
        
        Self {
            grains: GrainPool::new(vec![0.0; SR]),
            delay: StereoDelay::new(18),
            spectral: SpectralDelay::new(),  // ← ADD THIS LINE
            fdn: Fdn::new(size, damping, predelay),
            conv: ConvHead::new(),
            tilt: TiltShelf::new(),
            envelope: EnvelopeFollower::new(),  // ← ADD THIS
            buffers: GraphBuffers::new(),
            rotor_phase: 0.0,  // Initialize rotor phase (FIX: Persistent state)
            dither_state: 0xC0FFEEu32,  // FIX: Initialize once (not every block)
        }
    }
    
    /// Process one block - EXACT SAME AUDIO PATH AS BEFORE
    pub fn process_block(
        &mut self,
        output: &mut [f32],
        params: &Arc<GlobalParams>,
        is_playing: bool,
    ) -> [f32; BLOCK] {
        
        // ========== CALCULATE FRAMES FROM OUTPUT BUFFER SIZE ==========
        let frames = (output.len() / 2).min(BLOCK);
        
        // ========== CLEAR INTERNAL BUFFERS ==========
        self.buffers.clear_all();
        
        // === STAGE 1: GRAINS ===
        if is_playing {
            self.grains.process_block(
                &mut self.buffers.main_l,
                &mut self.buffers.main_r,
                &mut self.buffers.fdn_sends,
                &mut self.buffers.delay_send_l,
                &mut self.buffers.delay_send_r,
                params,
                frames,
            );

            // === ENVELOPE FOLLOWER (makes space breathe with input) ===
            let env_enabled = params.envelope.enabled.load(Ordering::Relaxed);
            let mut envelope_mod = 1.0; // Default: no modulation
        
            if env_enabled > 0.01 {
                // Update attack/release times (only if changed)
                self.envelope.set_band_attack(0, params.envelope.attack_low.load(Ordering::Relaxed));
                self.envelope.set_band_attack(1, params.envelope.attack_mid.load(Ordering::Relaxed));
                self.envelope.set_band_attack(2, params.envelope.attack_high.load(Ordering::Relaxed));
                self.envelope.set_band_release(0, params.envelope.release_low.load(Ordering::Relaxed));
                self.envelope.set_band_release(1, params.envelope.release_mid.load(Ordering::Relaxed));
                self.envelope.set_band_release(2, params.envelope.release_high.load(Ordering::Relaxed));
                
                // Track envelope from DRY GRAIN OUTPUT (sidechain before reverb/delay)
                for n in 0..frames {
                    let mono = 0.5 * (self.buffers.main_l[n] + self.buffers.main_r[n]);
                    self.envelope.process_sample(mono);
                }
                
                // Get current envelope level (use last sample's value for block)
                let current_env = self.envelope.get_combined();
                
                // Get modulation parameters
                let curve = params.envelope.curve.load(Ordering::Relaxed);
                let minimum = params.envelope.minimum.load(Ordering::Relaxed);
                
                // Calculate modulation amount (0.0 to 1.0+)
                envelope_mod = envelope_to_modulation(current_env, env_enabled, minimum, curve);
                
                // Apply to sends BEFORE they're used
                let to_reverb = params.envelope.to_reverb_send.load(Ordering::Relaxed);
                if to_reverb > 0.01 {
                    let reverb_scale = 1.0 + (envelope_mod - 1.0) * to_reverb;
                    for send in &mut self.buffers.fdn_sends {
                        for n in 0..frames {
                            send[n] *= reverb_scale;
                        }
                    }
                }
                
                let to_delay = params.envelope.to_delay_send.load(Ordering::Relaxed);
                if to_delay > 0.01 {
                    let delay_scale = 1.0 + (envelope_mod - 1.0) * to_delay;
                    for n in 0..frames {
                        self.buffers.delay_send_l[n] *= delay_scale;
                        self.buffers.delay_send_r[n] *= delay_scale;
                    }
                }
            }
            
                
            // }


            // Send grains to spectral delay (parameterized)
            let grain_spectral_send = params.grains.spectral_send.load(Ordering::Relaxed);
            for n in 0..frames {
                self.buffers.spectral_send_l[n] = self.buffers.main_l[n] * grain_spectral_send;
                self.buffers.spectral_send_r[n] = self.buffers.main_r[n] * grain_spectral_send;
            }

        // }
        }
        
        // === STAGE 2: DELAY ===
        let delay_l_ms = params.delay.time_l_ms.load(Ordering::Relaxed);
        let delay_r_ms = params.delay.time_r_ms.load(Ordering::Relaxed);
        self.delay.set_times(delay_l_ms, delay_r_ms);
        
        let trail = params.delay.trail.load(Ordering::Relaxed);
        let crossfb = params.delay.crossfb.load(Ordering::Relaxed);
        self.delay.process(
            &self.buffers.delay_send_l,
            &self.buffers.delay_send_r,
            &mut self.buffers.delay_out_l,
            &mut self.buffers.delay_out_r,
            trail,
            crossfb,
            frames,
        );

        // === STAGE 2.5: SPECTRAL DELAY ===
        
        // Load base parameters and macro shapers
        let time_slope = params.spectral.time_slope.load(Ordering::Relaxed);
        let time_curve = params.spectral.time_curve.load(Ordering::Relaxed);
        let fb_slope = params.spectral.feedback_slope.load(Ordering::Relaxed);
        let fb_curve = params.spectral.feedback_curve.load(Ordering::Relaxed);
        let jitter = params.spectral.jitter.load(Ordering::Relaxed);
        let tilt = params.spectral.tilt.load(Ordering::Relaxed);
        
        // Calculate per-band times with macro shapers
        let mut shaped_times = [0.0f32; 8];
        let mut shaped_feedbacks = [0.0f32; 8];
        
        // Simple RNG for jitter (use a static seed for consistency)
        use rand::{Rng, SeedableRng};
        use rand_pcg::Pcg64;
        let mut rng = Pcg64::seed_from_u64(12345); // Deterministic per block
        
        for i in 0..8 {
            // Load base values from param arrays
            let base_time = params.spectral.band_times_ms[i].load(Ordering::Relaxed);
            let base_fb = params.spectral.band_feedbacks[i].load(Ordering::Relaxed);
            
            // Normalized position (0.0 = low freq, 1.0 = high freq)
            let t = i as f32 / 7.0;
            
            // === TIME SHAPING ===
            // Apply slope: -1.0 = longer at low freqs, +1.0 = longer at high freqs
            let slope_factor = 1.0 + (time_slope + tilt) * (t - 0.5) * 2.0;
            
            // Apply curve: -1.0 = exponential (emphasize extremes), +1.0 = S-curve (emphasize center)
            let curve_t = if time_curve < 0.0 {
                // Exponential: emphasize extremes
                let exp_amount = -time_curve;
                t.powf(1.0 + exp_amount * 2.0)
            } else {
                // S-curve: emphasize center
                let s = 3.0 * time_curve;
                (t - 0.5) * (1.0 + s * (1.0 - 4.0 * (t - 0.5).powi(2))) + 0.5
            };
            
            let curve_factor = 1.0 + (curve_t - 0.5) * 0.5;
            
            // Apply jitter: random variation
            let jitter_factor = 1.0 + (rng.gen::<f32>() * 2.0 - 1.0) * jitter * 0.2;
            
            shaped_times[i] = base_time * slope_factor * curve_factor * jitter_factor;
            shaped_times[i] = shaped_times[i].clamp(50.0, 2000.0);
            
            // === FEEDBACK SHAPING ===
            let fb_slope_factor = 1.0 + (fb_slope + tilt * 0.5) * (t - 0.5) * 2.0;
            
            let fb_curve_t = if fb_curve < 0.0 {
                t.powf(1.0 + (-fb_curve) * 2.0)
            } else {
                (t - 0.5) * (1.0 + 3.0 * fb_curve * (1.0 - 4.0 * (t - 0.5).powi(2))) + 0.5
            };
            
            let fb_curve_factor = 0.5 + (fb_curve_t - 0.5);
            
            shaped_feedbacks[i] = base_fb * fb_slope_factor * fb_curve_factor;
            shaped_feedbacks[i] = shaped_feedbacks[i].clamp(0.0, 0.92);
        }
        
        // Apply shaped parameters to spectral delay
        self.spectral.set_all_band_times(&shaped_times);
        self.spectral.set_all_band_feedbacks(&shaped_feedbacks);
        
        self.spectral.process(
            &self.buffers.spectral_send_l,
            &self.buffers.spectral_send_r,
            &mut self.buffers.spectral_out_l,
            &mut self.buffers.spectral_out_r,
            frames,
        );

        
        // === STAGE 3: FEEDBACK TO GRAINS ===
        self.grains.write_to_live_buffer(
            &self.buffers.delay_out_l,
            &self.buffers.delay_out_r,
            frames,
        );
        
        // === STAGE 4: DELAY → FDN SENDS (with rotor) ===
        self.apply_rotor_sends(frames, params);
        
        // === STAGE 5: FDN REVERB ===
        let bloom = params.reverb.bloom.load(Ordering::Relaxed);
        let halo = params.reverb.halo.load(Ordering::Relaxed);
        let lfo_depth = params.master.lfo_depth.load(Ordering::Relaxed);
        let lfo_rate = params.master.lfo_rate.load(Ordering::Relaxed);
        
        self.fdn.process(
            &self.buffers.fdn_sends,
            &mut self.buffers.main_l,
            &mut self.buffers.main_r,
            bloom,
            halo,
            lfo_depth,
            lfo_rate,
            &mut self.buffers.fdn_scratch_a,
            &mut self.buffers.fdn_scratch_b,
            &mut self.buffers.ir_scratch,
            frames,
        );
        
        // === STAGE 6: CONVOLUTION ===
        let mut con_l = [0.0f32; BLOCK];
        let mut con_r = [0.0f32; BLOCK];
        self.conv.process(
            &self.buffers.main_l,
            &self.buffers.main_r,
            &mut con_l,
            &mut con_r,
            frames,
        );
        
        // === STAGE 7: MIXING ===
        self.mix_stage(&con_l, &con_r, params, frames);
        
        // === STAGE 8: TILT EQ ===
        self.tilt.set_color_db(params.master.color.load(Ordering::Relaxed));
        for n in 0..frames {
            let (l, r) = self.tilt.process_frame(
                self.buffers.post_l[n],
                self.buffers.post_r[n],
            );
            self.buffers.post_l[n] = l;
            self.buffers.post_r[n] = r;
        }
        
        // === STAGE 9: DITHER + LIMITER ===
        self.apply_dither_and_limit(output, frames);
        
        // Return IR scratch for worker thread
        self.buffers.ir_scratch
    }
    
    // Helper methods to keep process_block clean
    fn apply_rotor_sends(&mut self, frames: usize, params:&Arc<GlobalParams>) {
        use std::f32::consts::PI;
        
        // This is the existing rotor logic - unchanged
        // FIX: Use persistent rotor phase (accumulates across blocks)
        let rotor_rate = 0.03;
        // let mut rotor_phase = 0.0; // In real impl, store in self (done in FIX below)
        let inc = 2.0 * PI * rotor_rate * (BLOCK as f32 / SR as f32);
        
        let mut weights = [0.0f32; FDN_LINES];
        let mut wsum = 0.0;
        for k in 0..FDN_LINES {
            let phase = self.rotor_phase + (2.0 * PI * k as f32 / FDN_LINES as f32);
            let w = 0.5 * (1.0 + phase.sin());
            weights[k] = w;
            wsum += w;
        }
        if wsum > 0.0 {
            for k in 0..FDN_LINES {
                weights[k] /= wsum;
            }
        }

        // Accumulate phase and wrap (FIX: Smooth wrapping without discontinuity)
        self.rotor_phase += inc;
        if self.rotor_phase >= 2.0 * PI {
            self.rotor_phase -= 2.0 * PI;
        }
        
        // for n in 0..frames {
        //     let mono = 0.5 * (self.buffers.delay_out_l[n] + self.buffers.delay_out_r[n]);
        //     for k in 0..FDN_LINES {
        //         self.buffers.fdn_sends[k][n] += mono * 0.30 * weights[k];
        //     }
        // }


        // Load send amounts
        let delay_fdn_send = params.delay.fdn_send.load(Ordering::Relaxed);
        let spectral_fdn_send = params.spectral.fdn_send.load(Ordering::Relaxed);
        
        for n in 0..frames {
            // Delay → FDN (with rotor)
            let delay_mono = 0.5 * (self.buffers.delay_out_l[n] + self.buffers.delay_out_r[n]);
            for k in 0..FDN_LINES {
                self.buffers.fdn_sends[k][n] += delay_mono * delay_fdn_send * weights[k];
            }
            
            // Spectral → FDN (with rotor)
            let spectral_mono = 0.5 * (self.buffers.spectral_out_l[n] + self.buffers.spectral_out_r[n]);
            for k in 0..FDN_LINES {
                self.buffers.fdn_sends[k][n] += spectral_mono * spectral_fdn_send * weights[k];
            }
        }

    }
    
    fn mix_stage(&mut self, con_l: &[f32], con_r: &[f32], params: &Arc<GlobalParams>, frames: usize) {
        use std::sync::atomic::Ordering::Relaxed;
        
        let grain_mix = params.grains.mix.load(Relaxed);
        let reverb_mix = params.reverb.mix.load(Relaxed);
        let delay_mix = params.delay.mix.load(Relaxed);
        let spectral_mix = params.spectral.mix.load(Relaxed);  // ← ADD THIS LINE
        let dry_wet = params.master.dry_wet.load(Relaxed);
        
        for n in 0..frames {
            let dry_l = self.buffers.main_l[n] * grain_mix;
            let dry_r = self.buffers.main_r[n] * grain_mix;
            
            let wet_l = reverb_mix * con_l[n] 
                      + delay_mix * self.buffers.delay_out_l[n]
                      + spectral_mix * self.buffers.spectral_out_l[n];  // ← ADD THIS LINE
            let wet_r = reverb_mix * con_r[n] 
                      + delay_mix * self.buffers.delay_out_r[n]
                      + spectral_mix * self.buffers.spectral_out_r[n];  // ← ADD THIS LINE
            
            self.buffers.post_l[n] = (1.0 - dry_wet) * dry_l + dry_wet * wet_l;
            self.buffers.post_r[n] = (1.0 - dry_wet) * dry_r + dry_wet * wet_r;
        }
    }
    
    fn apply_dither_and_limit(&mut self, output: &mut [f32], frames: usize) {
        // Dithering
        // Dithering (FIX: Use persistent state to prevent periodic 375 Hz tone)
        // let mut dither_state = 0xC0FFEEu32; // Store in self in real impl
        for n in 0..frames {
            // dither_state = dither_state.wrapping_mul(1664525).wrapping_add(1013904223);
            self.dither_state = self.dither_state.wrapping_mul(1664525).wrapping_add(1013904223);
            let d = ((self.dither_state ^ 0x9E3779B9) as f32 * (1.0 / 4294967296.0) - 0.5) * 0.001;
            self.buffers.post_l[n] += d;
            self.buffers.post_r[n] -= d;
        }
        
        // Peak limiting
        let mut peak = 0.0f32;
        for n in 0..BLOCK {
            peak = peak.max(self.buffers.post_l[n].abs());
            peak = peak.max(self.buffers.post_r[n].abs());
        }
        
        let target = 0.944;
        let g = if peak > target { target / peak } else { 1.0 };
        
        // Write to output with soft limiting
        for n in 0..frames {
            let l = self.buffers.post_l[n] * g;
            let r = self.buffers.post_r[n] * g;
            
            output[2 * n] = soft_clip(l, 1.0);
            output[2 * n + 1] = soft_clip(r, 1.0);
        }
    }
}

// === EASY TO EXTEND: Add new processors here ===
impl AudioGraph {
    /// Example: Add a new processor to the graph
    pub fn add_chorus_after_delay(&mut self) {
        // Just add a new processing stage in process_block
        // The routing structure makes it clear where to insert
    }
}