//! A radix-2 FFT and the spectra readouts show: which frequencies a probe's
//! record or a pulse holds, and how much of each, and how much of each
//! passes from one record to another.

/// Why a spectrum could not be taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SpectrumError {
    /// The FFT takes a power-of-two length.
    NotAPowerOfTwo,
    LengthMismatch,
    TooFewSamples,
    InvalidInterval,
    NonFinite,
    /// A resampled span reaches past either end of its record.
    OutsideRecord,
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

/// The transform of `samples`, zero padded by `PADDING`.
fn padded_transform(
    samples: impl ExactSizeIterator<Item = f64>,
) -> Result<(Vec<f64>, Vec<f64>), SpectrumError> {
    let size = (PADDING * samples.len()).next_power_of_two();
    let mut re = vec![0.0; size];
    let mut im = vec![0.0; size];
    for (slot, value) in re.iter_mut().zip(samples) {
        *slot = value;
    }
    fft(&mut re, &mut im)?;
    Ok((re, im))
}

/// The transform of `samples`, times `scale` for each input sample, zero
/// padded, as one-sided magnitudes.
fn one_sided(
    samples: impl ExactSizeIterator<Item = f64>,
    interval: f64,
    doubled: bool,
    scale: f64,
) -> Result<Spectrum, SpectrumError> {
    let (re, im) = padded_transform(samples)?;
    let size = re.len();
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
    let (tapered, gain) = hann_tapered(samples);
    one_sided(tapered.into_iter(), interval, true, 1.0 / gain)
}

/// `samples` as `amplitude_spectrum` transforms them: the Hann window's own
/// mean taken out and the window applied; and the window's sum.
fn hann_tapered(samples: &[f64]) -> (Vec<f64>, f64) {
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
    let tapered = samples
        .iter()
        .enumerate()
        .map(|(index, value)| (value - mean) * window(index))
        .collect();
    (tapered, gain)
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

/// How a response follows a reference, frequency by frequency, from 0 Hz one
/// value every `frequency_step_hz`: the response's transform over the
/// reference's, whose magnitude is the gain and whose argument the phase.
#[derive(Clone, Debug, PartialEq)]
pub struct TransferSpectrum {
    pub frequency_step_hz: f64,
    /// Each ratio's real and imaginary parts, or `None` where the reference
    /// holds too little of that frequency to divide by.
    pub ratios: Vec<Option<[f64; 2]>>,
}

impl TransferSpectrum {
    /// The frequency of the `index`th value.
    pub fn frequency_hz(&self, index: usize) -> f64 {
        index as f64 * self.frequency_step_hz
    }

    /// The gain at the `index`th frequency, where there is one.
    pub fn magnitude(&self, index: usize) -> Option<f64> {
        self.ratios[index].map(|[re, im]| re.hypot(im))
    }
}

/// Below this fraction of its strongest, 40 dB down, a reference holds too
/// little of a frequency to divide by: the ratio there would be the
/// response's small errors over nearly nothing.
pub const TRANSFER_FLOOR: f64 = 1.0e-2;

/// The transfer from `reference` to `response`, two records of one length
/// sampled together every `interval` seconds. Neither is windowed: a window
/// weights each moment of a record differently, so a response arriving later
/// than its reference would come out scaled by the window's shape. The ratio
/// is exact when both records hold the whole of what passes; a record that
/// cuts a response short reads the cut as ripple.
pub fn transfer_spectrum(
    reference: &[f64],
    response: &[f64],
    interval: f64,
) -> Result<TransferSpectrum, SpectrumError> {
    validate(reference, interval)?;
    validate(response, interval)?;
    if reference.len() != response.len() {
        return Err(SpectrumError::LengthMismatch);
    }
    let (reference_re, reference_im) = padded_transform(reference.iter().copied())?;
    let (response_re, response_im) = padded_transform(response.iter().copied())?;
    let size = reference_re.len();
    let power = |k: usize| reference_re[k] * reference_re[k] + reference_im[k] * reference_im[k];
    let peak = (0..=size / 2).map(power).fold(0.0, f64::max);
    let floor = TRANSFER_FLOOR * TRANSFER_FLOOR * peak;
    let ratios = (0..=size / 2)
        .map(|k| {
            let power = power(k);
            (power > floor).then(|| {
                let (x_re, x_im) = (reference_re[k], reference_im[k]);
                let (y_re, y_im) = (response_re[k], response_im[k]);
                [
                    (y_re * x_re + y_im * x_im) / power,
                    (y_im * x_re - y_re * x_im) / power,
                ]
            })
        })
        .collect();
    Ok(TransferSpectrum {
        frequency_step_hz: 1.0 / (size as f64 * interval),
        ratios,
    })
}

/// A transfer averaged over many windows, as a two-channel analyser takes
/// one: the sum of `Y X̄` over the sum of `|X|²`, every window Hann-tapered
/// as `amplitude_spectrum` tapers it. Two records a delay apart hold
/// different parts of a signal in any one window, and leak differently, so a
/// single window's ratio wobbles as the window slides; summed over windows,
/// what differs averages out. A window short beside the delay still reads
/// low, by the window's overlap with itself shifted by the delay: 6% where
/// the delay is a tenth of the window, 32% where it is a quarter.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct TransferAverage {
    length: usize,
    interval: f64,
    cross: Vec<[f64; 2]>,
    power: Vec<f64>,
    windows: usize,
}

