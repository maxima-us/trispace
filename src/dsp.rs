//! DSP Primitives and Utilities

use crate::types::{SR, WIN_TAB};
use std::f32::consts::PI;

// ===================== Biquad Filter =====================

#[derive(Clone, Copy)]
pub struct Biquad {
    b0: f32,
    b1: f32,
    b2: f32,
    a1: f32,
    a2: f32,
    x1: f32,
    x2: f32,
    y1: f32,
    y2: f32,
}

impl Biquad {
    pub fn new() -> Self {
        Self {
            b0: 1.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    pub fn set_lowpass(&mut self, fc: f32, q: f32) {
        let w0 = 2.0 * PI * fc / SR as f32;
        let (sin_w, cos_w) = w0.sin_cos();
        let alpha = sin_w / (2.0 * q);

        let a0 = 1.0 + alpha;
        self.b0 = ((1.0 - cos_w) / 2.0) / a0;
        self.b1 = (1.0 - cos_w) / a0;
        self.b2 = ((1.0 - cos_w) / 2.0) / a0;
        self.a1 = (-2.0 * cos_w) / a0;
        self.a2 = (1.0 - alpha) / a0;
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = self.b0 * x + self.b1 * self.x1 + self.b2 * self.x2 - self.a1 * self.y1
            - self.a2 * self.y2;

        self.x2 = self.x1;
        self.x1 = x;
        self.y2 = self.y1;
        self.y1 = y;

        y
    }

    pub fn reset(&mut self) {
        self.x1 = 0.0;
        self.x2 = 0.0;
        self.y1 = 0.0;
        self.y2 = 0.0;
    }
}

// ===================== #22: Exact 1-Pole Filters =====================

#[derive(Clone, Copy)]
pub struct OnePoleLP {
    a: f32,
    z: f32,
}

impl OnePoleLP {
    pub fn new(fc: f32, sample_rate: f32) -> Self {
        let a = Self::compute_coeff_f64(fc, sample_rate);
        Self { a, z: 0.0 }
    }

    fn compute_coeff_f64(fc: f32, sample_rate: f32) -> f32 {
        let fc_64 = fc as f64;
        let sr_64 = sample_rate as f64;
        let a_64 = (-2.0 * std::f64::consts::PI * fc_64 / sr_64).exp();
        a_64 as f32
    }

    pub fn set_cutoff(&mut self, fc: f32, sample_rate: f32) {
        self.a = Self::compute_coeff_f64(fc, sample_rate);
    }

    /// Get the filter coefficient (for building complementary filters)
    #[inline]
    pub fn get_coeff(&self) -> f32 {
        self.a
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        self.z = (1.0 - self.a) * x + self.a * self.z;
        self.z
    }

    pub fn reset(&mut self) {
        self.z = 0.0;
    }
}

#[derive(Clone, Copy)]
pub struct DcBlockerExact {
    r: f32,
    x_z1: f32,
    y_z1: f32,
}

impl DcBlockerExact {
    pub fn new(fc: f32, sample_rate: f32) -> Self {
        let r = Self::compute_coeff_f64(fc, sample_rate);
        Self {
            r,
            x_z1: 0.0,
            y_z1: 0.0,
        }
    }

    fn compute_coeff_f64(fc: f32, sample_rate: f32) -> f32 {
        let fc_64 = fc as f64;
        let sr_64 = sample_rate as f64;
        let r_64 = (-2.0 * std::f64::consts::PI * fc_64 / sr_64).exp();
        r_64 as f32
    }

    pub fn set_cutoff(&mut self, fc: f32, sample_rate: f32) {
        self.r = Self::compute_coeff_f64(fc, sample_rate);
    }

    #[inline]
    pub fn process(&mut self, x: f32) -> f32 {
        let y = x - self.x_z1 + self.r * self.y_z1;
        self.x_z1 = x;
        self.y_z1 = y;
        y
    }

    pub fn reset(&mut self) {
        self.x_z1 = 0.0;
        self.y_z1 = 0.0;
    }
}

// ===================== #25: Slew Limiters =====================

pub struct SlewLimiter {
    current: f32,
    target: f32,
    rate_per_second: f32,
    mode: SlewMode,
}

#[derive(Clone, Copy, PartialEq)]
pub enum SlewMode {
    Linear,
    Logarithmic,
}

impl SlewLimiter {
    pub fn new(initial_value: f32, rate_per_second: f32, mode: SlewMode) -> Self {
        Self {
            current: initial_value,
            target: initial_value,
            rate_per_second,
            mode,
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.target = target;
    }

    pub fn set_rate(&mut self, rate_per_second: f32) {
        self.rate_per_second = rate_per_second;
    }

    pub fn process(&mut self, delta_time: f32) -> f32 {
        match self.mode {
            SlewMode::Linear => {
                let max_change = self.rate_per_second * delta_time;
                let diff = self.target - self.current;
                let change = diff.clamp(-max_change, max_change);
                self.current += change;
                self.current
            }
            SlewMode::Logarithmic => {
                let epsilon = 1e-10;
                let current_log = (self.current.abs() + epsilon).log2();
                let target_log = (self.target.abs() + epsilon).log2();

                let max_change_octaves = self.rate_per_second * delta_time;
                let diff = target_log - current_log;
                let change = diff.clamp(-max_change_octaves, max_change_octaves);

                let new_log = current_log + change;
                self.current = 2.0f32.powf(new_log) * self.target.signum();
                self.current
            }
        }
    }

    pub fn current(&self) -> f32 {
        self.current
    }

    pub fn is_settled(&self, tolerance: f32) -> bool {
        (self.current - self.target).abs() < tolerance
    }

    pub fn reset_to(&mut self, value: f32) {
        self.current = value;
        self.target = value;
    }
}

pub struct BlockRateSlewLimiter {
    slewer: SlewLimiter,
    block_size: usize,
}

impl BlockRateSlewLimiter {
    pub fn new(
        initial_value: f32,
        rate_per_second: f32,
        mode: SlewMode,
        block_size: usize,
    ) -> Self {
        Self {
            slewer: SlewLimiter::new(initial_value, rate_per_second, mode),
            block_size,
        }
    }

    pub fn set_target(&mut self, target: f32) {
        self.slewer.set_target(target);
    }

    pub fn process_block(&mut self, sample_rate: f32) -> f32 {
        let delta_time = self.block_size as f32 / sample_rate;
        self.slewer.process(delta_time)
    }

    pub fn current(&self) -> f32 {
        self.slewer.current()
    }
}

pub mod slew_rates {
    pub const DELAY_TIME_OCTAVES_PER_SEC: f32 = 2.0;

    #[allow(dead_code)]
    pub const PITCH_SEMITONES_PER_SEC: f32 = 12.0;

    #[allow(dead_code)]
    pub const FILTER_CUTOFF_OCTAVES_PER_SEC: f32 = 4.0;

    #[allow(dead_code)]
    pub const GAIN_MIX_PER_SEC: f32 = 10.0;

    pub const FEEDBACK_PER_SEC: f32 = 2.0;
}

// ===================== #28: Window Normalization =====================

pub fn normalize_window_unit_power(window: &mut [f32]) {
    let n = window.len() as f32;
    let power_sum: f32 = window.iter().map(|&w| w * w).sum();

    if power_sum > 1e-12 {
        let scale = (n / power_sum).sqrt();
        for w in window.iter_mut() {
            *w *= scale;
        }
    }
}

pub fn create_hann_squared_unit_power(size: usize) -> Vec<f32> {
    let mut window = vec![0.0; size];

    for i in 0..size {
        let t = i as f32 / (size - 1).max(1) as f32;
        let hann = 0.5 - 0.5 * (2.0 * PI * t).cos();
        window[i] = hann * hann;
    }

    normalize_window_unit_power(&mut window);

    window
}

// ===================== Resampler =====================

const MIN_SAFE_GAP: usize = 8192;

pub struct Resampler {
    pub r: f32,
    w: usize,
    fifo: Vec<f32>,
    mask: usize,
    ratio_base: f32,
    lfo_phase: f32,
    lfo_inc: f32,
    lfo_depth: f32,
    pre_alpha: f32,
    pre_x1: f32,
    pre_y1: f32,
    de_alpha: f32,
    de_y1: f32,
    dc_blocker: DcBlockerExact,
}

impl Resampler {
    pub fn new(ratio_base: f32) -> Self {
        let fifo_len_pow2 = 16;
        let fifo_len = 1usize << fifo_len_pow2;

        let cutoff_hz = (SR as f32 / 2.0) / ratio_base.max(1.0) * 0.85;
        let rc = 1.0 / (2.0 * PI * cutoff_hz);
        let dt = 1.0 / SR as f32;
        let alpha = rc / (rc + dt);

        Self {
            r: MIN_SAFE_GAP as f32,
            w: 0,
            fifo: vec![0.0; fifo_len],
            mask: fifo_len - 1,
            ratio_base,
            lfo_phase: 0.0,
            lfo_inc: 2.0 * PI * 0.3 / SR as f32,
            lfo_depth: 0.001,
            pre_alpha: alpha,
            pre_x1: 0.0,
            pre_y1: 0.0,
            de_alpha: alpha,
            de_y1: 0.0,
            dc_blocker: DcBlockerExact::new(5.0, SR as f32),
        }
    }

    pub fn update_antialiasing(&mut self, ratio: f32) {
        let cutoff_hz = (SR as f32 / 2.0) / ratio.max(1.0) * 0.85;
        let rc = 1.0 / (2.0 * PI * cutoff_hz);
        let dt = 1.0 / SR as f32;
        self.pre_alpha = rc / (rc + dt);
        self.de_alpha = self.pre_alpha;
    }

    pub fn set_lfo(&mut self, depth: f32, rate_hz: f32) {
        self.lfo_depth = depth;
        self.lfo_inc = 2.0 * PI * rate_hz / SR as f32;
    }

    #[inline]
    fn hp_preemph(&mut self, x: f32) -> f32 {
        let y = self.pre_alpha * (self.pre_y1 + x - self.pre_x1);
        self.pre_x1 = x;
        self.pre_y1 = y;
        y
    }

    #[inline]
    fn lp_deemph(&mut self, x: f32) -> f32 {
        self.de_y1 = self.de_alpha * self.de_y1 + (1.0 - self.de_alpha) * x;
        self.de_y1
    }

    pub fn push_block(&mut self, input: &[f32]) {
        for &x in input {
            let y = self.hp_preemph(x);
            let y_dc = self.dc_blocker.process(y);
            self.fifo[self.w] = y_dc;
            self.w = (self.w + 1) & self.mask;
        }

        let r_to_w = ((self.w as isize - self.r as isize) & self.mask as isize) as usize;
        if r_to_w < MIN_SAFE_GAP {
            self.r = ((self.w + MIN_SAFE_GAP) & self.mask) as f32;
        }
    }

    #[inline]
    fn hermite_interp(&self, idx: f32) -> f32 {
        let i1 = idx.floor() as usize & self.mask;
        let i0 = (i1.wrapping_sub(1)) & self.mask;
        let i2 = (i1 + 1) & self.mask;
        let i3 = (i1 + 2) & self.mask;

        let ym1 = self.fifo[i0];
        let y0 = self.fifo[i1];
        let y1 = self.fifo[i2];
        let y2 = self.fifo[i3];

        let x = idx - idx.floor();

        let c0 = y0;
        let c1 = 0.5 * (y1 - ym1);
        let c2 = ym1 - 2.5 * y0 + 2.0 * y1 - 0.5 * y2;
        let c3 = 0.5 * (y2 - ym1) + 1.5 * (y0 - y1);

        ((c3 * x + c2) * x + c1) * x + c0
    }

    pub fn process(&mut self, out: &mut [f32]) {
        for y in out.iter_mut() {
            let s = self.hermite_interp(self.r);
            *y = self.lp_deemph(s);

            let lfo = self.lfo_phase.sin() * self.lfo_depth;
            let current_ratio = self.ratio_base * (1.0 + lfo);
            self.r += current_ratio;

            if self.r >= (self.mask as f32 + 1.0) {
                self.r -= self.mask as f32 + 1.0;
            }

            self.lfo_phase = (self.lfo_phase + self.lfo_inc) % (2.0 * PI);
        }
    }
}

// ===================== Shelf EQ =====================

pub struct TiltShelf {
    alpha: f32,
    g_lin: f32,
    zl: f32,
    zr: f32,
}

impl TiltShelf {
    pub fn new() -> Self {
        let fc = 1000.0;
        let alpha = (2.0 * PI * fc / SR as f32).clamp(0.0001, 0.9999);
        Self {
            alpha,
            g_lin: 1.0,
            zl: 0.0,
            zr: 0.0,
        }
    }

    pub fn set_color_db(&mut self, db: f32) {
        self.g_lin = 10f32.powf(db / 20.0);
    }

    #[inline]
    pub fn process_frame(&mut self, l: f32, r: f32) -> (f32, f32) {
        self.zl += self.alpha * (l - self.zl);
        self.zr += self.alpha * (r - self.zr);
        let hp_l = l - self.zl;
        let hp_r = r - self.zr;
        let l2 = l + (self.g_lin - 1.0) * hp_l;
        let r2 = r + (self.g_lin - 1.0) * hp_r;
        (l2, r2)
    }
}

// ===================== LFO =====================

pub struct Lfo {
    phase: f32,
    inc: f32,
}

impl Lfo {
    pub fn new(rate_hz: f32) -> Self {
        Self {
            phase: 0.0,
            inc: 2.0 * PI * rate_hz / SR as f32,
        }
    }

    #[inline]
    pub fn process(&mut self) -> f32 {
        let s = self.phase.sin();
        self.phase = (self.phase + self.inc) % (2.0 * PI);
        s
    }

    pub fn set_rate(&mut self, rate_hz: f32) {
        self.inc = 2.0 * PI * rate_hz / SR as f32;
    }
}