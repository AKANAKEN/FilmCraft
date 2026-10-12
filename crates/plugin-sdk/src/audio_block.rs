//! Safe planar audio block abstraction for FilmCraft DSP plugin authors.
//!
//! Provides bound-checked, zero-panic sample manipulation over planar 32-bit float channels.

/// Safe wrapper around planar audio buffers (`&mut [&mut [f32]]`).
pub struct AudioBlock<'a> {
    channels: Vec<&'a mut [f32]>,
    sample_rate: u32,
    num_samples: usize,
}

impl<'a> AudioBlock<'a> {
    /// Creates a new AudioBlock from planar channel slices.
    ///
    /// Validates that all channels share the exact same sample count.
    /// Returns `None` if channels are empty or have mismatched slice lengths.
    pub fn new(channels: Vec<&'a mut [f32]>, sample_rate: u32) -> Option<Self> {
        if channels.is_empty() || sample_rate == 0 {
            return None;
        }

        let num_samples = channels.first()?.len();
        for ch in &channels {
            if ch.len() != num_samples {
                return None;
            }
        }

        Some(Self {
            channels,
            sample_rate,
            num_samples,
        })
    }

    #[inline(always)]
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    #[inline(always)]
    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }

    #[inline(always)]
    pub fn sample_count(&self) -> usize {
        self.num_samples
    }

    /// Retrieve an immutable slice for channel `ch_idx`.
    pub fn channel(&self, ch_idx: usize) -> Option<&[f32]> {
        self.channels.get(ch_idx).map(|slice| &**slice)
    }

    /// Retrieve a mutable slice for channel `ch_idx`.
    pub fn channel_mut(&mut self, ch_idx: usize) -> Option<&mut [f32]> {
        self.channels.get_mut(ch_idx).map(|slice| &mut **slice)
    }

    /// Apply an in-place sample transformation across all channels with bound checking.
    pub fn apply_gain(&mut self, linear_gain: f32) {
        for ch in &mut self.channels {
            for sample in ch.iter_mut() {
                *sample *= linear_gain;
            }
        }
    }

    /// Iterates across sample frames (sample index `s`, slice across all channels at index `s`).
    pub fn for_each_frame_mut<F>(&mut self, mut f: F)
    where
        F: FnMut(usize, &mut [&mut f32]),
    {
        let num_channels = self.channels.len();
        let num_samples = self.num_samples;

        // Process sample by sample
        for s in 0..num_samples {
            let mut frame_refs: Vec<&mut f32> = Vec::with_capacity(num_channels);
            for ch in &mut self.channels {
                if let Some(sample_ref) = ch.get_mut(s) {
                    // Safe reference cast within the bounded loop
                    frame_refs.push(sample_ref);
                }
            }
            if frame_refs.len() == num_channels {
                f(s, &mut frame_refs);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audio_block_creation_and_gain() {
        let mut ch_l = vec![0.5f32, -0.2f32, 1.0f32];
        let mut ch_r = vec![0.1f32, -0.8f32, 0.4f32];

        let mut block = AudioBlock::new(vec![&mut ch_l, &mut ch_r], 48000).expect("valid block");
        assert_eq!(block.channel_count(), 2);
        assert_eq!(block.sample_count(), 3);
        assert_eq!(block.sample_rate(), 48000);

        block.apply_gain(2.0);

        assert_eq!(ch_l[0], 1.0);
        assert_eq!(ch_l[1], -0.4);
        assert_eq!(ch_l[2], 2.0);

        assert_eq!(ch_r[0], 0.2);
        assert_eq!(ch_r[1], -1.6);
        assert_eq!(ch_r[2], 0.8);
    }

    #[test]
    fn test_mismatched_channel_lengths_rejected() {
        let mut ch1 = vec![0.5f32; 128];
        let mut ch2 = vec![0.5f32; 256];

        assert!(AudioBlock::new(vec![&mut ch1, &mut ch2], 48000).is_none());
    }

    #[test]
    fn test_zero_sample_rate_rejected() {
        let mut ch1 = vec![0.5f32; 128];
        assert!(AudioBlock::new(vec![&mut ch1], 0).is_none());
    }
}
