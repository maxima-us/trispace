//! Audio Engine Core - Uses AudioGraph

use crate::routing::AudioGraph;
use crate::types::*;
use crossbeam_channel as spsc;
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rand::{Rng, SeedableRng};
use rand_pcg::Pcg64;
use rustfft::{num_complex::Complex, FftPlanner};  // ← ADDED THIS
use std::f32::consts::PI;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};  // ← ADDED THIS

pub struct Engine {
    params: Arc<GlobalParams>,
    graph: AudioGraph,

    // Communication channels
    ui_rx: spsc::Receiver<UiMessage>,
    ir_audio_tx: spsc::Sender<[f32; BLOCK]>,
    ir_taps_rx: spsc::Receiver<[f32; CONV_HEAD_TAPS]>,

    // State
    is_playing: Arc<AtomicBool>,
}

impl Engine {
    fn new(
        params: Arc<GlobalParams>,
        sample: Vec<f32>,
        ui_rx: spsc::Receiver<UiMessage>,
        ir_audio_tx: spsc::Sender<[f32; BLOCK]>,
        ir_taps_rx: spsc::Receiver<[f32; CONV_HEAD_TAPS]>,
        is_playing: Arc<AtomicBool>,
    ) -> Self {
        let mut graph = AudioGraph::new(&params);
        graph.grains.set_sample(sample);

        Self {
            params,
            graph,
            ui_rx,
            ir_audio_tx,
            ir_taps_rx,
            is_playing,
        }
    }

    fn handle_ui_messages(&mut self) {
        while let Ok(msg) = self.ui_rx.try_recv() {
            match msg {
                UiMessage::LoadSample(s) => self.graph.grains.set_sample(s),
                UiMessage::UpdateFdnConfig(size, damp, pre) => {
                    self.graph.fdn.update_config(size, damp, pre);
                }
                UiMessage::SetPlaying(state) => {
                    self.is_playing.store(state, Ordering::Release);
                }
            }
        }
    }

    fn render_block(&mut self, out: &mut [f32]) {
        self.handle_ui_messages();

        // Process taps from IR worker
        while let Ok(new_taps) = self.ir_taps_rx.try_recv() {
            self.graph.conv.update_taps(&new_taps);
        }

        // THE ENTIRE AUDIO PATH IS IN ONE CALL
        let ir_scratch = self.graph.process_block(
            out,
            &self.params,
            self.is_playing.load(Ordering::Acquire),
        );

        // Send to IR worker
        let _ = self.ir_audio_tx.try_send(ir_scratch);
    }
}

// ===================== IR Worker Thread =====================

fn start_ir_worker(
    params: Arc<GlobalParams>,
    rx: spsc::Receiver<[f32; BLOCK]>,
    tx: spsc::Sender<[f32; CONV_HEAD_TAPS]>,
) {
    std::thread::spawn(move || {

        // FIX: Enable FTZ/DAZ in worker thread (FFT/IFFT can generate denormals)
        ftz_daz();

        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(FFT_SIZE);
        let ifft = planner.plan_fft_inverse(FFT_SIZE);
        let mut rng = Pcg64::from_seed([3; 32]);

        let mut fft_buf = vec![Complex::default(); FFT_SIZE];
        let mut ifft_buf = vec![Complex::default(); FFT_SIZE];
        let mut spectral_mag = vec![0.0f32; FFT_SIZE / 2];
        let mut frozen_phases = vec![0.0f32; FFT_SIZE / 2];

        let hann_win: Vec<f32> = (0..FFT_SIZE)
            .map(|i| 0.5 - 0.5 * (2.0 * PI * i as f32 / (FFT_SIZE as f32 - 1.0)).cos())
            .collect();

        let norm = (FFT_SIZE as f32).recip();
        let ola_gain = 2.0;

        let mut last_update = Instant::now();
        let mut phases_initialized = false;

        loop {
            if let Ok(block) = rx.recv_timeout(Duration::from_millis(10)) {
                for i in 0..BLOCK {
                    fft_buf[i] = Complex::new(block[i] * hann_win[i], 0.0);
                }
                for i in BLOCK..FFT_SIZE {
                    fft_buf[i] = Complex::default();
                }
                fft.process(&mut fft_buf);

                let freeze_mix = params.master.ir_freeze.load(Ordering::Relaxed);

                for i in 0..spectral_mag.len() {
                    let mag =
                        (fft_buf[i].re * fft_buf[i].re + fft_buf[i].im * fft_buf[i].im).sqrt();
                    spectral_mag[i] = freeze_mix * spectral_mag[i] + (1.0 - freeze_mix) * mag;

                    if !phases_initialized || freeze_mix < 0.5 {
                        frozen_phases[i] = fft_buf[i].im.atan2(fft_buf[i].re);
                    }
                }

                if !phases_initialized {
                    phases_initialized = true;
                }

                if last_update.elapsed() > Duration::from_millis(25) {
                    for i in 0..FFT_SIZE {
                        ifft_buf[i] = Complex::default();
                    }
                    ifft_buf[0] = Complex::new(spectral_mag[0], 0.0);

                    for i in 1..(spectral_mag.len() - 1) {
                        let phi = if freeze_mix > 0.5 {
                            frozen_phases[i]
                        } else {
                            rng.gen::<f32>() * 2.0 * PI
                        };

                        let (s, c) = phi.sin_cos();
                        ifft_buf[i] = Complex::new(spectral_mag[i] * c, spectral_mag[i] * s);
                        ifft_buf[FFT_SIZE - i] =
                            Complex::new(spectral_mag[i] * c, -spectral_mag[i] * s);
                    }
                    ifft_buf[spectral_mag.len() - 1] =
                        Complex::new(*spectral_mag.last().unwrap(), 0.0);

                    ifft.process(&mut ifft_buf);

                    let mut new_taps = [0.0f32; CONV_HEAD_TAPS];
                    for i in 0..CONV_HEAD_TAPS {
                        new_taps[i] = ifft_buf[i].re * norm * hann_win[i] * ola_gain;
                    }

                    let _ = tx.try_send(new_taps);
                    last_update = Instant::now();
                }
            }
        }
    });
}