impl TransferAverage {
    /// Adds a window of `reference` and `response`, two records of one
    /// length sampled together every `interval` seconds. A window of another
    /// length or spacing than those summed is refused: its frequencies fall
    /// elsewhere.
    pub fn add(
        &mut self,
        reference: &[f64],
        response: &[f64],
        interval: f64,
    ) -> Result<(), SpectrumError> {
        validate(reference, interval)?;
        validate(response, interval)?;
        if reference.len() != response.len()
            || (self.windows > 0 && (reference.len() != self.length || interval != self.interval))
        {
            return Err(SpectrumError::LengthMismatch);
        }
        let (reference_re, reference_im) = padded_transform(hann_tapered(reference).0.into_iter())?;
        let (response_re, response_im) = padded_transform(hann_tapered(response).0.into_iter())?;
        let bins = reference_re.len() / 2 + 1;
        if self.windows == 0 {
            self.length = reference.len();
            self.interval = interval;
            self.cross = vec![[0.0; 2]; bins];
            self.power = vec![0.0; bins];
        }
        for k in 0..bins {
            let (x_re, x_im) = (reference_re[k], reference_im[k]);
            let (y_re, y_im) = (response_re[k], response_im[k]);
            self.cross[k][0] += y_re * x_re + y_im * x_im;
            self.cross[k][1] += y_im * x_re - y_re * x_im;
            self.power[k] += x_re * x_re + x_im * x_im;
        }
        self.windows += 1;
        Ok(())
    }

    /// How many windows the average holds.
    pub fn windows(&self) -> usize {
        self.windows
    }

    /// The averaged transfer, or `None` before any window. A frequency the
    /// reference's summed power holds under `TRANSFER_FLOOR` of its
    /// strongest, in amplitude, is left out.
    pub fn transfer(&self) -> Option<TransferSpectrum> {
        if self.windows == 0 {
            return None;
        }
        let size = 2 * (self.power.len() - 1);
        let peak = self.power.iter().copied().fold(0.0, f64::max);
        let floor = TRANSFER_FLOOR * TRANSFER_FLOOR * peak;
        let ratios = self
            .power
            .iter()
            .zip(&self.cross)
            .map(|(power, [re, im])| (*power > floor).then(|| [re / power, im / power]))
            .collect();
        Some(TransferSpectrum {
            frequency_step_hz: 1.0 / (size as f64 * self.interval),
            ratios,
        })
    }
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
    let (Some(first), Some(last)) = (times.first(), times.last()) else {
        return Err(SpectrumError::TooFewSamples);
    };
    let count = ((last - first) / interval).floor() as usize + 1;
    resample_from(times, values, *first, interval, count)
}

