//! A playout buffer for the painted field.
//!
//! The solver encodes the same number of steps every frame and one state copy
//! is taken a frame, but the copies reach the main world unevenly: a copy
//! that finishes on the device just after a frame polls for it lands with
//! the next one. Measured at 120 Hz on a quiet host, the copies a frame
//! received ran `1 0 2 1` over and over, so a quarter of the frames painted
//! nothing new and the frame after skipped a copy
//! (`docs/spikes/funfern-drawn-pacing.md`, "The readback's delivery"). The
//! in-flight depth was not the limit: three, four and five copies in flight
//! all left the picture still on a quarter of the frames.
//!
//! So the picture plays the copies out one a frame from a short queue, a frame
//! or so behind their arrival, the way a jitter buffer plays out packets: a
//! frame that receives nothing paints the copy held in reserve, and one that
//! receives two keeps the second for the next frame.

use std::collections::VecDeque;

/// Copies a queue may hold before the oldest is dropped, so the lag a burst
/// of arrivals builds stays bounded.
const LIMIT: usize = 4;

/// Frames a lone copy waits for a second before it is painted anyway: with
/// one frame's wait a reserve builds up, and a stream that has stopped still
/// shows its last copy.
const LONE_WAIT: u32 = 1;

/// Consecutive frames with two or more copies left after a release before one
/// is dropped: the queue settles at one in reserve at its emptiest, and a
/// reserve that never runs down is lag that buys nothing.
const SURPLUS_FRAMES: u32 = 60;

/// One copy: the readback serial it arrived as, the step it holds and the
/// values.
#[derive(Clone, Debug, PartialEq)]
pub struct PictureCopy<T> {
    pub serial: u64,
    pub step: u64,
    pub values: T,
}

#[derive(Clone, Debug)]
pub struct PicturePlayout<T> {
    queue: VecDeque<PictureCopy<T>>,
    shown: Option<PictureCopy<T>>,
    lone_frames: u32,
    surplus_frames: u32,
}

impl<T> Default for PicturePlayout<T> {
    fn default() -> Self {
        Self {
            queue: VecDeque::new(),
            shown: None,
            lone_frames: 0,
            surplus_frames: 0,
        }
    }
}

impl<T> PicturePlayout<T> {
    /// A copy as it lands.
    pub fn push(&mut self, copy: PictureCopy<T>) {
        self.queue.push_back(copy);
        while self.queue.len() > LIMIT {
            self.queue.pop_front();
        }
    }

    /// Once a frame: moves the next copy to the one shown.
    pub fn release(&mut self) {
        match self.queue.len() {
            0 => self.lone_frames = 0,
            1 if self.lone_frames < LONE_WAIT => self.lone_frames += 1,
            _ => {
                self.shown = self.queue.pop_front();
                self.lone_frames = 0;
            }
        }
        if self.queue.len() >= 2 {
            self.surplus_frames += 1;
            if self.surplus_frames >= SURPLUS_FRAMES {
                self.queue.pop_front();
                self.surplus_frames = 0;
            }
        } else {
            self.surplus_frames = 0;
        }
    }

    /// The copy the frame paints.
    pub fn shown(&self) -> Option<&PictureCopy<T>> {
        self.shown.as_ref()
    }

    /// Forgets every copy, for a new generation.
    pub fn clear(&mut self) {
        *self = Self::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Plays `arrivals` copies a frame for `frames` frames and answers the
    /// serial each frame shows.
    fn play(arrivals: &[usize], frames: usize) -> Vec<Option<u64>> {
        let mut playout = PicturePlayout::default();
        let mut serial = 0;
        (0..frames)
            .map(|frame| {
                for _ in 0..arrivals[frame % arrivals.len()] {
                    serial += 1;
                    playout.push(PictureCopy {
                        serial,
                        step: 4 * serial,
                        values: (),
                    });
                }
                playout.release();
                playout.shown().map(|copy| copy.serial)
            })
            .collect()
    }

    /// The delivery measured at 120 Hz, a copy a frame arriving `1 0 2 1`:
    /// played out, every frame after the first few shows the next copy.
    #[test]
    fn the_measured_beat_plays_out_one_copy_a_frame() {
        let shown = play(&[1, 0, 2, 1], 400);
        let steady = &shown[8..];
        for pair in steady.windows(2) {
            assert_eq!(
                pair[1].unwrap(),
                pair[0].unwrap() + 1,
                "one copy a frame: {shown:?}"
            );
        }
        // At most two frames' worth behind the newest arrival.
        let arrived = (400 / 4) * 4;
        assert!(arrived as u64 - steady.last().unwrap().unwrap() <= 2);
    }

    /// Even arrivals pass straight through once the reserve is built, a
    /// frame behind.
    #[test]
    fn even_arrivals_pass_through_a_frame_behind() {
        let shown = play(&[1], 50);
        for (frame, serial) in shown.iter().enumerate().skip(2) {
            assert_eq!(serial.unwrap(), frame as u64);
        }
    }

    /// A stream that stops still shows its last copy, a frame late.
    #[test]
    fn a_stopped_stream_shows_its_last_copy() {
        let mut shown = play(&[1, 1, 1, 0, 0, 0, 0], 7);
        assert_eq!(shown.pop().unwrap(), Some(3));
    }

    /// A burst that leaves a standing reserve is trimmed back, so the lag it
    /// built does not last.
    #[test]
    fn a_standing_reserve_is_trimmed() {
        let mut arrivals = vec![4];
        arrivals.extend(std::iter::repeat_n(1, 199));
        let shown = play(&arrivals, 200);
        let newest = 4 + 199;
        assert!(newest - shown.last().unwrap().unwrap() <= 2, "{shown:?}");
    }

    /// The queue never holds more than its limit, whatever lands at once.
    #[test]
    fn a_flood_keeps_the_newest_copies() {
        let mut playout = PicturePlayout::default();
        for serial in 1..=10 {
            playout.push(PictureCopy {
                serial,
                step: serial,
                values: (),
            });
        }
        playout.release();
        assert_eq!(playout.shown().unwrap().serial, 7);
    }
}
