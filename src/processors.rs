//! Audio Effect Processors

use crate::dsp::{Biquad, DcBlockerExact, Lfo, Resampler};
use crate::types::*;
use rand::{Rng, SeedableRng};
use rand_pcg::Pcg64;
use std::f32::consts::PI;
use std::sync::atomic::Ordering;
use std::sync::Arc;

// ===================== Grain Engine =====================

#[repr(align(64))]
pub struct GrainPool {
    pos: Vec<f32>,
    step: Vec<f32>,
    end: Vec<f32>,
    gain: Vec<f32>,
    panl: Vec<f32>,
    panr: Vec<f32>,
    winpos: Vec<u32>,
    winstep: Vec<u32>,
    filter_z: Vec<f32>,
    filter_c: Vec<f32>,
    vib_phase: Vec<f32>,
    vib_inc: Vec<f32>,
    vib_depth: Vec<f32>,
    active: Vec<u64>,
    free: Vec<u16>,
    src: Vec<f32>,
    live_src: Vec<f32>,
    live_w: usize,
    win: [f32; WIN_TAB],
    rng: Pcg64,
    last_spawn: f32,
    energy_estimate: f32,
    energy_alpha: f32,
    dc_blocker: DcBlockerExact,
	source_is_live: Vec<bool>,
    pan_index: usize,  // ← ADD THIS LINE for #10 golden ratio panning
    // Navigation state (NEW)
    position_lfo_phase: f32,    // Current phase of position LFO (0 to 2π)
    base_position: f32,         // Current base position in sample (0.0-1.0)
    // Content-aware gain management (FIX: Energy estimator mismatch)
    static_rms_cache: Vec<f32>,     // Pre-computed RMS per 1-second region
    grain_output_rms: f32,          // Track actual grain output energy
    output_energy_alpha: f32,       // Faster adaptation (0.01 = ~10ms)
    active_grain_count: usize,      // Track density for gain compensation
}

impl GrainPool {
    pub fn new(sample_mono: Vec<f32>) -> Self {
        use crate::dsp::create_hann_squared_unit_power;
        
        let max_grains = crate::types::MAX_GRAINS;
        let mut free = Vec::with_capacity(max_grains);
        for i in (0..max_grains).rev() {
            free.push(i as u16);
        }

        let w_vec = create_hann_squared_unit_power(WIN_TAB);
        let mut w = [0.0; WIN_TAB];
        w.copy_from_slice(&w_vec);

        // Pre-compute static sample RMS cache (1-second windows)
        let static_rms_cache = Self::compute_static_rms_cache(&sample_mono);

        Self {
            pos: vec![0.0; max_grains],
            step: vec![0.0; max_grains],
            end: vec![0.0; max_grains],
            gain: vec![0.0; max_grains],
            panl: vec![1.0; max_grains],
            panr: vec![0.0; max_grains],
            winpos: vec![0; max_grains],
            winstep: vec![0; max_grains],
            filter_z: vec![0.0; max_grains],
            filter_c: vec![0.0; max_grains],
            vib_phase: vec![0.0; max_grains],
            vib_inc: vec![0.0; max_grains],
            vib_depth: vec![0.0; max_grains],
            active: vec![0; max_grains / 64],
            free,
            src: sample_mono,
            live_src: vec![0.0; SR * 2],
            live_w: 0,
            win: w,
            rng: Pcg64::from_seed([0; 32]),
            last_spawn: 0.0,
            energy_estimate: 0.0,
            energy_alpha: 0.001,
            dc_blocker: DcBlockerExact::new(5.0, SR as f32),
			source_is_live: vec![false; max_grains],  // ← ADD THIS LINE
            pan_index: 0,  // ← ADD THIS LINE
            // Navigation state (NEW)
            position_lfo_phase: 0.0,
            base_position: 0.0,
            // Content-aware gain (NEW)
            static_rms_cache,
            grain_output_rms: 0.0,
            output_energy_alpha: 0.01,  // 10ms adaptation (~480 samples @ 48kHz)
            active_grain_count: 0,
        }
    }

