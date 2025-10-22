# Tri-Space Ambient Engine
- Real-time granular synthesis engine for evolving ambient textures and soundscapes.

## Combines three interconnected processors
- Granular engine (100k+ grain polyphony with Poisson spawning), 24-line FDN reverb (with spectral shimmer and IR freeze), and 8-band spectral delay—into a single audio graph with complex feedback routing.
- Features dynamic sample navigation (Static/Scan/Jump modes) for exploring long-form source material, 3-band envelope follower for adaptive space response, and macro parameter shaping for playable spectral control. 
- Built in Rust using lock-free atomic parameters, SIMD-aligned processing, and efficient block-rate rendering at 48kHz.

## Core Architecture: 
- WAV sample → Grain spawner → Parallel FDN/Delay/Spectral buses → Convolution head → Tilt EQ → Output limiter.