// ===================== Engine Handle =====================

pub struct EngineHandle {
    ui_tx: spsc::Sender<UiMessage>,
    is_playing: Arc<AtomicBool>,
    _stream: cpal::Stream,
}

impl EngineHandle {
    pub fn send_message(&self, msg: UiMessage) {
        let _ = self.ui_tx.try_send(msg);
    }

    pub fn is_playing(&self) -> bool {
        self.is_playing.load(Ordering::Acquire)
    }

    pub fn set_playing(&self, state: bool) {
        self.is_playing.store(state, Ordering::Release);
        let _ = self.ui_tx.try_send(UiMessage::SetPlaying(state));
    }
}

// ===================== Start Engine =====================

pub fn start_engine(params: Arc<GlobalParams>) -> EngineHandle {

    // FIX: Enable flush-to-zero and denormals-are-zero (prevents denormal performance hit)
    ftz_daz();

    let (ui_tx, ui_rx) = spsc::bounded::<UiMessage>(4);
    let (ir_audio_tx, ir_audio_rx) = spsc::bounded::<[f32; BLOCK]>(2);
    let (ir_taps_tx, ir_taps_rx) = spsc::bounded::<[f32; CONV_HEAD_TAPS]>(2);

    start_ir_worker(params.clone(), ir_audio_rx, ir_taps_tx);

    let sample = vec![0.0; SR];
    let is_playing = Arc::new(AtomicBool::new(false));

    let host = cpal::default_host();
    let dev = host
        .default_output_device()
        .expect("No output device");
    let mut cfg = dev
        .default_output_config()
        .expect("No default config")
        .config();
    cfg.channels = 2;
    cfg.sample_rate = cpal::SampleRate(SR as u32);

    let mut engine = Engine::new(
        params.clone(),
        sample,
        ui_rx,
        ir_audio_tx,
        ir_taps_rx,
        is_playing.clone(),
    );

    let stream = dev
        .build_output_stream(
            &cfg,
            move |data: &mut [f32], _| {
                
                // FIX: Enable FTZ/DAZ in audio thread (CPU mode not inherited from parent)
                ftz_daz();

                let num_frames = data.len() / 2;
                let mut rendered = 0;
                while rendered < num_frames {
                    let to_render = (num_frames - rendered).min(BLOCK);
                    let start = rendered * 2;
                    let end = start + to_render * 2;
                    engine.render_block(&mut data[start..end]);
                    rendered += to_render;
                }
            },
            move |err| eprintln!("Audio error: {err}"),
            None,
        )
        .expect("Failed to build stream");

    stream.play().expect("Failed to play stream");

    EngineHandle {
        ui_tx,
        is_playing,
        _stream: stream,
    }
}