    /// Pre-compute RMS values for 1-second regions of static sample
    fn compute_static_rms_cache(sample: &[f32]) -> Vec<f32> {
        if sample.len() < SR {
            return vec![0.1]; // Fallback for short samples
        }
        
        let window_size = SR; // 1 second windows
        let num_windows = (sample.len() + window_size - 1) / window_size;
        let mut rms_cache = Vec::with_capacity(num_windows);
        
        for w in 0..num_windows {
            let start = w * window_size;
            let end = (start + window_size).min(sample.len());
            
            let mut sum_sq = 0.0;
            for i in start..end {
                sum_sq += sample[i] * sample[i];
            }
            
            let rms = (sum_sq / (end - start) as f32).sqrt();
            rms_cache.push(rms.max(0.01)); // Floor at -40dB to prevent division issues
        }
        
        rms_cache
    }
    
    /// Look up pre-computed RMS for a sample position
    #[inline]
    fn get_source_rms(&self, position: f32, source_len: usize) -> f32 {
        if self.static_rms_cache.is_empty() {
            return 0.1;
        }
        
        let window_idx = ((position / source_len as f32) * self.static_rms_cache.len() as f32)
            .floor() as usize;
        
        self.static_rms_cache
            .get(window_idx)
            .copied()
            .unwrap_or(0.1)
    }


    #[inline]
    fn bit_set(&mut self, i: usize) {
        self.active[i >> 6] |= 1u64 << (i & 63);
    }

    #[inline]
    fn bit_clear(&mut self, i: usize) {
        self.active[i >> 6] &= !(1u64 << (i & 63));
    }

    pub fn set_sample(&mut self, sample_mono: Vec<f32>) {
        let max_grains = crate::types::MAX_GRAINS;

        // Recompute RMS cache for new sample (FIX: Content-aware gain)
        self.static_rms_cache = Self::compute_static_rms_cache(&sample_mono);

        self.src = sample_mono;
        self.free.clear();
        for i in (0..max_grains).rev() {
            self.free.push(i as u16);
        }
        self.active.fill(0);
        self.energy_estimate = 0.0;
        self.grain_output_rms = 0.0; // Reset output tracking
        self.active_grain_count = 0;
    }

