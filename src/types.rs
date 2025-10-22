//! Shared Types, Constants, and Utilities

use anyhow::Result;
use atomic_float::AtomicF32;
use std::sync::atomic::Ordering;

// ===================== Constants =====================

pub const SR: usize = 48_000;
pub const BLOCK: usize = 128;
pub const MAX_GRAINS: usize = 102400;
pub const WIN_TAB: usize = 2048;
pub const FDN_LINES: usize = 24;
pub const FFT_SIZE: usize = 4096;
pub const CONV_HEAD_TAPS: usize = 1024;


// ===================== Navigation Mode =====================

#[derive(Clone, Copy, PartialEq)]
pub enum NavigationMode {
    Static = 0,   // Original behavior - grains spawn around single point
    Scan = 1,     // LFO-driven smooth scanning through sample
    Jump = 2,     // Random jumps across entire sample
}

// ===================== Envelope Parameter Registry =====================

pub struct EnvelopeParams {
    pub enabled: AtomicF32,
    
    // Per-band attack/release (ms)
    pub attack_low: AtomicF32,
    pub attack_mid: AtomicF32,
    pub attack_high: AtomicF32,
    pub release_low: AtomicF32,
    pub release_mid: AtomicF32,
    pub release_high: AtomicF32,
    
    // Modulation targets
    pub to_reverb_send: AtomicF32,
    pub to_delay_send: AtomicF32,
    // pub to_density: AtomicF32,
    
    // Curve shaping
    pub curve: AtomicF32,
    pub minimum: AtomicF32,
}

impl Default for EnvelopeParams {
    fn default() -> Self {
        Self {
            enabled: AtomicF32::new(0.5),
            
            attack_low: AtomicF32::new(10.0),
            attack_mid: AtomicF32::new(5.0),
            attack_high: AtomicF32::new(2.0),
            release_low: AtomicF32::new(100.0),
            release_mid: AtomicF32::new(50.0),
            release_high: AtomicF32::new(25.0),
            
            to_reverb_send: AtomicF32::new(0.3),
            to_delay_send: AtomicF32::new(0.2),
            // to_density: AtomicF32::new(0.15),
            
            curve: AtomicF32::new(0.0),
            minimum: AtomicF32::new(0.4),
        }
    }
}

// ===================== Global Parameter Registry =====================

pub struct GlobalParams {
    // Core processors
    pub reverb: ReverbParams,
    pub delay: DelayParams,
    pub spectral: SpectralParams,  // ← ADD THIS LINE
    pub grains: GrainParams,
    pub envelope: EnvelopeParams,  // ← ADD THIS
    
    // Global
    pub master: MasterParams,
}

impl Default for GlobalParams {
    fn default() -> Self {
        Self {
            reverb: ReverbParams::default(),
            delay: DelayParams::default(),
            spectral: SpectralParams::default(),  // ← ADD THIS LINE
            grains: GrainParams::default(),
            envelope: EnvelopeParams::default(),  // ← ADD THIS
            master: MasterParams::default(),
        }
    }
}

// ===================== Individual Parameter Structs =====================

pub struct ReverbParams {
    pub bloom: AtomicF32,
    pub halo: AtomicF32,
    pub size: AtomicF32,
    pub damping: AtomicF32,
    pub predelay_ms: AtomicF32,
    pub mix: AtomicF32,
}

impl Default for ReverbParams {
    fn default() -> Self {
        Self {
            bloom: AtomicF32::new(0.75),
            halo: AtomicF32::new(0.35),
            size: AtomicF32::new(1.0),
            damping: AtomicF32::new(0.3),
            predelay_ms: AtomicF32::new(25.0),
            mix: AtomicF32::new(0.6),
        }
    }
}

pub struct DelayParams {
    pub trail: AtomicF32,
    pub time_l_ms: AtomicF32,
    pub time_r_ms: AtomicF32,
    pub crossfb: AtomicF32,
    pub mix: AtomicF32,
    pub fdn_send: AtomicF32,
}

impl Default for DelayParams {
    fn default() -> Self {
        Self {
            trail: AtomicF32::new(0.5),
            time_l_ms: AtomicF32::new(650.0),
            time_r_ms: AtomicF32::new(475.0),
            crossfb: AtomicF32::new(0.2),
            mix: AtomicF32::new(0.4),
            fdn_send: AtomicF32::new(0.5),  // Most delay feeds reverb
        }
    }
}

