//! Integration Tests

#[cfg(test)]
mod tests {
    use crate::dsp::*;
    use crate::types::*;

    #[test]
    fn test_constants() {
        assert_eq!(SR, 48_000);
        assert_eq!(BLOCK, 128);
        assert!(MAX_GRAINS > 0);
    }

    #[test]
    fn test_dc_blocker() {
        let mut dcb = DcBlockerExact::new(10.0, SR as f32);

        // DC input should be blocked after settling
        for _ in 0..1000 {
            dcb.process(1.0);
        }
        let output = dcb.process(1.0);
        assert!(output.abs() < 0.01, "DC should be blocked");
    }

    #[test]
    fn test_slew_limiter_linear() {
        let mut slew = SlewLimiter::new(0.0, 1.0, SlewMode::Linear);
        slew.set_target(1.0);

        let dt = 1.0 / SR as f32;
        for _ in 0..(SR / 2) {
            slew.process(dt);
        }

        let val = slew.current();
        assert!(
            val > 0.4 && val < 0.6,
            "Should be near halfway after 0.5 seconds"
        );
    }

    #[test]
    fn test_window_normalization() {
        let window = create_hann_squared_unit_power(1024);

        let power: f32 = window.iter().map(|&w| w * w).sum();
        let expected_power = 1024.0;

        assert!(
            (power - expected_power).abs() / expected_power < 0.01,
            "Window power should equal N"
        );
    }

    #[test]
    fn test_soft_clip() {
        assert_eq!(soft_clip(0.5, 0.9), 0.5);
        let clipped = soft_clip(2.0, 0.9);
        assert!(clipped > 0.9 && clipped < 1.0);
    }

    #[test]
    fn test_lerp() {
        assert_eq!(lerp(0.0, 10.0, 0.5), 5.0);
        assert_eq!(lerp(0.0, 10.0, 0.0), 0.0);
        assert_eq!(lerp(0.0, 10.0, 1.0), 10.0);
    }

    #[test]
    fn test_params_default() {
        let params = GlobalParams::default();
        assert_eq!(params.reverb.bloom.load(Ordering::Relaxed), 0.0);
        assert_eq!(params.grains.density.load(Ordering::Relaxed), 0.0);
    }
}