    fn spawn(&mut self, params: &Arc<GlobalParams>) {
        if let Some(id) = self.free.pop() {
            let i = id as usize;

            let min_ms = params.grains.size_min_ms.load(Ordering::Relaxed);
            let max_ms = params.grains.size_max_ms.load(Ordering::Relaxed);
            let t = self.rng.gen::<f32>();
            let gms = min_ms + (max_ms - min_ms) * t.powi(2);

            let feedback_prob = params.grains.feedback.load(Ordering::Relaxed);
            let use_live = self.rng.gen::<f32>() < feedback_prob;

            let (source_len,) = if use_live && self.live_src.len() > 1024 {
                (self.live_src.len(),)
            } else {
                (self.src.len(),)
            };

			// Store which buffer this grain will use
            self.source_is_live[i] = use_live && self.live_src.len() > 1024;  // ← ADD THIS 

            // let spray_ms = params.grains.spray_ms.load(Ordering::Relaxed);
            // let spray_samps = spray_ms * 0.001 * SR as f32;
            // let spray_offset = (self.rng.gen::<f32>() * 2.0 - 1.0) * spray_samps;

            // let base_start = self.rng.gen::<f32>() * (source_len as f32 - 2.0);
            // let start = (base_start + spray_offset).clamp(0.0, source_len as f32 - 2.0);

            // let dur_samp = (gms * 0.001 * SR as f32).clamp(64.0, 0.5 * source_len as f32);


            // ========== NAVIGATION: Determine spawn position center ==========
            let nav_mode = params.grains.nav_mode.load(Ordering::Relaxed);
            let position_center = if nav_mode < 0.5 {
                // Static mode: random position (original behavior)
                self.rng.gen::<f32>() * source_len as f32
            } else if nav_mode < 1.5 {
                // Scan mode: LFO-driven position (smooth traversal)
                self.base_position * source_len as f32
            } else {
                // Jump mode: random across entire sample (chaotic exploration)
                self.rng.gen::<f32>() * source_len as f32
            };

            // ========== PROPORTIONAL SPRAY: Scale with sample length ==========
            let spray_ms = params.grains.spray_ms.load(Ordering::Relaxed);
            let spray_scale = params.grains.spray_scale.load(Ordering::Relaxed);
            let spray_samps = (spray_ms * 0.001 * SR as f32) * spray_scale;
            let spray_offset = (self.rng.gen::<f32>() * 2.0 - 1.0) * spray_samps;

            let start = (position_center + spray_offset).clamp(0.0, source_len as f32 - 2.0);

            let dur_samp = (gms * 0.001 * SR as f32).clamp(64.0, 0.5 * source_len as f32);



            self.pos[i] = start;
            self.end[i] = (start + dur_samp).min(source_len as f32 - 1.0);

            let pitch_var_semi = params.grains.pitch_var.load(Ordering::Relaxed);
            let cents = (self.rng.gen::<f32>() * 2.0 - 1.0) * pitch_var_semi * 100.0;
            self.step[i] = 2f32.powf(cents / 1200.0);
            
            // energy reduction start
            // let energy_reduction = if self.energy_estimate > 0.3 {
            //     (0.3 / self.energy_estimate).min(1.0)
            // } else {
            //     1.0
            // };
            // self.gain[i] = (0.12 + 0.38 * self.rng.gen::<f32>()) * energy_reduction;
            // energy reduction end

           
            // Content-aware gain: account for source loudness + current output level + density
            let source_rms = if use_live && self.live_src.len() > 1024 {
                // For live buffer, use current energy estimate
                self.energy_estimate.max(0.01)
            } else {
                // For static sample, use pre-computed RMS at spawn position
                self.get_source_rms(start, source_len)
            };
            
            // Grain output energy reduction (tracks actual output, not just feedback)
            let output_reduction = if self.grain_output_rms > 0.25 {
                (0.25 / self.grain_output_rms).min(1.0)
            } else {
                1.0
            };
            
            // Density-based reduction: scale down as grain count increases
            let density_reduction = if self.active_grain_count > 0 {
                let target_count = 200.0; // Expected avg active grains at density=5000
                (target_count / self.active_grain_count.max(1) as f32).min(1.0).sqrt()
            } else {
                1.0
            };
            
            // Source-aware reduction: louder regions get less gain
            let source_reduction = if source_rms > 0.15 {
                (0.15 / source_rms).sqrt() // Gentler reduction (square root)
            } else {
                1.0
            };
            
            // Combine all reduction factors
            let base_gain = 0.12 + 0.38 * self.rng.gen::<f32>();
            self.gain[i] = base_gain * output_reduction * density_reduction * source_reduction;



            // let pan = (self.rng.gen::<f32>() * 2.0) - 1.0;
            // let t: f32 = 0.5 * (pan + 1.0);
            // self.panl[i] = (1.0 - t).sqrt();
            // self.panr[i] = t.sqrt();

            // #10: Golden ratio panning for even stereo distribution
            const PHI_GOLDEN: f32 = 0.6180339887498948; // (√5 - 1) / 2
            let angle = 2.0 * PI * (self.pan_index as f32) * PHI_GOLDEN;
            let pan = angle.cos(); // Maps to [-1, 1]
            
            // Equal-power panning
            let t: f32 = 0.5 * (pan + 1.0); // Map to [0, 1]
            self.panl[i] = (1.0 - t).sqrt();
            self.panr[i] = t.sqrt();
            
            self.pan_index = self.pan_index.wrapping_add(1); // Increment for next grain


            self.winpos[i] = 0;
            let step_q12 = ((WIN_TAB as f32 / dur_samp) * (1 << 12) as f32)
                .max(1.0)
                .min(u32::MAX as f32 - 1.0);
            self.winstep[i] = step_q12.round() as u32;

            let vib_rate_hz = 0.2 + self.rng.gen::<f32>() * 0.3;
            self.vib_inc[i] = 2.0 * PI * vib_rate_hz / SR as f32;
            self.vib_phase[i] = self.rng.gen::<f32>() * 2.0 * PI;
            let vib_cents = 2.0 + self.rng.gen::<f32>() * 3.0;
            self.vib_depth[i] = vib_cents;

            let cutoff_hz = 1000.0 + self.rng.gen::<f32>().powi(2) * 15000.0;
            let base_c = (PI * cutoff_hz / SR as f32).clamp(0.001, 1.0);
            let filter_q = params.grains.filter_q.load(Ordering::Relaxed);
            self.filter_c[i] = base_c + filter_q * (1.0 - base_c);
            self.filter_z[i] = 0.0;

            self.bit_set(i);
        }
    }

    // fn auto_spawn(&mut self, params: &Arc<GlobalParams>, block_secs: f32) {
    //     let density = params.grains.density.load(Ordering::Relaxed).max(1.0);
    //     self.last_spawn += density * block_secs;
    //     let mut to_spawn = self.last_spawn.floor() as i32;
    //     self.last_spawn -= to_spawn as f32;

    //     while to_spawn > 0 {
    //         to_spawn -= 1;
    //         self.spawn(params);
    //     }
    // }

