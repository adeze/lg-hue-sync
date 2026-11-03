use super::zones::{FloatColor, RgbColor};

const LUMA_BINS: usize = 32;
const TRIM_FRACTION: f32 = 0.01;

#[derive(Clone, Copy, Default)]
struct Bin {
    weight: f32,
    rgb: [f32; 3],
    highlight_weight: f32,
    highlight_rgb: [f32; 3],
    max_luma: f32,
}

/// Bounded regional statistics in the existing encoded RGB domain.
#[derive(Default)]
pub(crate) struct RegionSample {
    bins: [Bin; LUMA_BINS],
    weight: f32,
}

impl RegionSample {
    pub fn add(&mut self, pixel: RgbColor, weight: f32) {
        if !weight.is_finite() || weight <= 0.0 {
            return;
        }
        let luma = pixel.luminance();
        let index = ((luma * LUMA_BINS as f32) as usize).min(LUMA_BINS - 1);
        let bin = &mut self.bins[index];
        let highlight_weight = weight * luma.powi(4);
        for (index, component) in [pixel.r, pixel.g, pixel.b].into_iter().enumerate() {
            bin.rgb[index] += f32::from(component) * weight;
            bin.highlight_rgb[index] += f32::from(component) * highlight_weight;
        }
        bin.weight += weight;
        bin.highlight_weight += highlight_weight;
        bin.max_luma = bin.max_luma.max(luma);
        self.weight += weight;
    }

    pub fn finish(self) -> (FloatColor, FloatColor, f32) {
        if self.weight == 0.0 {
            return (FloatColor::default(), FloatColor::default(), 0.0);
        }
        let lower = self.weight * TRIM_FRACTION;
        let upper = self.weight - lower;
        let mut accumulated = 0.0_f32;
        let mut retained = Bin::default();
        for bin in self.bins {
            let end = accumulated + bin.weight;
            let kept = (end.min(upper) - accumulated.max(lower)).max(0.0);
            accumulated = end;
            if kept == 0.0 {
                continue;
            }
            // Boundary bins lose only their trimmed mass, preserving within-bin RGB sums.
            let fraction = kept / bin.weight;
            retained.weight += kept;
            retained.highlight_weight += bin.highlight_weight * fraction;
            retained.max_luma = retained.max_luma.max(bin.max_luma);
            for index in 0..3 {
                retained.rgb[index] += bin.rgb[index] * fraction;
                retained.highlight_rgb[index] += bin.highlight_rgb[index] * fraction;
            }
        }
        let colour = |rgb: [f32; 3], weight: f32| {
            FloatColor::new(rgb[0] / weight, rgb[1] / weight, rgb[2] / weight)
        };
        let mean = colour(retained.rgb, retained.weight);
        let highlight = if retained.highlight_weight > 0.0 {
            colour(retained.highlight_rgb, retained.highlight_weight)
        } else {
            mean
        };
        (mean, highlight, retained.max_luma)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_black_and_uniform_regions_preserve_colour() {
        assert_eq!(
            RegionSample::default().finish(),
            (FloatColor::default(), FloatColor::default(), 0.0)
        );
        for pixel in [RgbColor::new(0, 0, 0), RgbColor::new(150, 70, 25)] {
            let mut sample = RegionSample::default();
            sample.add(RgbColor::new(255, 255, 255), f32::NAN);
            sample.add(RgbColor::new(255, 255, 255), -1.0);
            for weight in [0.1, 0.4, 0.7, 1.0] {
                sample.add(pixel, weight);
            }
            let (mean, highlight, luma) = sample.finish();
            assert_eq!(mean.to_rgb(), pixel);
            assert_eq!(highlight.to_rgb(), pixel);
            assert_eq!(luma, pixel.luminance());
        }
    }

    #[test]
    fn sparse_opposing_outliers_are_trimmed_by_weight() {
        let mut sample = RegionSample::default();
        for _ in 0..1000 {
            sample.add(RgbColor::new(80, 80, 80), 1.0);
        }
        // Several pixels with low spatial weight must not outweigh the region.
        for _ in 0..50 {
            sample.add(RgbColor::new(255, 0, 0), 0.1);
            sample.add(RgbColor::new(0, 255, 0), 0.1);
        }
        let (mean, highlight, _) = sample.finish();
        assert_eq!(mean.to_rgb(), RgbColor::new(80, 80, 80));
        assert_eq!(highlight.to_rgb(), RgbColor::new(80, 80, 80));
    }

    #[test]
    fn near_tied_highlights_change_smoothly() {
        let estimate = |green| {
            let mut sample = RegionSample::default();
            for _ in 0..100 {
                sample.add(RgbColor::new(255, 0, 0), 1.0);
                sample.add(RgbColor::new(0, green, 0), 1.0);
            }
            sample.finish().1
        };
        let before = estimate(76);
        let after = estimate(77);
        assert!((after.r - before.r).abs() < 4.0);
        assert!((after.g - before.g).abs() < 4.0);
        assert_eq!(after.b, 0.0);
        assert!(before.r > 100.0 && before.g > 30.0);
    }
}
