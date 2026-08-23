use raylib::{
	audio::{AudioStream, RaylibAudio, Wave},
	error::UpdateAudioStreamError,
};

use crate::AUDIO_STREAM_BUF_SIZE;
#[derive(Debug)]
pub struct WavePlayer<'a> {
	samples: Vec<f32>,
	audio_stream: AudioStream<'a>,
	next_frame_start_index: usize,
	current_time: f32,
}

impl<'a> WavePlayer<'a> {
	pub fn new(audio: &'a RaylibAudio, mut wave: Wave<'_>) -> Self {
		wave.format(wave.sample_rate() as i32, 32, 1);
		let audio_stream =
			audio.new_audio_stream(wave.sample_rate(), wave.sample_size(), wave.channels());

		Self {
			samples: wave.load_samples().as_ref().to_vec(),
			audio_stream,
			current_time: 0.0,
			next_frame_start_index: 0,
		}
	}

	fn update_stream(&mut self) -> Result<(), UpdateAudioStreamError> {
		let len = self.samples.len();
		if self.next_frame_start_index > len {
			self.next_frame_start_index = len;
		}

		let start = self.next_frame_start_index;
		let end = len.min(start + AUDIO_STREAM_BUF_SIZE);
		let frame = &self.samples[start..end];
		self.audio_stream.update(frame)?;
		println!("Samples loaded (frame [{start}..{end}])");
		self.next_frame_start_index = end;

		self.current_time = start as f32 / self.audio_stream.sample_rate() as f32;

		Ok(())
	}

	pub fn tick(&mut self, dt: f32) {
		if self.audio_stream.is_playing() {
			self.current_time += dt;
		}
		if self.current_time > self.length() {
			self.current_time = self.length();
			return;
		}
		if self.audio_stream.is_processed() {
			self.update_stream().unwrap();
		}
	}
	pub fn play(&mut self) {
		if !self.audio_stream.is_playing() {
			self.set_time(self.current_time);
			self.audio_stream.play();
		}
	}
	pub fn pause(&mut self) {
		self.audio_stream.pause();
	}
	pub fn get_time(&self) -> f32 {
		self.current_time
	}

	pub fn set_time(&mut self, time: f32) {
		self.current_time = (0.0f32).max(time).min(self.length());
		self.next_frame_start_index =
			(self.current_time * self.audio_stream.sample_rate() as f32) as usize;
		println!(
			"stream set to play at time {} (index {})",
			self.current_time, self.next_frame_start_index
		);
		self.update_stream().unwrap();
	}

	pub fn length(&self) -> f32 {
		self.samples.len() as f32 / self.audio_stream.sample_rate() as f32
	}

	pub fn is_playing(&self) -> bool {
		self.audio_stream.is_playing()
	}

	pub fn samples(&self) -> &[f32] {
		&self.samples
	}
	pub fn sample_rate(&self) -> usize {
		self.audio_stream.sample_rate() as usize
	}
}