    fn auto_spawn(&mut self, params: &Arc<GlobalParams>, block_secs: f32) {
        // #8: Poisson spawning - non-periodic grain triggers
        let lambda = params.grains.density.load(Ordering::Relaxed).max(1.0);
        
        // Accumulate wall-clock time
        self.last_spawn -= block_secs;
        
        // Spawn while accumulator indicates we should (Poisson process)
        while self.last_spawn <= 0.0 {
            self.spawn(params);
            
            // Generate next inter-arrival time: Δt = -ln(1-u)/λ
            let u = self.rng.gen::<f32>(); // U(0,1)
            let delta_t = -((1.0 - u).max(1e-7).ln()) / lambda;
            
            // Clamp to avoid bursts: min ~0.1ms, max 1s
            let delta_t_clamped = delta_t.clamp(0.0001, 1.0);
            
            self.last_spawn += delta_t_clamped;
        }
    }



    #[inline]
    fn win_lookup(&self, q20_12: u32) -> f32 {
        self.win[((q20_12 >> 12) as usize).min(WIN_TAB - 1)]
    }

    pub fn write_to_live_buffer(&mut self, input_l: &[f32], input_r: &[f32], frames: usize) {
        let mut block_energy = 0.0;

        for n in 0..frames {
            let sample = 0.5 * (input_l[n] + input_r[n]);
            let sample_dc = self.dc_blocker.process(sample);
            block_energy += sample_dc * sample_dc;
            self.live_src[self.live_w] = sample_dc;
            self.live_w = (self.live_w + 1) % self.live_src.len();
        }

        let block_rms = (block_energy / frames as f32).sqrt();
        self.energy_estimate =
            (1.0 - self.energy_alpha) * self.energy_estimate + self.energy_alpha * block_rms;

        if self.energy_estimate > 0.35 {
            let reduction = (0.35 / self.energy_estimate).min(1.0);
            for i in 0..self.live_src.len() {
                self.live_src[i] *= reduction;
            }
            self.energy_estimate *= reduction;
        }
    }