/// `count` samples of the record every `interval` seconds from `from`, all
/// within it, so that records resampled from one time at one spacing come
/// out on one grid, sample for sample.
pub fn resample_from(
    times: &[f64],
    values: &[f64],
    from: f64,
    interval: f64,
    count: usize,
) -> Result<Vec<f64>, SpectrumError> {
    if times.len() != values.len() {
        return Err(SpectrumError::LengthMismatch);
    }
    validate(values, interval)?;
    if times.iter().any(|time| !time.is_finite()) || times.windows(2).any(|pair| pair[1] <= pair[0])
    {
        return Err(SpectrumError::NonFinite);
    }
    if !from.is_finite() {
        return Err(SpectrumError::NonFinite);
    }
    // A span past the record by under a millionth of a sample is round-off
    // in where it was asked to start or end, and taken as meeting it.
    let slack = 1.0e-6 * interval;
    let to = from + (count.max(1) - 1) as f64 * interval;
    if from < times[0] - slack || to > times[times.len() - 1] + slack {
        return Err(SpectrumError::OutsideRecord);
    }
    let first = from;
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

    /// A Gaussian pulse on a carrier, centred `centre` seconds into a record
    /// of `count` samples.
    fn tone_burst(count: usize, interval: f64, centre: f64) -> Vec<f64> {
        (0..count)
            .map(|index| {
                let time = index as f64 * interval - centre;
                (-0.5 * (time / 0.04).powi(2)).exp() * (TAU * 6.0 * time).sin()
            })
            .collect()
    }

    /// A copy scaled and delayed by whole samples transfers its scale at
    /// every frequency the reference holds, and its delay as a phase that
    /// falls with frequency, however late it arrives. A Hann window would
    /// have weighted a late copy up against an early reference.
    #[test]
    fn a_delayed_copy_transfers_its_scale_whatever_its_delay() {
        let interval = 1.0e-3;
        let reference = tone_burst(2000, interval, 0.2);
        for delay in [0, 150, 900, 1500] {
            let mut response = vec![0.0; 2000];
            for (index, value) in reference.iter().enumerate() {
                if index + delay < response.len() {
                    response[index + delay] = 0.4 * value;
                }
            }
            let transfer = transfer_spectrum(&reference, &response, interval).unwrap();
            let mut held = 0;
            for (index, ratio) in transfer.ratios.iter().enumerate() {
                let Some([re, im]) = *ratio else { continue };
                held += 1;
                let phase = -TAU * transfer.frequency_hz(index) * delay as f64 * interval;
                assert!(
                    (re - 0.4 * phase.cos()).abs() < 1.0e-9
                        && (im - 0.4 * phase.sin()).abs() < 1.0e-9,
                    "delay {delay}, {} Hz: {re} {im}",
                    transfer.frequency_hz(index)
                );
            }
            assert!(held > 100, "the burst's band holds {held} frequencies");
        }
    }

    /// An echo `a` as strong and `τ` late reads the comb `|1 + a e^(-iωτ)|`.
    #[test]
    fn an_echo_reads_its_comb() {
        let interval = 1.0e-3;
        let reference = tone_burst(2000, interval, 0.2);
        let (echo, lag) = (0.5, 70);
        let response = (0..2000)
            .map(|index| {
                reference[index]
                    + index
                        .checked_sub(lag)
                        .map_or(0.0, |late| echo * reference[late])
            })
            .collect::<Vec<_>>();
        let transfer = transfer_spectrum(&reference, &response, interval).unwrap();
        for index in 0..transfer.ratios.len() {
            let Some(gain) = transfer.magnitude(index) else {
                continue;
            };
            let phase = TAU * transfer.frequency_hz(index) * lag as f64 * interval;
            let expected = (1.0 + echo * phase.cos()).hypot(echo * phase.sin());
            assert!(
                (gain - expected).abs() < 1.0e-9,
                "{gain} against {expected}"
            );
        }
    }

    /// Where the reference holds under 1% of its strongest there is no
    /// ratio, and a silent reference has none anywhere.
    #[test]
    fn a_transfer_leaves_out_what_its_reference_does_not_hold() {
        let interval = 1.0e-3;
        let reference = tone_burst(2000, interval, 0.5);
        let response = tone_burst(2000, interval, 0.7);
        let transfer = transfer_spectrum(&reference, &response, interval).unwrap();
        let at = |frequency: f64| (frequency / transfer.frequency_step_hz).round() as usize;
        assert!(transfer.ratios[at(6.0)].is_some());
        // The burst's spectrum, a Gaussian 1/(2π·0.04) wide about 6 Hz, is
        // down to 1% about 12 Hz either side.
        assert!(transfer.ratios[at(0.0)].is_none());
        assert!(transfer.ratios[at(25.0)].is_none());
        let silent = transfer_spectrum(&[0.0; 64], &response[..64], interval).unwrap();
        assert!(silent.ratios.iter().all(Option::is_none));
        assert_eq!(
            transfer_spectrum(&reference, &response[..100], interval),
            Err(SpectrumError::LengthMismatch)
        );
    }

    /// Two records whose rings started at different times, resampled over
    /// the span they share, come out on one grid.
    #[test]
    fn two_records_resample_onto_one_grid_over_the_span_they_share() {
        let early = [0.0, 0.1, 0.2, 0.3, 0.4, 0.5];
        let late = [0.25, 0.35, 0.45, 0.55, 0.65];
        let line = |time: f64| 3.0 * time + 1.0;
        let from = late[0];
        let first = resample_from(&early, &early.map(line), from, 0.05, 6).unwrap();
        let second = resample_from(&late, &late.map(line), from, 0.05, 6).unwrap();
        assert_eq!(first.len(), 6);
        assert_eq!(first.len(), second.len());
        for (index, (a, b)) in first.iter().zip(&second).enumerate() {
            let expected = line(from + 0.05 * index as f64);
            assert!((a - expected).abs() < 1.0e-12 && (b - expected).abs() < 1.0e-12);
        }
        assert_eq!(
            resample_from(&late, &late.map(line), 0.2, 0.05, 6),
            Err(SpectrumError::OutsideRecord)
        );
        assert_eq!(
            resample_from(&early, &early.map(line), from, 0.05, 7),
            Err(SpectrumError::OutsideRecord)
        );
        // Ending a hair past the record, as a window ending on its last
        // sample can, is round-off.
        assert!(resample_from(&early, &early.map(line), 0.25 + 1.0e-15, 0.05, 6).is_ok());
    }

    /// Hann bursts on a 3 Hz carrier, a second long and back to back.
    fn bursts(time: f64) -> f64 {
        let phase = time.rem_euclid(1.0);
        (std::f64::consts::PI * phase).sin().powi(2) * (TAU * 3.0 * time).sin()
    }

    /// `count` samples every `interval` from `from` of `signal`.
    fn sampled(signal: impl Fn(f64) -> f64, from: f64, interval: f64, count: usize) -> Vec<f64> {
        (0..count)
            .map(|index| signal(from + index as f64 * interval))
            .collect()
    }

    /// The averaged gain at the frequency nearest `frequency`.
    fn gain_at(transfer: &TransferSpectrum, frequency: f64) -> f64 {
        let index = (frequency / transfer.frequency_step_hz).round() as usize;
        transfer.magnitude(index).unwrap()
    }

    /// Through a window a burst long, a train of bursts and a copy of it
    /// 0.3 s late read a different gain in each window, which the window's
    /// position decides. Averaged over windows a quarter second apart it
    /// settles on a gain that holds from one average to the next: the copy's
    /// scale times the Hann window's overlap with itself shifted by the delay,
    /// `R(u) = [(1 − u)(2 + cos 2πu) + 3 sin(2πu)/2π] / 3` at `u` the delay
    /// over the window, 0.68 here. A window ten bursts long, which the delay
    /// barely touches, reads the copy's own scale.
    #[test]
    fn an_average_of_windows_settles_where_one_window_wobbles() {
        let interval = 1.0 / 120.0;
        let count = 150;
        let response = |time: f64| 0.4 * bursts(time - 0.3);
        let window = |from: f64| {
            (
                sampled(bursts, from, interval, count),
                sampled(response, from, interval, count),
            )
        };
        let mut single = Vec::new();
        let mut averages = [TransferAverage::default(), TransferAverage::default()];
        for step in 0..80 {
            let from = 0.25 * f64::from(step);
            let (reference, response) = window(from);
            let mut one = TransferAverage::default();
            one.add(&reference, &response, interval).unwrap();
            single.push(gain_at(&one.transfer().unwrap(), 2.0));
            averages[usize::from(step >= 40)]
                .add(&reference, &response, interval)
                .unwrap();
        }
        let spread = single.iter().copied().fold(0.0, f64::max)
            - single.iter().copied().fold(f64::MAX, f64::min);
        assert!(spread > 0.1, "single windows spread {spread:.3}");
        let [first, second] = averages.map(|average| gain_at(&average.transfer().unwrap(), 2.0));
        assert!((first - second).abs() < 1.0e-3, "{first} then {second}");
        let u = 0.3 / (count as f64 * interval);
        let overlap = ((1.0 - u) * (2.0 + (TAU * u).cos()) + 3.0 * (TAU * u).sin() / TAU) / 3.0;
        // 0.2738 against 0.2726 on the first run.
        assert!(
            (first - 0.4 * overlap).abs() < 3.0e-3,
            "{first} against {}",
            0.4 * overlap
        );
        let mut long = TransferAverage::default();
        let (reference, response) = (
            sampled(bursts, 0.0, interval, 1200),
            sampled(response, 0.0, interval, 1200),
        );
        long.add(&reference, &response, interval).unwrap();
        for line in [2.0, 3.0, 4.0] {
            let gain = gain_at(&long.transfer().unwrap(), line);
            assert!((gain - 0.4).abs() < 1.0e-3, "{line} Hz reads {gain}");
        }
    }

    /// A tone reads its gain from one window, a window of another length or
    /// spacing is refused, and an empty average has no transfer.
    #[test]
    fn a_tone_reads_its_gain_and_an_average_keeps_its_grid() {
        let interval = 1.0e-2;
        let tone = |time: f64| (TAU * 3.0 * time).sin();
        let late = |time: f64| 0.7 * tone(time - 0.05);
        let mut average = TransferAverage::default();
        assert!(average.transfer().is_none());
        average
            .add(
                &sampled(tone, 0.0, interval, 400),
                &sampled(late, 0.0, interval, 400),
                interval,
            )
            .unwrap();
        assert_eq!(average.windows(), 1);
        let transfer = average.transfer().unwrap();
        // 6.4e-6 off, the window's leakage of the tone's negative frequency.
        assert!((gain_at(&transfer, 3.0) - 0.7).abs() < 1.0e-4);
        assert!(transfer.ratios.iter().any(Option::is_none));
        assert_eq!(
            average.add(&[0.0; 300], &[0.0; 300], interval),
            Err(SpectrumError::LengthMismatch)
        );
        assert_eq!(
            average.add(&[0.0; 400], &[0.0; 400], 2.0e-2),
            Err(SpectrumError::LengthMismatch)
        );
        assert_eq!(average.windows(), 1);
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
