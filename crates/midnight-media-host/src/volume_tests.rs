use super::*;

fn scaled(input: &[i16], gain: u16) -> Vec<i16> {
    let mut output = Vec::with_capacity(input.len());
    scale_pcm(input, gain, &mut output);
    output
}

const EXTREMES: [i16; 6] = [i16::MIN, i16::MIN + 1, -1, 0, 1, i16::MAX];

#[test]
fn full_volume_is_bit_exact_and_zero_is_silence() {
    assert_eq!(gain(100), UNITY_GAIN);
    assert_eq!(scaled(&EXTREMES, gain(100)), EXTREMES);
    assert_eq!(gain(0), 0);
    assert_eq!(scaled(&EXTREMES, gain(0)), [0; 6]);
}

#[test]
fn half_volume_follows_the_cubic_curve_and_rounds_to_nearest() {
    // (50 / 100)³ = 1/8 of unity: −18 dB.
    assert_eq!(gain(50), UNITY_GAIN / 8);
    assert_eq!(
        scaled(&[8000, -8000, 4, -4, 3, i16::MAX, i16::MIN], gain(50)),
        [1000, -1000, 1, 0, 0, 4096, -4096]
    );
    // The curve rises monotonically and a level of 1 % is below 16-bit resolution.
    assert!((1..=100).all(|percent| gain(percent - 1) <= gain(percent)));
    assert_eq!(gain(1), 0);
    assert!(gain(5) > 0);
}

#[test]
fn extreme_samples_saturate_instead_of_wrapping() {
    for percent in [99, 75, 10] {
        let factor = f64::from(gain(percent)) / f64::from(UNITY_GAIN);
        for (&input, output) in EXTREMES.iter().zip(scaled(&EXTREMES, gain(percent))) {
            // Same sign, never louder, and within rounding of the exact product.
            assert!(output.signum() == input.signum() || output == 0);
            assert!(output.unsigned_abs() <= input.unsigned_abs());
            assert!((f64::from(output) - f64::from(input) * factor).abs() <= 0.5);
        }
    }
    // A gain above unity is never produced, but it must not wrap either.
    assert_eq!(
        scaled(&EXTREMES, u16::MAX),
        [i16::MIN, i16::MIN, -2, 0, 2, i16::MAX]
    );
}

#[test]
fn mute_keeps_the_level_and_silences_it() {
    let volume = AudioVolume::default();
    assert_eq!(volume.get(), (100, false));
    assert_eq!(volume.gain(), UNITY_GAIN);
    volume.set(40, true).unwrap();
    assert_eq!(volume.get(), (40, true));
    assert_eq!(volume.gain(), 0);
    volume.set(40, false).unwrap();
    assert_eq!(volume.gain(), gain(40));
}

#[test]
fn a_level_above_full_is_refused_and_the_current_one_stays() {
    let volume = AudioVolume::default();
    volume.set(70, false).unwrap();
    for percent in [101, 200, u8::MAX] {
        let error = volume.set(percent, true).unwrap_err();
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }
    assert_eq!(volume.get(), (70, false));
    volume.set(0, false).unwrap();
    assert_eq!(volume.get(), (0, false));
}