    pub fn process_block(
        &mut self,
        out_l: &mut [f32],
        out_r: &mut [f32],
        fdn_sends: &mut [Vec<f32>; FDN_LINES],
        delay_send_l: &mut [f32],
        delay_send_r: &mut [f32],
        params: &Arc<GlobalParams>,
        frames: usize,
    ) {
        const TILE: usize = 64;
        let live_src_len = self.live_src.len();
        let static_src_len = self.src.len();
        let num_tiles = (frames + TILE - 1) / TILE;  // ← ADD THIS LINE

        // Load send amounts once per block (ADD THESE 2 LINES)
        let grain_fdn_send = params.grains.fdn_send.load(Ordering::Relaxed);
        let grain_delay_send = params.grains.delay_send.load(Ordering::Relaxed);

        // Track grain output energy and active count (FIX: Energy estimator)
        let mut block_output_energy = 0.0;
        self.active_grain_count = 0;


        // ========== UPDATE POSITION LFO (Scan mode only) ==========
        let nav_mode = params.grains.nav_mode.load(Ordering::Relaxed);
        if nav_mode >= 0.5 && nav_mode < 1.5 {
            // Scan mode: update LFO to slowly move through sample
            let lfo_rate = params.grains.position_lfo_rate.load(Ordering::Relaxed);
            let lfo_depth = params.grains.position_lfo_depth.load(Ordering::Relaxed);
            
            // Increment phase based on block size
            let phase_inc = 2.0 * std::f32::consts::PI * lfo_rate * (frames as f32 / SR as f32);
            self.position_lfo_phase = (self.position_lfo_phase + phase_inc) % (2.0 * std::f32::consts::PI);
            
            // Calculate base position (0.0-1.0, oscillates around 0.5)
            let lfo_value = self.position_lfo_phase.sin(); // -1.0 to 1.0
            self.base_position = 0.5 + (lfo_value * lfo_depth * 0.5);
            self.base_position = self.base_position.clamp(0.0, 1.0);
        }


        for t in 0..num_tiles {
            let s = t * TILE;
            let e = (s + TILE).min(frames); 

            for w in 0..self.active.len() {
                let mut mask = self.active[w];
                while mask != 0 {
                    let bit = mask.trailing_zeros() as usize;
                    let i = (w << 6) + bit;
                    mask &= mask - 1;

                    // Count active grains for density compensation
                    self.active_grain_count += 1;

                    let mut p = self.pos[i];
                    let mut wq = self.winpos[i];
                    let step = self.step[i];
                    let end = self.end[i];
                    let g = self.gain[i];
                    let pl = self.panl[i];
                    let pr = self.panr[i];
                    let c = self.filter_c[i];
                    let mut z = self.filter_z[i];

					// REPLCAED WITH BELOW BLOCK
                    // let use_live = end <= live_src_len as f32 && p < live_src_len as f32;
                    // let (source, src_len) = if use_live {
                    //     (&self.live_src, live_src_len)
                    // } else {
                    //     (&self.src, static_src_len)
                    // };

					// Use the stored flag instead of recalculating
                    let (source, src_len) = if self.source_is_live[i] {
                        (&self.live_src, live_src_len)
                    } else {
                        (&self.src, static_src_len)
                    };

                    for n in s..e {
                        if p >= end {
                            self.bit_clear(i);
                            self.free.push(i as u16);
                            break;
                        }

                        let i0 = p as usize;
                        let i1 = (i0 + 1).min(src_len - 1);
                        let frac = p - i0 as f32;
                        let samp = source[i0] + frac * (source[i1] - source[i0]);
                        let win = self.win_lookup(wq);
                        let mut s1 = samp * win * g;

                        s1 = z + c * (s1 - z);
                        z = s1;

                        let l = s1 * pl;
                        let r = s1 * pr;
                        out_l[n] += l;
                        out_r[n] += r;

                        // Track output energy (for adaptive gain)
                        block_output_energy += l * l + r * r;

                        let line = (((pl - pr).abs() * (FDN_LINES as f32 - 1.0)).round() as usize)
                            .min(FDN_LINES - 1);
                        // fdn_sends[line][n] += (l + r) * 0.5 * 0.20;
                        // delay_send_l[n] += l * 0.12;
                        // delay_send_r[n] += r * 0.12;
                        fdn_sends[line][n] += (l + r) * 0.5 * grain_fdn_send;
                        delay_send_l[n] += l * grain_delay_send;
                        delay_send_r[n] += r * grain_delay_send;

                        p += step;
                        self.vib_phase[i] = (self.vib_phase[i] + self.vib_inc[i]) % (2.0 * PI);

                        wq = wq.wrapping_add(self.winstep[i]);
                    }

                    self.pos[i] = p;
                    self.winpos[i] = wq;
                    self.filter_z[i] = z;
                }
            }
        }

        // Update grain output RMS tracker (FIX: Track actual output, not just feedback)
        let block_rms = (block_output_energy / (frames as f32 * 2.0)).sqrt(); // Stereo channels
        self.grain_output_rms = (1.0 - self.output_energy_alpha) * self.grain_output_rms 
                               + self.output_energy_alpha * block_rms;
        
        // Apply block-rate safety limiting if output is still too hot
        if self.grain_output_rms > 0.35 {
            let scale = 0.35 / self.grain_output_rms;
            for n in 0..frames {
                out_l[n] *= scale;
                out_r[n] *= scale;
            }
            // Also scale sends to prevent feedback buildup
            for send in fdn_sends.iter_mut() {
                for n in 0..frames {
                    send[n] *= scale;
                }
            }
            for n in 0..frames {
                delay_send_l[n] *= scale;
                delay_send_r[n] *= scale;
            }
            
            self.grain_output_rms *= scale; // Update tracker
        }


        self.auto_spawn(params, frames as f32 / SR as f32);
    }
}

// ===================== Stereo Delay =====================

pub struct StereoDelay {
    buf_l: Vec<f32>,
    buf_r: Vec<f32>,
    mask: usize,
    w: usize,
    damp_l: Biquad,
    damp_r: Biquad,
    dc_l: DcBlockerExact,
    dc_r: DcBlockerExact,
    time_l_target: f32,
    time_r_target: f32,
    time_l_current: f32,
    time_r_current: f32,
    smooth_alpha: f32,
}

impl StereoDelay {
    pub fn new(pow2: usize) -> Self {
        let len = 1usize << pow2;
        let mask = len - 1;

        let mut damp_l = Biquad::new();
        let mut damp_r = Biquad::new();
        damp_l.set_lowpass(4000.0, 0.707);
        damp_r.set_lowpass(4000.0, 0.707);

        let default_time = 650.0 * 0.001 * SR as f32;

        Self {
            buf_l: vec![0.0; len],
            buf_r: vec![0.0; len],
            mask,
            w: 0,
            damp_l,
            damp_r,
            dc_l: DcBlockerExact::new(5.0, SR as f32),
            dc_r: DcBlockerExact::new(5.0, SR as f32),
            time_l_target: default_time,
            time_r_target: default_time,
            time_l_current: default_time,
            time_r_current: default_time,
            smooth_alpha: 0.0001,
        }
    }

