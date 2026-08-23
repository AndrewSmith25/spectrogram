pub mod player;
pub mod transform;

use std::sync::mpsc::{Receiver, Sender};

use player::WavePlayer;
use raylib::{
	RaylibHandle, RaylibThread,
	audio::{RaylibAudio, Wave},
	drawing::{RaylibDraw, RaylibTextureModeExt},
	ffi::{Color, Rectangle, Vector2, Vector4},
	prelude::{RaylibDrawHandle, RenderTexture2D},
	text::WeakFont,
	texture::RaylibTexture2D,
};

use crate::transform::StftResult;

pub const AUDIO_STREAM_BUF_SIZE: usize = 4096;
pub const TIME_RESOLUTION_MULTIPLIER: f32 = 0.001;
pub const TIME_OVERLAP_MULTIPLIER: f32 = 0.0001;

pub struct State<'a> {
	pub player: WavePlayer<'a>,
	pub font: WeakFont,
	pub spectrogram_texture: Option<RenderTexture2D>,
	pub spectral_analysis: Option<StftResult>,
	pub compute_channel: (Sender<ComputeMessage>, Receiver<StftResult>),
	pub compute_waiting: bool,
	pub time_resolution: usize,
	pub time_overlap: usize,
}
impl<'a> State<'a> {
	const DRAW_BAR: bool = true;

	pub fn new(
		_rl: &mut RaylibHandle,
		_thread: &RaylibThread,
		audio: &'a RaylibAudio,
		wave: Wave,
		font: Option<WeakFont>,
		compute_channel: (Sender<ComputeMessage>, Receiver<StftResult>),
	) -> Self {
		let player = WavePlayer::new(audio, wave);
		let font = font.unwrap_or_else(|| _rl.get_font_default());

		State {
			player,
			font,
			spectrogram_texture: None,
			spectral_analysis: None,
			compute_channel,
			compute_waiting: true,
			time_resolution: 20, /* 1.0 ms */
			time_overlap: 190,    /* 0.1 ms */
		}
	}

	pub fn draw(&mut self, d: &mut RaylibDrawHandle) {
		d.clear_background(Color::new(0, 0, 0, 255));
		if let Some(texture) = self.spectrogram_texture.as_ref() {
			let src = Rectangle::new(0.0, 0.0, texture.width() as f32, texture.height() as f32);
			let dst = Rectangle::new(
				0.0,
				0.0,
				d.get_render_width() as f32,
				d.get_render_height() as f32,
			);
			println!("src = {src:?}");
			println!("dst = {dst:?}");
			d.draw_texture_pro(texture, src, dst, Vector2::default(), 0.0, Color::WHITE);
		}

		let mut info_lines = Vec::new();
		info_lines.push(format!(
			"Time Resolution: {:.1} ms / Window Overlap: {:.1} ms",
			self.time_resolution as f32 * TIME_RESOLUTION_MULTIPLIER * 1000.0,
			self.time_overlap as f32 * TIME_OVERLAP_MULTIPLIER * 1000.0,
		));
		info_lines.push(format!(
			"Frequency Resolution: {:.3}",
			1.0 / (self.time_resolution as f32 * TIME_RESOLUTION_MULTIPLIER),
		));
		if self.compute_waiting {
			info_lines.push(format!("Computing spectrogram..."));
		}

		if Self::DRAW_BAR {
			let progress = self.player.get_time() / self.player.length();
			let bar_x = (progress * d.get_render_width() as f32) as i32;
			d.draw_rectangle(bar_x, 0, 3, d.get_render_height(), (255, 0, 0, 255));
		}

        let font_size = 32.0 * d.get_render_width() as f32 / 1280.0;
		for (i, line) in info_lines.iter().enumerate() {
			d.draw_text_ex(
				&self.font,
				&line,
				Vector2::new(4.0, font_size * i as f32 + 4.0),
				font_size,
				2.0,
				Color::WHITE,
			);
		}
	}

	pub fn start_generate_spectrogram_job(&mut self) {
		let time_resolution = self.time_resolution as f32 * TIME_RESOLUTION_MULTIPLIER;
		let time_overlap = self.time_overlap as f32 * TIME_OVERLAP_MULTIPLIER;

		let samples = self.player.samples();
		let sample_rate = self.player.sample_rate();
		let segment_length = (time_resolution * sample_rate as f32) as usize;

		let overlap_amount = time_overlap * sample_rate as f32;
		let offset_amount = segment_length
			.saturating_sub(overlap_amount as usize)
			.max(20);

		self.spectrogram_texture.take();
		self.spectral_analysis.take();
		self.compute_waiting = true;
		self.compute_channel
			.0
			.send(ComputeMessage::ComputeStft {
				samples: samples.to_vec(),
				segment_length,
				sample_rate,
				offset_amount,
			})
			.unwrap();
	}

	pub fn handle_finished_compute(&mut self, rl: &mut RaylibHandle, rl_thread: &RaylibThread) {
		let Ok(stft) = self.compute_channel.1.try_recv() else {
			return;
		};

		let (width, height) = (stft.segments.len(), stft.segments[0].spectrum.len());
		let mut spectrogram_texture = rl
			.load_render_texture(rl_thread, width as u32, height as u32)
			.unwrap();

		let max_amplitude = stft
			.segments
			.iter()
			.flat_map(|segment| segment.spectrum.iter().map(|freq| freq.amplitude))
			.max_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Less))
			.unwrap_or(1.0);

		rl.draw_texture_mode(rl_thread, &mut spectrogram_texture, |mut d| {
			d.clear_background(Color::BLACK);
			for (x, s) in stft.segments.iter().enumerate() {
				for (y, freq_data) in s.spectrum.iter().enumerate() {
					let db_ref = 60.0;
					let value = freq_data.amplitude / max_amplitude;
					let value = (20.0 * value.log10() + db_ref) / db_ref;
					let value = (0.0f32).max(value).min(1.0);
					d.draw_pixel_v(
						Vector2::new(x as f32, y as f32),
						Color::color_from_normalized(Vector4 {
							x: value,
							y: value,
							z: value,
							w: 1.0,
						}),
					);
				}
			}
		});

		self.spectrogram_texture = Some(spectrogram_texture);
		self.spectral_analysis = Some(stft);
		self.compute_waiting = false;
	}
}

pub enum ComputeMessage {
	ComputeStft {
		samples: Vec<f32>,
		segment_length: usize,
		sample_rate: usize,
		offset_amount: usize,
	},
	Goodbye,
}
