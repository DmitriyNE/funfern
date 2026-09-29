//! A radix-2 FFT and the spectra readouts show: which frequencies a probe's
//! record or a pulse holds, and how much of each.

/// Why a spectrum could not be taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpectrumError {
    /// The FFT takes a power-of-two length.
    NotAPowerOfTwo,
    LengthMismatch,
    TooFewSamples,
    InvalidInterval,
    NonFinite,
}

/// The forward transform `X_k = Σ x_n e^{-2πikn/N}` in place, of a length
/// that is a power of two, by decimation in time.
pub fn fft(re: &mut [f64], im: &mut [f64]) -> Result<(), SpectrumError> {
    let n = re.len();
    if im.len() != n {
        return Err(SpectrumError::LengthMismatch);
    }
    if !n.is_power_of_two() {
        return Err(SpectrumError::NotAPowerOfTwo);
    }
    if n == 1 {
        return Ok(());
    }
    let bits = n.trailing_zeros();
    for i in 0..n {
        let j = i.reverse_bits() >> (usize::BITS - bits);
        if j > i {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    // One table of e^{-2πik/N}; a stage of `size` reads every (N/size)th.
    let twiddles = (0..n / 2)
        .map(|k| (-std::f64::consts::TAU * k as f64 / n as f64).sin_cos())
        .collect::<Vec<_>>();
    let mut size = 2;
    while size <= n {
        let half = size / 2;
        let stride = n / size;
        for start in (0..n).step_by(size) {
            for k in 0..half {
                let (sin, cos) = twiddles[k * stride];
                let (a, b) = (start + k, start + k + half);
                let product_re = re[b] * cos - im[b] * sin;
                let product_im = re[b] * sin + im[b] * cos;
                re[b] = re[a] - product_re;
                im[b] = im[a] - product_im;
                re[a] += product_re;
                im[a] += product_im;
            }
        }
        size *= 2;
    }
    Ok(())
}

/// A one-sided spectrum from 0 Hz, one value every `frequency_step_hz`.
#[derive(Clone, Debug, PartialEq)]
pub struct Spectrum {
    pub frequency_step_hz: f64,
    pub magnitudes: Vec<f64>,
}

impl Spectrum {
    /// The frequency of the `index`th value.
    pub fn frequency_hz(&self, index: usize) -> f64 {
        index as f64 * self.frequency_step_hz
    }
}

/// How far a record is zero padded: to this many times its length, rounded
/// up to a power of two. That draws the spectrum between the bins a record of
/// that length resolves, and bounds the Hann window's scalloping to about 1%.
const PADDING: usize = 4;

fn validate(samples: &[f64], interval: f64) -> Result<(), SpectrumError> {
    if samples.len() < 2 {
        return Err(SpectrumError::TooFewSamples);
    }
    if !interval.is_finite() || interval <= 0.0 {
        return Err(SpectrumError::InvalidInterval);
    }
    if samples.iter().any(|value| !value.is_finite()) {
        return Err(SpectrumError::NonFinite);
    }
    Ok(())
}

/// The transform of `samples`, times `scale` for each input sample, zero
/// padded, as one-sided magnitudes.
fn one_sided(
    samples: impl ExactSizeIterator<Item = f64>,
    interval: f64,
    doubled: bool,
    scale: f64,
) -> Result<Spectrum, SpectrumError> {
    let size = (PADDING * samples.len()).next_power_of_two();
    let mut re = vec![0.0; size];
    let mut im = vec![0.0; size];
    for (slot, value) in re.iter_mut().zip(samples) {
        *slot = value;
    }
    fft(&mut re, &mut im)?;
    let magnitudes = (0..=size / 2)
        .map(|k| {
            let both = if doubled && k != 0 && k != size / 2 {
                2.0
            } else {
                1.0
            };
            both * scale * re[k].hypot(im[k])
        })
        .collect();
    Ok(Spectrum {
        frequency_step_hz: 1.0 / (size as f64 * interval),
        magnitudes,
    })
}

/// The amplitude spectrum of a record sampled every `interval` seconds, for
/// what a signal holds while it runs. A Hann window tapers the record so that
/// its ends, which rarely meet, do not spread every line across the whole
/// plot, and the window's own mean is removed, so no offset shows. It is
/// scaled so that a steady tone of amplitude `A` reads `A` at its frequency; a
/// transient reads its amplitude averaged over the record.
pub fn amplitude_spectrum(samples: &[f64], interval: f64) -> Result<Spectrum, SpectrumError> {
    validate(samples, interval)?;
    let count = samples.len();
    // Sampled between its zeros, so the window's sum is exactly half the
    // record's length whatever that length.
    let window = |index: usize| {
        let angle = std::f64::consts::PI * (index as f64 + 0.5) / count as f64;
        angle.sin().powi(2)
    };
    let gain = 0.5 * count as f64;
    // The mean the window sees rather than the record's: taking that away
    // leaves the windowed record summing to zero, where the plain mean would
    // leave the window's leakage of every line at 0 Hz.
    let mean = samples
        .iter()
        .enumerate()
        .map(|(index, value)| value * window(index))
        .sum::<f64>()
        / gain;
    one_sided(
        samples
            .iter()
            .enumerate()
            .map(|(index, value)| (value - mean) * window(index)),
        interval,
        true,
        1.0 / gain,
    )
}

/// The magnitude of the Fourier transform of a transient sampled every
/// `interval` seconds, whole within the record so that nothing of it is cut
/// at either end: `interval |X_k|`, in the samples' units times seconds. Unlike
/// `amplitude_spectrum` it is neither windowed nor detrended, since a pulse
/// that starts and ends at zero needs neither.
pub fn transient_spectrum(samples: &[f64], interval: f64) -> Result<Spectrum, SpectrumError> {
    validate(samples, interval)?;
    one_sided(samples.iter().copied(), interval, false, interval)
}

/// A record of `(time, value)` pairs, times increasing, sampled every
/// `interval` seconds from its first time and linearly between the pairs.
/// Probes record at a stride of whole steps, and a handoff that changes the
/// step changes their spacing, which an FFT cannot take.
pub fn resample_evenly(
    times: &[f64],
    values: &[f64],
    interval: f64,
) -> Result<Vec<f64>, SpectrumError> {
    if times.len() != values.len() {
        return Err(SpectrumError::LengthMismatch);
    }
    validate(values, interval)?;
    if times.iter().any(|time| !time.is_finite()) || times.windows(2).any(|pair| pair[1] <= pair[0])
    {
        return Err(SpectrumError::NonFinite);
    }
    let first = times[0];
    let count = ((times[times.len() - 1] - first) / interval).floor() as usize + 1;
    let mut resampled = Vec::with_capacity(count);
    let mut segment = 0;
    for index in 0..count {
        let time = first + index as f64 * interval;
        while segment + 2 < times.len() && times[segment + 1] < time {
            segment += 1;
        }
        let (start, end) = (times[segment], times[segment + 1]);
        let fraction = ((time - start) / (end - start)).clamp(0.0, 1.0);
        resampled.push(values[segment] + fraction * (values[segment + 1] - values[segment]));
    }
    Ok(resampled)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::TAU;

    fn direct_dft(values: &[f64]) -> Vec<(f64, f64)> {
        let n = values.len();
        (0..n)
            .map(|k| {
                values
                    .iter()
                    .enumerate()
                    .fold((0.0, 0.0), |(re, im), (index, value)| {
                        let angle = -TAU * (k * index) as f64 / n as f64;
                        (re + value * angle.cos(), im + value * angle.sin())
                    })
            })
            .collect()
    }

    #[test]
    fn the_fft_is_the_direct_transform_and_keeps_parseval() {
        for n in [1, 2, 8, 64, 256] {
            let values = (0..n)
                .map(|index| (0.37 * index as f64).sin() + 0.2 * (index as f64 * 1.9).cos() - 0.1)
                .collect::<Vec<_>>();
            let (mut re, mut im) = (values.clone(), vec![0.0; n]);
            fft(&mut re, &mut im).unwrap();
            for (k, (expected_re, expected_im)) in direct_dft(&values).into_iter().enumerate() {
                assert!(
                    (re[k] - expected_re).abs() < 1.0e-11 && (im[k] - expected_im).abs() < 1.0e-11,
                    "bin {k} of {n}"
                );
            }
            let time = values.iter().map(|value| value * value).sum::<f64>();
            let frequency = re
                .iter()
                .zip(&im)
                .map(|(re, im)| re * re + im * im)
                .sum::<f64>()
                / n as f64;
            assert!((time - frequency).abs() < 1.0e-10 * time.max(1.0));
        }
        assert_eq!(
            fft(&mut [0.0; 6], &mut [0.0; 6]),
            Err(SpectrumError::NotAPowerOfTwo)
        );
        assert_eq!(
            fft(&mut [0.0; 4], &mut [0.0; 2]),
            Err(SpectrumError::LengthMismatch)
        );
    }

    /// A steady tone reads its amplitude at its frequency, on a bin or
    /// between bins, and a record whose length is no power of two is taken as
    /// readily. The mean does not show.
    #[test]
    fn a_tone_reads_its_amplitude_at_its_frequency() {
        let interval = 1.0 / 60.0;
        for (count, frequency, amplitude) in [(600, 3.0, 0.7), (1000, 2.37, 1.3), (777, 11.1, 0.05)]
        {
            let samples = (0..count)
                .map(|index| {
                    0.4 + amplitude * (TAU * frequency * index as f64 * interval + 0.3).sin()
                })
                .collect::<Vec<_>>();
            let spectrum = amplitude_spectrum(&samples, interval).unwrap();
            let (peak, value) = spectrum
                .magnitudes
                .iter()
                .enumerate()
                .max_by(|a, b| a.1.total_cmp(b.1))
                .unwrap();
            let found = spectrum.frequency_hz(peak);
            assert!(
                (found - frequency).abs() <= spectrum.frequency_step_hz,
                "{frequency} Hz read at {found}"
            );
            assert!(
                (value / amplitude - 1.0).abs() < 0.02,
                "{amplitude} read {value} at {frequency} Hz"
            );
            assert!(spectrum.magnitudes[0] < 1.0e-12);
        }
    }

    /// A Gaussian's transform is a Gaussian: σ√(2π) e^(-2π²σ²f²).
    #[test]
    fn a_transient_reads_its_fourier_transform() {
        let (width, interval) = (0.05, 1.0e-3);
        let samples = (0..1000)
            .map(|index| {
                let time = index as f64 * interval - 0.5;
                (-0.5 * (time / width).powi(2)).exp()
            })
            .collect::<Vec<_>>();
        let spectrum = transient_spectrum(&samples, interval).unwrap();
        for index in [0, 10, 40, 80] {
            let frequency = spectrum.frequency_hz(index);
            let expected = width * TAU.sqrt() * (-0.5 * (TAU * width * frequency).powi(2)).exp();
            assert!(
                (spectrum.magnitudes[index] - expected).abs() < 1.0e-9,
                "{} against {expected} at {frequency} Hz",
                spectrum.magnitudes[index]
            );
        }
    }

    /// Records a handoff left at two spacings come out at one, on the line
    /// through their neighbours.
    #[test]
    fn an_uneven_record_resamples_onto_one_spacing() {
        let times = [0.0, 0.1, 0.2, 0.25, 0.3, 0.35, 0.4];
        let values = times.map(|time| 2.0 * time - 0.5);
        let resampled = resample_evenly(&times, &values, 0.05).unwrap();
        assert_eq!(resampled.len(), 9);
        for (index, value) in resampled.iter().enumerate() {
            assert!((value - (2.0 * 0.05 * index as f64 - 0.5)).abs() < 1.0e-12);
        }
        assert_eq!(
            resample_evenly(&[0.0, 0.0], &[1.0, 2.0], 0.1),
            Err(SpectrumError::NonFinite)
        );
        assert_eq!(
            resample_evenly(&[0.0, 1.0], &[1.0], 0.1),
            Err(SpectrumError::LengthMismatch)
        );
        assert_eq!(
            amplitude_spectrum(&[1.0], 0.1),
            Err(SpectrumError::TooFewSamples)
        );
        assert_eq!(
            amplitude_spectrum(&[1.0, f64::NAN], 0.1),
            Err(SpectrumError::NonFinite)
        );
        assert_eq!(
            amplitude_spectrum(&[1.0, 2.0], 0.0),
            Err(SpectrumError::InvalidInterval)
        );
    }
}