    pub fn set_times(&mut self, time_l_ms: f32, time_r_ms: f32) {
        self.time_l_target = time_l_ms * 0.001 * SR as f32;
        self.time_r_target = time_r_ms * 0.001 * SR as f32;
    }

    #[inline]
    fn read_lin(buf: &Vec<f32>, mask: usize, r: f32) -> f32 {
        let i0 = r as usize & mask;
        let i1 = (i0 + 1) & mask;
        let t = r - i0 as f32;
        buf[i0] + t * (buf[i1] - buf[i0])
    }

    pub fn process(
        &mut self,
        in_l: &[f32],
        in_r: &[f32],
        out_l: &mut [f32],
        out_r: &mut [f32],
        fb_param: f32,
        crossfb: f32,
        frames: usize,
    ) {
        let fb = fb_param.clamp(0.0, 0.70);
        let xfb = crossfb.clamp(0.0, 0.70);

        for n in 0..frames {
            self.time_l_current +=
                self.smooth_alpha * (self.time_l_target - self.time_l_current);
            self.time_r_current +=
                self.smooth_alpha * (self.time_r_target - self.time_r_current);

            let rl = (self.w as f32 - self.time_l_current + self.mask as f32)
                % (self.mask as f32 + 1.0);
            let rr = (self.w as f32 - self.time_r_current + self.mask as f32)
                % (self.mask as f32 + 1.0);

            let dl = Self::read_lin(&self.buf_l, self.mask, rl);
            let dr = Self::read_lin(&self.buf_r, self.mask, rr);

            let yl = self.damp_l.process(dl);
            let yr = self.damp_r.process(dr);

            let yl_dc = self.dc_l.process(yl);
            let yr_dc = self.dc_r.process(yr);

            out_l[n] = yl_dc;
            out_r[n] = yr_dc;

            let fb_l = soft_clip(in_l[n] + fb * yl_dc + xfb * yr_dc, 0.9);
            let fb_r = soft_clip(in_r[n] + fb * yr_dc + xfb * yl_dc, 0.9);

            self.buf_l[self.w] = fb_l;
            self.buf_r[self.w] = fb_r;

            self.w = (self.w + 1) & self.mask;
        }
    }
}

// ===================== FDN Reverb =====================

pub struct FdnLine {
    buf: Vec<f32>,
    shimmer_buf: Vec<f32>,
    len: usize,
    w: usize,
    damper: Biquad,
    dc_blocker: DcBlockerExact,
    shimmer: Option<Resampler>,
}

pub struct Fdn {
    lines: Vec<FdnLine>,
    matrix: [[f32; FDN_LINES]; FDN_LINES],
    lfos: Vec<Lfo>,
    predelay_buf: Vec<f32>,
    predelay_len: usize,
    predelay_w: usize,
}

impl Fdn {
    pub fn new(size_scale: f32, damping: f32, predelay_ms: f32) -> Self {
        let base_lengths_ms = [
            40.1, 46.3, 53.2, 61.3, 72.1, 85.0, 99.7, 116.2, 137.4, 161.8, 189.2, 221.1, 251.3,
            289.7, 337.1, 392.3, 457.2, 532.8, 620.5, 723.1, 842.7, 982.1, 1144.3, 1333.9,
        ];

        let mut lines = Vec::with_capacity(FDN_LINES);
        let mut lfos = Vec::with_capacity(FDN_LINES);
        let mut rng = Pcg64::from_seed([1; 32]);

        let damp_fc = lerp(1000.0, 6000.0, 1.0 - damping);

        for (k, &base_ms) in base_lengths_ms.iter().enumerate() {
            let ms = base_ms * size_scale + (rng.gen::<f32>() * 2.0 - 1.0);
            let len = (ms * 0.001 * SR as f32) as usize;

            let mut damper = Biquad::new();
            damper.set_lowpass(damp_fc, 0.707);

            let shimmer = if k == 3 {
                let mut r = Resampler::new(2.000);
                r.set_lfo(0.001, 0.35);
                Some(r)
            } else if k == 9 {
                let mut r = Resampler::new(1.996);
                r.set_lfo(0.001, 0.31);
                Some(r)
            } else {
                None
            };

            lines.push(FdnLine {
                buf: vec![0.0; len],
                shimmer_buf: vec![0.0; len],
                len,
                w: 0,
                damper,
                dc_blocker: DcBlockerExact::new(5.0, SR as f32),
                shimmer,
            });

            lfos.push(Lfo::new(0.1 + rng.gen::<f32>() * 0.1));
        }

        let matrix = Self::householder_matrix();

        let predelay_len = (predelay_ms * 0.001 * SR as f32) as usize;

        Self {
            lines,
            matrix,
            lfos,
            predelay_buf: vec![0.0; predelay_len.max(1)],
            predelay_len,
            predelay_w: 0,
        }
    }

