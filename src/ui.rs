//! GUI Application Layer

use crate::engine::EngineHandle;
use crate::types::*;
use eframe::egui;
use std::sync::atomic::Ordering;
use std::sync::Arc;

pub struct TriSpaceApp {
    params: Arc<GlobalParams>,
    engine_handle: EngineHandle,
    sample_loaded: bool,
}

impl TriSpaceApp {
    fn new(_cc: &eframe::CreationContext<'_>) -> Self {
        let params = Arc::new(GlobalParams::default());
        
        // Set musical defaults
		params.reverb.bloom.store(0.75, Ordering::Relaxed);
		params.reverb.halo.store(0.75, Ordering::Relaxed);
		params.reverb.size.store(1.0, Ordering::Relaxed);
		params.reverb.damping.store(0.3, Ordering::Relaxed);
		params.reverb.predelay_ms.store(25.0, Ordering::Relaxed);
		params.reverb.mix.store(0.6, Ordering::Relaxed);

        params.spectral.time_scale.store(1.0, Ordering::Relaxed);  // ← ADD THESE 3 LINES
		params.spectral.feedback.store(0.5, Ordering::Relaxed);
		params.spectral.mix.store(0.3, Ordering::Relaxed);

		params.delay.trail.store(0.3, Ordering::Relaxed);
		params.delay.time_l_ms.store(950.0, Ordering::Relaxed);
		params.delay.time_r_ms.store(675.0, Ordering::Relaxed);
		params.delay.crossfb.store(0.2, Ordering::Relaxed);
		params.delay.mix.store(0.3, Ordering::Relaxed);

		params.grains.density.store(3000.0, Ordering::Relaxed);
		params.grains.size_min_ms.store(75.0, Ordering::Relaxed);
		params.grains.size_max_ms.store(150.0, Ordering::Relaxed);
		params.grains.pitch_var.store(0.0, Ordering::Relaxed);
		params.grains.spray_ms.store(50.0, Ordering::Relaxed);
		params.grains.feedback.store(0.3, Ordering::Relaxed);
		params.grains.filter_q.store(0.3, Ordering::Relaxed);
		params.grains.mix.store(0.5, Ordering::Relaxed);

		params.master.color.store(1.0, Ordering::Relaxed);
		params.master.lfo_depth.store(3.0, Ordering::Relaxed);
		params.master.lfo_rate.store(0.16, Ordering::Relaxed);
		params.master.ir_freeze.store(0.2, Ordering::Relaxed);
		params.master.dry_wet.store(0.7, Ordering::Relaxed);

        // Send matrix defaults
        params.grains.delay_send.store(0.3, Ordering::Relaxed);
        params.grains.spectral_send.store(0.75, Ordering::Relaxed);
        params.grains.fdn_send.store(0.3, Ordering::Relaxed);
        params.delay.fdn_send.store(0.7, Ordering::Relaxed);
        params.spectral.fdn_send.store(0.6, Ordering::Relaxed);

        // Navigation defaults (NEW)
        params.grains.nav_mode.store(1.0, Ordering::Relaxed);              // Scan mode
        params.grains.position_lfo_rate.store(0.02, Ordering::Relaxed);     // 20-second cycle
        params.grains.position_lfo_depth.store(0.9, Ordering::Relaxed);     // 30% of sample
        params.grains.spray_scale.store(15.0, Ordering::Relaxed);           // 10x larger spray

        let engine_handle = crate::engine::start_engine(params.clone());

        Self {
            params,
            engine_handle,
            sample_loaded: false,
        }
    }
}