pub struct SpectralParams {
    // Legacy global controls (kept for compatibility)
    pub time_scale: AtomicF32,
    pub feedback: AtomicF32,
    pub mix: AtomicF32,
    pub fdn_send: AtomicF32,
    
    // Per-band arrays (8 bands: 100Hz, 200Hz, 400Hz, 800Hz, 1.6kHz, 3.2kHz, 6.4kHz, 12kHz)
    pub band_times_ms: [AtomicF32; 8],
    pub band_feedbacks: [AtomicF32; 8],
    
    // Macro shapers for playability
    pub time_slope: AtomicF32,      // -1.0 to 1.0: tilt time across bands (low→high or high→low)
    pub time_curve: AtomicF32,      // -1.0 to 1.0: exponential vs linear spacing
    pub feedback_slope: AtomicF32,  // -1.0 to 1.0: tilt feedback across bands
    pub feedback_curve: AtomicF32,  // -1.0 to 1.0: shape feedback distribution
    pub jitter: AtomicF32,          // 0.0 to 1.0: randomize per-band times slightly
    pub tilt: AtomicF32,            // -1.0 to 1.0: global spectral tilt (combines with slope)
}

impl Default for SpectralParams {
    fn default() -> Self {
        Self {
            time_scale: AtomicF32::new(1.0),
            feedback: AtomicF32::new(0.5),
            mix: AtomicF32::new(0.3),
            fdn_send: AtomicF32::new(0.6),
            
            // Default band times (150ms to 500ms, logarithmically spaced)
            band_times_ms: [
                AtomicF32::new(150.0),  // 100Hz
                AtomicF32::new(200.0),  // 200Hz
                AtomicF32::new(250.0),  // 400Hz
                AtomicF32::new(300.0),  // 800Hz
                AtomicF32::new(350.0),  // 1.6kHz
                AtomicF32::new(400.0),  // 3.2kHz
                AtomicF32::new(450.0),  // 6.4kHz
                AtomicF32::new(500.0),  // 12kHz
            ],
            
            // Default band feedbacks (0.5 for all bands)
            band_feedbacks: [
                AtomicF32::new(0.5),
                AtomicF32::new(0.5),
                AtomicF32::new(0.5),
                AtomicF32::new(0.5),
                AtomicF32::new(0.5),
                AtomicF32::new(0.5),
                AtomicF32::new(0.5),
                AtomicF32::new(0.5),
            ],
            
            // Macro shapers (neutral defaults)
            time_slope: AtomicF32::new(0.0),
            time_curve: AtomicF32::new(0.0),
            feedback_slope: AtomicF32::new(0.0),
            feedback_curve: AtomicF32::new(0.0),
            jitter: AtomicF32::new(0.0),
            tilt: AtomicF32::new(0.0),
        }
    }
}


pub struct GrainParams {
    pub density: AtomicF32,
    pub size_min_ms: AtomicF32,
    pub size_max_ms: AtomicF32,
    pub pitch_var: AtomicF32,
    pub spray_ms: AtomicF32,
    pub feedback: AtomicF32,
    pub filter_q: AtomicF32,
    pub mix: AtomicF32,
    // Send matrix
    pub delay_send: AtomicF32,
    pub spectral_send: AtomicF32,
    pub fdn_send: AtomicF32,
    // Navigation parameters (NEW)
    pub nav_mode: AtomicF32,              // 0.0=Static, 1.0=Scan, 2.0=Jump
    pub position_lfo_rate: AtomicF32,     // Hz for scan mode (0.001-0.5)
    pub position_lfo_depth: AtomicF32,    // 0.0-1.0, fraction of sample to scan
    pub spray_scale: AtomicF32,           // Multiplier for spray (5-50x for long samples)
}

impl Default for GrainParams {
    fn default() -> Self {
        Self {
            density: AtomicF32::new(5000.0),
            size_min_ms: AtomicF32::new(40.0),
            size_max_ms: AtomicF32::new(150.0),
            pitch_var: AtomicF32::new(4.0),
            spray_ms: AtomicF32::new(8.0),
            feedback: AtomicF32::new(0.5),
            filter_q: AtomicF32::new(0.55),
            mix: AtomicF32::new(0.5),
            // Send matrix defaults
            delay_send: AtomicF32::new(1.0),      // Full signal
            spectral_send: AtomicF32::new(0.5),   // Parallel texture
            fdn_send: AtomicF32::new(0.2),        // Existing default
            // Navigation defaults (NEW)
            nav_mode: AtomicF32::new(1.0),              // Scan mode (best for long samples)
            position_lfo_rate: AtomicF32::new(0.05),    // 20-second scan cycle
            position_lfo_depth: AtomicF32::new(0.3),    // Explore 30% of sample
            spray_scale: AtomicF32::new(10.0),          // 10x larger spray (80ms → 800ms)
        }
    }
}