    fn householder_matrix() -> [[f32; FDN_LINES]; FDN_LINES] {
        let mut h = [[0.0; FDN_LINES]; FDN_LINES];
        let v = 1.0 / (FDN_LINES as f32).sqrt();

        for i in 0..FDN_LINES {
            for j in 0..FDN_LINES {
                h[i][j] = if i == j {
                    -1.0 + 2.0 * v * v
                } else {
                    2.0 * v * v
                };
            }
        }
        h
    }

    pub fn update_config(&mut self, size_scale: f32, damping: f32, predelay_ms: f32) {
        *self = Self::new(size_scale, damping, predelay_ms);
    }

    pub fn process(
        &mut self,
        sends: &[Vec<f32>; FDN_LINES],
        out_l: &mut [f32],
        out_r: &mut [f32],
        bloom: f32,
        halo: f32,
        lfo_depth: f32,
        lfo_rate: f32,
        scratch_a: &mut [f32],
        scratch_b: &mut [f32],
        ir_scratch: &mut [f32],
        frames: usize,
    ) {
        let lfo_samps = lfo_depth * 0.001 * SR as f32;
        let fb = bloom.clamp(0.0, 0.75);

        let mut y: [f32; FDN_LINES] = [0.0; FDN_LINES];
        let mut feedback_in: [f32; FDN_LINES] = [0.0; FDN_LINES];

        for n in 0..frames {
            for k in 0..FDN_LINES {
                let line = &mut self.lines[k];

                self.lfos[k].set_rate(lfo_rate * (0.8 + 0.4 * (k as f32 / FDN_LINES as f32)));
                let mod_samps = self.lfos[k].process() * lfo_samps;

                let base_delay = line.len as f32 - 1.0;
                let read_pos =
                    (line.w as f32 - base_delay - mod_samps + line.len as f32) % line.len as f32;

                let i0 = read_pos.floor() as usize % line.len;
                let i1 = (i0 + 1) % line.len;
                let frac = read_pos - read_pos.floor();

                let d = line.buf[i0] + frac * (line.buf[i1] - line.buf[i0]);
                let d_damp = line.damper.process(d);
                let d_dc = line.dc_blocker.process(d_damp);
                y[k] = d_dc;
            }

            for i in 0..FDN_LINES {
                let mut sum = 0.0;
                for j in 0..FDN_LINES {
                    sum += self.matrix[i][j] * y[j];
                }
                feedback_in[i] = sum;
            }


            // Energy limiting: prevent FDN runaway
            let mut total_energy = 0.0;
            for i in 0..FDN_LINES {
                total_energy += feedback_in[i] * feedback_in[i];
            }
            let rms = (total_energy / FDN_LINES as f32).sqrt();
            if rms > 0.5 {
                let scale = 0.5 / rms;
                for i in 0..FDN_LINES {
                    feedback_in[i] *= scale;
                }
            }


            let predelay_out = if self.predelay_len > 0 {
                let pd_read = (self.predelay_w + 1) % self.predelay_len;
                self.predelay_buf[pd_read]
            } else {
                0.0
            };

            for k in 0..FDN_LINES {
                let line = &mut self.lines[k];

                let x_in = soft_clip(sends[k][n] + predelay_out * 0.1 + feedback_in[k] * fb, 0.9);

                line.buf[line.w] = x_in;

                out_l[n] += x_in * (1.0 - (k as f32 / (FDN_LINES as f32 - 1.0)));
                out_r[n] += x_in * (k as f32 / (FDN_LINES as f32 - 1.0));

                line.w = (line.w + 1) % line.len;
            }

            if self.predelay_len > 0 {
                let mono_in = sends.iter().map(|s| s[n]).sum::<f32>() / FDN_LINES as f32;
                self.predelay_buf[self.predelay_w] = mono_in;
                self.predelay_w = (self.predelay_w + 1) % self.predelay_len;
            }
        }

        {
            let line = &mut self.lines[3];
            let write_start_idx =
                (line.w as isize - frames as isize).rem_euclid(line.len as isize) as usize;

            for i in 0..frames {
                scratch_a[i] = line.buf[(write_start_idx + i) % line.len];
            }

            if let Some(res) = &mut line.shimmer {
                // res.push_block(scratch_a);
                res.push_block(&scratch_a[..frames]);  // ← Only pass valid data
                let mut sh = [0.0; BLOCK]; // ← Keep BLOCK allocation
                // res.process(&mut sh);
                res.process(&mut sh[..frames]);  // ← Only process what we need

                for i in 0..frames {
                    line.shimmer_buf[(write_start_idx + i) % line.len] = sh[i];
                }
            }

            for i in 0..frames {
                let idx = (write_start_idx + i) % line.len;
                let original = line.buf[idx];
                let shimmer_val = line.shimmer_buf[idx];
                line.buf[idx] = (1.0 - halo) * original + halo * shimmer_val;
            }
        }

        {
            let line = &mut self.lines[9];
            let write_start_idx =
                (line.w as isize - frames as isize).rem_euclid(line.len as isize) as usize;

            for i in 0..frames {
                scratch_b[i] = line.buf[(write_start_idx + i) % line.len];
            }

            if let Some(res) = &mut line.shimmer {
                // res.push_block(scratch_b);
                res.push_block(&scratch_b[..frames]);  // ← Only pass valid data
                let mut sh = [0.0; BLOCK];
                // res.process(&mut sh);
                res.process(&mut sh[..frames]);  // ← Only process what we need

                for i in 0..frames {
                    line.shimmer_buf[(write_start_idx + i) % line.len] = sh[i];
                }
            }

            for i in 0..frames {
                let idx = (write_start_idx + i) % line.len;
                let original = line.buf[idx];
                let shimmer_val = line.shimmer_buf[idx];
                line.buf[idx] = (1.0 - halo) * original + halo * shimmer_val;
            }
        }

        for i in 0..frames {
            ir_scratch[i] = 0.0;
            for k in 0..FDN_LINES {
                ir_scratch[i] += y[k];
            }
            ir_scratch[i] *= 1.0 / FDN_LINES as f32;
        }
    }
}