impl eframe::App for TriSpaceApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        use std::sync::atomic::Ordering::Relaxed;
        
        // Keyboard input handling
        ctx.input(|i| {
            if i.key_pressed(egui::Key::Space) {
                let new_state = !self.engine_handle.is_playing();
                self.engine_handle.set_playing(new_state);
            }
        });
        
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("🎛️ Tri-Space Ambient Engine v3.1 PROFESSIONAL");
            
            // Playback status indicator
            ui.horizontal(|ui| {
                if self.engine_handle.is_playing() {
                    ui.colored_label(egui::Color32::GREEN, "▶ PLAYING");
                } else {
                    ui.colored_label(egui::Color32::GRAY, "⏸ PAUSED");
                }
                ui.label("(Press SPACE to toggle)");
            });
            
            ui.separator();
            
            // File loader
            ui.horizontal(|ui| {
                ui.label("Sample:");
                if ui.button("Load WAV...").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("WAV Audio", &["wav"])
                        .pick_file()
                    {
                        if let Ok(sample) = load_wav_mono(path.to_str().unwrap()) {
                            self.engine_handle.send_message(UiMessage::LoadSample(sample));
                            self.sample_loaded = true;
                        }
                    }
                }
                if self.sample_loaded {
                    ui.label("✓ Loaded");
                }
            });
            
            ui.separator();
            
            egui::ScrollArea::vertical().show(ui, |ui| {


                // REVERB section
                ui.collapsing("🌊 REVERB", |ui| {
					let mut bloom = self.params.reverb.bloom.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Bloom (feedback):");
                        if ui.add(egui::Slider::new(&mut bloom, 0.0..=0.985).step_by(0.01)).changed() {
                            self.params.reverb.bloom.store(bloom, Relaxed);
                        }
                    });
                    
                    let mut halo = self.params.reverb.halo.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Halo (shimmer):");
                        if ui.add(egui::Slider::new(&mut halo, 0.0..=2.0).step_by(0.01)).changed() {
                            self.params.reverb.halo.store(halo, Relaxed);
                        }
                    });
                    
                    let mut size = self.params.reverb.size.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Size:");
                        if ui.add(egui::Slider::new(&mut size, 0.5..=5.0).step_by(0.01)).changed() {
                            self.params.reverb.size.store(size, Relaxed);
                            let damp = self.params.reverb.damping.load(Relaxed);
                            let pre = self.params.reverb.predelay_ms.load(Relaxed);
                            self.engine_handle.send_message(UiMessage::UpdateFdnConfig(size, damp, pre));
                        }
                    });
                    
                    let mut damping = self.params.reverb.damping.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Damping (HF absorb):");
                        if ui.add(egui::Slider::new(&mut damping, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.reverb.damping.store(damping, Relaxed);
                            let size = self.params.reverb.size.load(Relaxed);
                            let pre = self.params.reverb.predelay_ms.load(Relaxed);
                            self.engine_handle.send_message(UiMessage::UpdateFdnConfig(size, damping, pre));
                        }
                    });
                    
                    let mut predelay = self.params.reverb.predelay_ms.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Predelay (ms):");
                        if ui.add(egui::Slider::new(&mut predelay, 0.0..=500.0).step_by(1.0)).changed() {
                            self.params.reverb.predelay_ms.store(predelay, Relaxed);
                            let size = self.params.reverb.size.load(Relaxed);
                            let damp = self.params.reverb.damping.load(Relaxed);
                            self.engine_handle.send_message(UiMessage::UpdateFdnConfig(size, damp, predelay));
                        }
                    });
                    
                    let mut rev_mix = self.params.reverb.mix.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Mix:");
                        if ui.add(egui::Slider::new(&mut rev_mix, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.reverb.mix.store(rev_mix, Relaxed);
                        }
                    });
                });
                
                ui.separator();
                
                // DELAY section
                ui.collapsing("⏱️ DELAY", |ui| {
                    let mut trail = self.params.delay.trail.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Trail (feedback):");
                        if ui.add(egui::Slider::new(&mut trail, 0.0..=0.92).step_by(0.01)).changed() {
                            self.params.delay.trail.store(trail, Relaxed);
                        }
                    });
                    
                    let mut time_l = self.params.delay.time_l_ms.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Time L (ms):");
                        if ui.add(egui::Slider::new(&mut time_l, 100.0..=2000.0).step_by(10.0)).changed() {
                            self.params.delay.time_l_ms.store(time_l, Relaxed);
                        }
                    });
                    
                    let mut time_r = self.params.delay.time_r_ms.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Time R (ms):");
                        if ui.add(egui::Slider::new(&mut time_r, 100.0..=2000.0).step_by(10.0)).changed() {
                            self.params.delay.time_r_ms.store(time_r, Relaxed);
                        }
                    });
                    
                    let mut crossfb = self.params.delay.crossfb.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Crossfeedback (ping-pong):");
                        if ui.add(egui::Slider::new(&mut crossfb, 0.0..=0.92).step_by(0.01)).changed() {
                            self.params.delay.crossfb.store(crossfb, Relaxed);
                        }
                    });
                    
                    let mut dly_mix = self.params.delay.mix.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Mix:");
                        if ui.add(egui::Slider::new(&mut dly_mix, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.delay.mix.store(dly_mix, Relaxed);
                        }
                    });
                });
                
                ui.separator();


                // SPECTRAL DELAY section
                
                ui.collapsing("🌈 SPECTRAL DELAY (8-Band)", |ui| {
                    ui.label("🎛️ MACRO SHAPERS (Playability)");
                    ui.separator();
                    
                    // Time shapers
                    let mut time_slope = self.params.spectral.time_slope.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Time Slope:");
                        if ui.add(egui::Slider::new(&mut time_slope, -1.0..=1.0).step_by(0.01)).changed() {
                            self.params.spectral.time_slope.store(time_slope, Relaxed);
                        }
                    });
                    ui.label("  ↳ -1.0 = longer at bass, +1.0 = longer at treble");
                    
                    let mut time_curve = self.params.spectral.time_curve.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Time Curve:");
                        if ui.add(egui::Slider::new(&mut time_curve, -1.0..=1.0).step_by(0.01)).changed() {
                            self.params.spectral.time_curve.store(time_curve, Relaxed);
                        }
                    });
                    ui.label("  ↳ -1.0 = exponential, +1.0 = S-curve");
                    
                    ui.separator();
                    
                    // Feedback shapers
                    let mut fb_slope = self.params.spectral.feedback_slope.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Feedback Slope:");
                        if ui.add(egui::Slider::new(&mut fb_slope, -1.0..=1.0).step_by(0.01)).changed() {
                            self.params.spectral.feedback_slope.store(fb_slope, Relaxed);
                        }
                    });
                    ui.label("  ↳ -1.0 = more at bass, +1.0 = more at treble");
                    
                    let mut fb_curve = self.params.spectral.feedback_curve.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Feedback Curve:");
                        if ui.add(egui::Slider::new(&mut fb_curve, -1.0..=1.0).step_by(0.01)).changed() {
                            self.params.spectral.feedback_curve.store(fb_curve, Relaxed);
                        }
                    });
                    ui.label("  ↳ Shape feedback distribution across bands");
                    
                    ui.separator();
                    
                    // Global modifiers
                    let mut jitter = self.params.spectral.jitter.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Jitter:");
                        if ui.add(egui::Slider::new(&mut jitter, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.spectral.jitter.store(jitter, Relaxed);
                        }
                    });
                    ui.label("  ↳ Randomize timing for organic texture");
                    
                    let mut tilt = self.params.spectral.tilt.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Tilt:");
                        if ui.add(egui::Slider::new(&mut tilt, -1.0..=1.0).step_by(0.01)).changed() {
                            self.params.spectral.tilt.store(tilt, Relaxed);
                        }
                    });
                    ui.label("  ↳ Global spectral tilt (combines with slopes)");
                    
                    ui.separator();
                    
                    // Mix
                    let mut spectral_mix = self.params.spectral.mix.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Mix:");
                        if ui.add(egui::Slider::new(&mut spectral_mix, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.spectral.mix.store(spectral_mix, Relaxed);
                        }
                    });
                    
                    ui.separator();
                    
                    // Per-band precision controls (collapsible advanced section)
                    ui.collapsing("⚙️ PER-BAND PRECISION (Advanced)", |ui| {
                        ui.label("🎚️ Fine-tune individual bands:");
                        ui.separator();
                        
                        let band_labels = [
                            "100 Hz (Sub/Bass)",
                            "200 Hz (Bass)",
                            "400 Hz (Low Mids)",
                            "800 Hz (Mids)",
                            "1.6 kHz (Upper Mids)",
                            "3.2 kHz (Presence)",
                            "6.4 kHz (Brilliance)",
                            "12 kHz (Air)",
                        ];
                        
                        for (i, label) in band_labels.iter().enumerate() {
                            ui.horizontal(|ui| {
                                ui.label(*label);
                            });
                            
                            let mut time = self.params.spectral.band_times_ms[i].load(Relaxed);
                            ui.horizontal(|ui| {
                                ui.label("  Time (ms):");
                                if ui.add(egui::Slider::new(&mut time, 50.0..=2000.0).step_by(10.0)).changed() {
                                    self.params.spectral.band_times_ms[i].store(time, Relaxed);
                                }
                            });
                            
                            let mut fb = self.params.spectral.band_feedbacks[i].load(Relaxed);
                            ui.horizontal(|ui| {
                                ui.label("  Feedback:");
                                if ui.add(egui::Slider::new(&mut fb, 0.0..=0.92).step_by(0.01)).changed() {
                                    self.params.spectral.band_feedbacks[i].store(fb, Relaxed);
                                }
                            });
                            
                            ui.separator();
                        }
                        
                        ui.label("💡 Macro shapers modify these base values");
                    });
                    
                    ui.separator();
                    ui.label("💡 8-band parallel delay with macro control + per-band precision");
                    ui.label("🎨 Use macros for playability, per-band for surgical control");
                });
                
                ui.separator();

                
                // GRAINS section
                ui.collapsing("✨ GRAINS", |ui| {
                    let mut density = self.params.grains.density.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Density (grains/sec):");
                        if ui.add(egui::Slider::new(&mut density, 100.0..=5000.0).step_by(10.0)).changed() {
                            self.params.grains.density.store(density, Relaxed);
                        }
                    });
                    
                    let mut size_min = self.params.grains.size_min_ms.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Size Min (ms):");
                        if ui.add(egui::Slider::new(&mut size_min, 10.0..=200.0).step_by(5.0)).changed() {
                            self.params.grains.size_min_ms.store(size_min, Relaxed);
                        }
                    });
                    
                    let mut size_max = self.params.grains.size_max_ms.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Size Max (ms):");
                        if ui.add(egui::Slider::new(&mut size_max, 50.0..=1000.0).step_by(10.0)).changed() {
                            self.params.grains.size_max_ms.store(size_max, Relaxed);
                        }
                    });
                    
                    let mut pitch_var = self.params.grains.pitch_var.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Pitch Variance (±semitones):");
                        if ui.add(egui::Slider::new(&mut pitch_var, 0.0..=24.0).step_by(0.5)).changed() {
                            self.params.grains.pitch_var.store(pitch_var, Relaxed);
                        }
                    });
                    
                    let mut spray = self.params.grains.spray_ms.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Spray (timing jitter ms):");
                        if ui.add(egui::Slider::new(&mut spray, 0.0..=100.0).step_by(1.0)).changed() {
                            self.params.grains.spray_ms.store(spray, Relaxed);
                        }
                    });
                    
                    let mut feedback = self.params.grains.feedback.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Feedback (live buffer):");
                        if ui.add(egui::Slider::new(&mut feedback, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.grains.feedback.store(feedback, Relaxed);
                        }
                    });
                    
                    let mut filter_q = self.params.grains.filter_q.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Filter Q (tone):");
                        if ui.add(egui::Slider::new(&mut filter_q, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.grains.filter_q.store(filter_q, Relaxed);
                        }
                    });
                    
                    let mut gr_mix = self.params.grains.mix.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Mix:");
                        if ui.add(egui::Slider::new(&mut gr_mix, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.grains.mix.store(gr_mix, Relaxed);
                        }
                    });


                });
                
                ui.separator();
                    
                // ========== NAVIGATION CONTROLS (NEW) ==========
                ui.collapsing("🧭 NAVIGATION (Sample Exploration)", |ui| {
                    ui.label("💡 For long samples (30s+): use Scan or Jump mode");
                    
                    let mut nav_mode = self.params.grains.nav_mode.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Mode:");
                        if ui.add(egui::Slider::new(&mut nav_mode, 0.0..=2.0).step_by(1.0)
                            .custom_formatter(|n, _| {
                                match n as i32 {
                                    0 => "Static".to_string(),
                                    1 => "Scan".to_string(),
                                    _ => "Jump".to_string(),
                                }
                            })).changed() {
                            self.params.grains.nav_mode.store(nav_mode, Relaxed);
                        }
                    });
                    ui.label("  ↳ Static=one point | Scan=LFO traverse | Jump=random leaps");
                    
                    // Scan mode controls (only visible when active)
                    if nav_mode >= 0.5 && nav_mode < 1.5 {
                        ui.separator();
                        ui.label("📡 Scan Mode Settings:");
                        
                        let mut pos_lfo_rate = self.params.grains.position_lfo_rate.load(Relaxed);
                        ui.horizontal(|ui| {
                            ui.label("Scan Rate (Hz):");
                            if ui.add(egui::Slider::new(&mut pos_lfo_rate, 0.001..=0.5)
                                .step_by(0.001)
                                .logarithmic(true)).changed() {
                                self.params.grains.position_lfo_rate.store(pos_lfo_rate, Relaxed);
                            }
                        });
                        ui.label(format!("  ↳ Period: {:.1}s (time to traverse range)", 1.0 / pos_lfo_rate.max(0.001)));
                        
                        let mut pos_lfo_depth = self.params.grains.position_lfo_depth.load(Relaxed);
                        ui.horizontal(|ui| {
                            ui.label("Scan Range:");
                            if ui.add(egui::Slider::new(&mut pos_lfo_depth, 0.0..=1.0).step_by(0.01)).changed() {
                                self.params.grains.position_lfo_depth.store(pos_lfo_depth, Relaxed);
                            }
                        });
                        ui.label("  ↳ Fraction of sample to explore (0.3 = 30%)");
                    }
                    
                    ui.separator();
                    let mut spray_scale = self.params.grains.spray_scale.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Spray Scale (×):");
                        if ui.add(egui::Slider::new(&mut spray_scale, 1.0..=50.0)
                            .step_by(0.5)
                            .logarithmic(true)).changed() {
                            self.params.grains.spray_scale.store(spray_scale, Relaxed);
                        }
                    });
                    let effective_spray_ms = self.params.grains.spray_ms.load(Relaxed) * spray_scale;
                    ui.label(format!("  ↳ Effective spray window: {:.0}ms ({:.1}% of 30s sample)", 
                        effective_spray_ms,
                        (effective_spray_ms / 30000.0) * 100.0
                    ));
                    ui.label("  ↳ Higher = wider exploration around position");

                });
                
                ui.separator();


                // ENVELOPE FOLLOWER section
                ui.collapsing("📊 ENVELOPE FOLLOWER (Dynamic Response)", |ui| {
                    ui.label("💡 Make reverb/delay 'breathe' with input energy");
                    ui.separator();

                    let mut enabled = self.params.envelope.enabled.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Depth:");
                        if ui.add(egui::Slider::new(&mut enabled, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.envelope.enabled.store(enabled, Relaxed);
                        }
                    });
                    ui.label("  ↳ 0.0 = off, 1.0 = maximum response");
                    
                    ui.separator();
                    ui.heading("Attack Times (ms):");
                    
                    let mut attack_low = self.params.envelope.attack_low.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Low (< 250Hz):");
                        if ui.add(egui::Slider::new(&mut attack_low, 1.0..=100.0).step_by(1.0)).changed() {
                            self.params.envelope.attack_low.store(attack_low, Relaxed);
                        }
                    });
                    
                    let mut attack_mid = self.params.envelope.attack_mid.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Mid (250Hz-2.5kHz):");
                        if ui.add(egui::Slider::new(&mut attack_mid, 1.0..=100.0).step_by(1.0)).changed() {
                            self.params.envelope.attack_mid.store(attack_mid, Relaxed);
                        }
                    });
                    
                    let mut attack_high = self.params.envelope.attack_high.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("High (> 2.5kHz):");
                        if ui.add(egui::Slider::new(&mut attack_high, 1.0..=100.0).step_by(1.0)).changed() {
                            self.params.envelope.attack_high.store(attack_high, Relaxed);
                        }
                    });
                    
                    ui.separator();


                    ui.heading("Release Times (ms):");
                    
                    let mut release_low = self.params.envelope.release_low.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Low:");
                        if ui.add(egui::Slider::new(&mut release_low, 10.0..=500.0).step_by(10.0)).changed() {
                            self.params.envelope.release_low.store(release_low, Relaxed);
                        }
                    });
                    
                    let mut release_mid = self.params.envelope.release_mid.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Mid:");
                        if ui.add(egui::Slider::new(&mut release_mid, 10.0..=500.0).step_by(10.0)).changed() {
                            self.params.envelope.release_mid.store(release_mid, Relaxed);
                        }
                    });
                    
                    let mut release_high = self.params.envelope.release_high.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("High:");
                        if ui.add(egui::Slider::new(&mut release_high, 10.0..=500.0).step_by(10.0)).changed() {
                            self.params.envelope.release_high.store(release_high, Relaxed);
                        }
                    });
                    
                    ui.separator();
                    ui.heading("Modulation Targets:");
                    
                    let mut to_reverb = self.params.envelope.to_reverb_send.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("→ Reverb Send:");
                        if ui.add(egui::Slider::new(&mut to_reverb, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.envelope.to_reverb_send.store(to_reverb, Relaxed);
                        }
                    });
                    
                    let mut to_delay = self.params.envelope.to_delay_send.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("→ Delay Send:");
                        if ui.add(egui::Slider::new(&mut to_delay, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.envelope.to_delay_send.store(to_delay, Relaxed);
                        }
                    });
                    
                    // let mut to_density = self.params.envelope.to_density.load(Relaxed);
                    // ui.horizontal(|ui| {
                    //     ui.label("→ Grain Density:");
                    //     if ui.add(egui::Slider::new(&mut to_density, 0.0..=1.0).step_by(0.01)).changed() {
                    //         self.params.envelope.to_density.store(to_density, Relaxed);
                    //     }
                    // });
                    
                    ui.separator();
                    ui.heading("Response Curve:");
                    
                    let mut curve = self.params.envelope.curve.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Curve:");
                        if ui.add(egui::Slider::new(&mut curve, -1.0..=1.0).step_by(0.01)).changed() {
                            self.params.envelope.curve.store(curve, Relaxed);
                        }
                    });
                    ui.label("  ↳ -1.0 = exponential (sensitive), 0.0 = linear, +1.0 = compressed");
                    
                    let mut minimum = self.params.envelope.minimum.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Minimum:");
                        if ui.add(egui::Slider::new(&mut minimum, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.envelope.minimum.store(minimum, Relaxed);
                        }
                    });
                    ui.label("  ↳ Prevents complete silence (floor level)");
                    
                    ui.separator();
                    ui.label("🎚️ 3-band envelope tracking: Low/Mid/High");
                    ui.label("🌬️ Space breathes with musical dynamics");
                });
                
                
                ui.separator();


                // ROUTING section (collapsible)
                ui.collapsing("🔀 ROUTING (Advanced)", |ui| {
                    ui.label("💡 Control signal flow between processors");
                    
                    ui.separator();
                    ui.heading("Grains Send To:");
                    
                    let mut grain_delay_send = self.params.grains.delay_send.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("→ Delay:");
                        if ui.add(egui::Slider::new(&mut grain_delay_send, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.grains.delay_send.store(grain_delay_send, Relaxed);
                        }
                    });
                    
                    let mut grain_spectral_send = self.params.grains.spectral_send.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("→ Spectral:");
                        if ui.add(egui::Slider::new(&mut grain_spectral_send, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.grains.spectral_send.store(grain_spectral_send, Relaxed);
                        }
                    });
                    
                    let mut grain_fdn_send = self.params.grains.fdn_send.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("→ Reverb (FDN):");
                        if ui.add(egui::Slider::new(&mut grain_fdn_send, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.grains.fdn_send.store(grain_fdn_send, Relaxed);
                        }
                    });
                    
                    ui.separator();
                    ui.heading("Delay Send To:");
                    
                    let mut delay_fdn_send = self.params.delay.fdn_send.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("→ Reverb (FDN):");
                        if ui.add(egui::Slider::new(&mut delay_fdn_send, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.delay.fdn_send.store(delay_fdn_send, Relaxed);
                        }
                    });
                    
                    ui.separator();
                    ui.heading("Spectral Send To:");
                    
                    let mut spectral_fdn_send = self.params.spectral.fdn_send.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("→ Reverb (FDN):");
                        if ui.add(egui::Slider::new(&mut spectral_fdn_send, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.spectral.fdn_send.store(spectral_fdn_send, Relaxed);
                        }
                    });
                    
                    ui.separator();
                    ui.label("ℹ️ These sends create complex feedback networks");
                    ui.label("🎚️ Start with defaults, then experiment!");
                });
                
                ui.separator();
                
                // GLOBAL section
                ui.collapsing("🌍 GLOBAL", |ui| {
                    let mut color = self.params.master.color.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Color (tilt EQ dB):");
                        if ui.add(egui::Slider::new(&mut color, -12.0..=12.0).step_by(0.5)).changed() {
                            self.params.master.color.store(color, Relaxed);
                        }
                    });
                    
                    let mut lfo_depth = self.params.master.lfo_depth.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("LFO Depth (ms):");
                        if ui.add(egui::Slider::new(&mut lfo_depth, 0.0..=20.0).step_by(0.5)).changed() {
                            self.params.master.lfo_depth.store(lfo_depth, Relaxed);
                        }
                    });
                    
                    let mut lfo_rate = self.params.master.lfo_rate.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("LFO Rate (Hz):");
                        if ui.add(egui::Slider::new(&mut lfo_rate, 0.01..=5.0).step_by(0.01)).changed() {
                            self.params.master.lfo_rate.store(lfo_rate, Relaxed);
                        }
                    });
                    
                    let mut ir_freeze = self.params.master.ir_freeze.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("IR Freeze:");
                        if ui.add(egui::Slider::new(&mut ir_freeze, 0.0..=0.9999).step_by(0.01)).changed() {
                            self.params.master.ir_freeze.store(ir_freeze, Relaxed);
                        }
                    });
                    
                    let mut dry_wet = self.params.master.dry_wet.load(Relaxed);
                    ui.horizontal(|ui| {
                        ui.label("Dry/Wet (master):");
                        if ui.add(egui::Slider::new(&mut dry_wet, 0.0..=1.0).step_by(0.01)).changed() {
                            self.params.master.dry_wet.store(dry_wet, Relaxed);
                        }
                    });
                });
            });
        });
        
        ctx.request_repaint();
    }
}

pub fn run() -> Result<(), eframe::Error> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([600.0, 800.0])
            .with_title("Tri-Space Ambient Engine v3.1 PROFESSIONAL"),
        ..Default::default()
    };
    
    eframe::run_native(
        "Tri-Space Ambient Engine",
        options,
        Box::new(|cc| Ok(Box::new(TriSpaceApp::new(cc)))),
    )
}