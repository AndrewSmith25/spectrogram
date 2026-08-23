use rustfft::{
	FftPlanner,
	num_complex::{Complex, ComplexFloat},
};

#[derive(Clone, Copy, Debug)]
pub struct FrequencyData {
	pub frequency: f32,
	pub amplitude: f32,
	pub phase: f32,
}

#[derive(Clone, Debug)]
pub struct StftSegment {
	pub spectrum: Vec<FrequencyData>,
	pub offset: usize,
	pub segment_length: usize,
}
#[derive(Clone, Debug)]
pub struct StftResult {
	pub segments: Vec<StftSegment>,
	pub sample_rate: usize,
	pub num_samples: usize,
}

pub fn stft(
	samples: &[f32],
	sample_rate: usize,
	segment_length: usize,
	hop_size: usize,
) -> StftResult {
	let mut planner = FftPlanner::<f32>::new();
	let exact_fft = planner.plan_fft_forward(segment_length);
	let mut fft_output;
	let mut fft_scratch;

	let mut segments = Vec::new();
	let mut offset = 0;
	while offset < samples.len() {
		let num_samples_remaining = samples.len() - offset;
		let segment_length = segment_length.min(num_samples_remaining);
		let segment_samples = &samples[offset..(offset + segment_length)];

		let hann_window = (0..segment_length)
			.map(|i| i as f32 / segment_length as f32) // 0..1
			.map(|t| t - 0.5) // -0.5..0.5
			.map(|t| (-2.0 * t * t).exp());
		let scaled = segment_samples
			.iter()
			.zip(hann_window)
			.map(|(&a, b)| a * b)
			.map(|s| Complex::new(s, 0.0))
			.collect::<Vec<_>>();

		fft_output = vec![Complex::default(); segment_length];
		let mut processed = if segment_length == exact_fft.len() {
			fft_scratch = vec![Complex::default(); exact_fft.get_immutable_scratch_len()];
			exact_fft.process_immutable_with_scratch(&scaled, &mut fft_output, &mut fft_scratch);
			fft_output.clone()
		} else {
			let fft = planner.plan_fft_forward(segment_length);
			fft_scratch = vec![Complex::default(); fft.get_immutable_scratch_len()];
			fft.process_immutable_with_scratch(&scaled, &mut fft_output, &mut fft_scratch);
			fft_output.clone()
		};

		processed.truncate(processed.len() / 2);
		processed.iter_mut().for_each(|c| *c *= 2.0);
		let frequency_resolution = sample_rate as f32 / segment_length as f32;
		let spectrum = processed
			.iter()
			.enumerate()
			.map(|(i, c)| {
				let frequency = i as f32 * frequency_resolution;
				let amplitude = c.abs() / segment_length as f32;
				let phase = c.im.atan2(c.re);
				FrequencyData {
					frequency,
					amplitude,
					phase,
				}
			})
			.collect::<Vec<_>>();

		let segment = StftSegment {
			spectrum,
			offset,
			segment_length,
		};
		segments.push(segment);
		offset += hop_size
	}
	StftResult {
		segments,
		sample_rate,
		num_samples: samples.len(),
	}
}