// ===================== Convolution Head =====================

pub struct ConvHead {
    taps_a: Vec<f32>,
    taps_b: Vec<f32>,
    use_a: bool,
    hist_l: Vec<f32>,
    hist_r: Vec<f32>,
    idx: usize,
}

impl ConvHead {
    pub fn new() -> Self {
        Self {
            taps_a: vec![0.0; CONV_HEAD_TAPS],
            taps_b: vec![0.0; CONV_HEAD_TAPS],
            use_a: true,
            hist_l: vec![0.0; CONV_HEAD_TAPS],
            hist_r: vec![0.0; CONV_HEAD_TAPS],
            idx: 0,
        }
    }

    pub fn update_taps(&mut self, new_taps: &[f32; CONV_HEAD_TAPS]) {
        let dst = if self.use_a {
            &mut self.taps_b
        } else {
            &mut self.taps_a
        };
        dst.copy_from_slice(new_taps);
        self.use_a = !self.use_a;
    }

    pub fn process(&mut self, in_l: &[f32], in_r: &[f32], out_l: &mut [f32], out_r: &mut [f32], frames: usize) {
        let taps = if self.use_a {
            &self.taps_a
        } else {
            &self.taps_b
        };

        for n in 0..frames {
            self.hist_l[self.idx] = in_l[n];
            self.hist_r[self.idx] = in_r[n];

            let mut acc_l = 0.0;
            let mut acc_r = 0.0;
            let mut j = self.idx;

            for t in 0..CONV_HEAD_TAPS {
                acc_l += taps[t] * self.hist_l[j];
                acc_r += taps[t] * self.hist_r[j];
                j = if j == 0 {
                    CONV_HEAD_TAPS - 1
                } else {
                    j - 1
                };
            }

            out_l[n] = acc_l * 0.5;
            out_r[n] = acc_r * 0.5;
            self.idx = (self.idx + 1) % CONV_HEAD_TAPS;
        }
    }
}