pub struct MasterParams {
    pub color: AtomicF32,
    pub lfo_depth: AtomicF32,
    pub lfo_rate: AtomicF32,
    pub ir_freeze: AtomicF32,
    pub dry_wet: AtomicF32,
}

impl Default for MasterParams {
    fn default() -> Self {
        Self {
            color: AtomicF32::new(-1.0),
            lfo_depth: AtomicF32::new(3.0),
            lfo_rate: AtomicF32::new(0.16),
            ir_freeze: AtomicF32::new(0.2),
            dry_wet: AtomicF32::new(0.7),
        }
    }
}

// ===================== UI Messages =====================

pub enum UiMessage {
    LoadSample(Vec<f32>),
    UpdateFdnConfig(f32, f32, f32),
    SetPlaying(bool),
}

// ... (rest of types.rs - utility functions, WAV loading, etc.)

// ===================== Utility Functions =====================

#[inline]
pub fn ftz_daz() {
    #[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
    {
        #[cfg(target_arch = "x86")]
        use std::arch::x86::{_MM_FLUSH_ZERO_ON, _MM_SET_FLUSH_ZERO_MODE};
        #[cfg(target_arch = "x86_64")]
        use std::arch::x86_64::{_MM_FLUSH_ZERO_ON, _MM_SET_FLUSH_ZERO_MODE};

        unsafe {
            _MM_SET_FLUSH_ZERO_MODE(_MM_FLUSH_ZERO_ON);
        }
    }
}

#[inline]
pub fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a + t * (b - a)
}

#[inline]
pub fn soft_clip(x: f32, threshold: f32) -> f32 {
    if x.abs() <= threshold {
        x
    } else {
        let excess = x.abs() - threshold;
        let headroom = 1.0 - threshold;
        x.signum() * (threshold + headroom * (1.0 - (-excess / headroom).exp()))
    }
}

// ===================== WAV Loading =====================

pub fn load_wav_mono(path: &str) -> Result<Vec<f32>> {
    let reader = hound::WavReader::open(path)?;
    let spec = reader.spec();
    let sr_in = spec.sample_rate as usize;

    let mut samples: Vec<f32> = match (spec.sample_format, spec.bits_per_sample) {
        (hound::SampleFormat::Float, _) => reader
            .into_samples::<f32>()
            .map(|s| s.map(|v| v))
            .collect::<Result<Vec<_>, _>>()?,
        (hound::SampleFormat::Int, 0..=16) => {
            let scale = 1.0 / 32768.0;
            reader
                .into_samples::<i16>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<Vec<_>, _>>()?
        }
        (hound::SampleFormat::Int, 24) => {
            let scale = 1.0 / (1i32 << 23) as f32;
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|v| (v >> 8) as f32 * scale))
                .collect::<Result<Vec<_>, _>>()?
        }
        (hound::SampleFormat::Int, _) => {
            let scale = 1.0 / (i32::MAX as f32);
            reader
                .into_samples::<i32>()
                .map(|s| s.map(|v| v as f32 * scale))
                .collect::<Result<Vec<_>, _>>()?
        }
    };

    if spec.channels > 1 {
        samples = samples
            .chunks_exact(spec.channels as usize)
            .map(|c| c.iter().sum::<f32>() / c.len() as f32)
            .collect::<Vec<_>>();
    }

    if sr_in != SR {
        let ratio = SR as f32 / sr_in as f32;
        let out_len = (samples.len() as f32 * ratio) as usize;
        let mut out = vec![0.0f32; out_len];
        for i in 0..out_len {
            let x = i as f32 / ratio;
            let i0 = x.floor() as usize;
            let t = x - i0 as f32;
            let i1 = (i0 + 1).min(samples.len() - 1);
            out[i] = samples[i0] + t * (samples[i1] - samples[i0]);
        }
        Ok(out)
    } else {
        Ok(samples)
    }
}