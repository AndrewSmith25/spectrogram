use clap::Parser;
use raylib::{
	audio::{RaylibAudio, Wave},
	error::LoadSoundError,
	ffi::KeyboardKey,
	text::Font,
};
use spectrogram::{
	AUDIO_STREAM_BUF_SIZE, ComputeMessage, State,
	transform::{StftResult, stft},
};
use std::{
	ffi::OsStr,
	path::{Path, PathBuf},
	str::FromStr,
	sync::mpsc::{Receiver, Sender, TryRecvError, channel},
	thread,
};

#[derive(clap::Parser)]
struct CliArgs {
	audio_file_path: String,
	#[arg(short, long)]
	font_file: Option<String>,
}

fn decode_file_format(contents: &[u8]) -> Option<&'static str> {
	if contents.len() < 12 {
		return None;
	}
	match contents[0..12] {
		[0xFF, 0xFB, _, _, _, _, _, _, _, _, _, _] => Some(".mp3"),
		[0xFF, 0xF3, _, _, _, _, _, _, _, _, _, _] => Some(".mp3"),
		[0xFF, 0xF2, _, _, _, _, _, _, _, _, _, _] => Some(".mp3"),
		[0x49, 0x44, 0x33, _, _, _, _, _, _, _, _, _] => Some(".mp3"),
		[0x4F, 0x67, 0x67, 0x53, _, _, _, _, _, _, _, _] => Some(".ogg"),
		[0x52, 0x49, 0x46, 0x46, _, _, _, _, 0x57, 0x41, 0x56, 0x45] => Some(".wav"),
		[0x52, 0x49, 0x46, 0x46, _, _, _, _, 0x41, 0x56, 0x49, 0x20] => Some(".avi"),
		_ => None,
	}
}

fn get_wave_from_file<'a>(audio: &'a RaylibAudio, path: &str) -> Result<Wave<'a>, LoadSoundError> {
	let file_contents = std::fs::read(path).unwrap();
	let extension = decode_file_format(&file_contents)
		.map(ToString::to_string)
		.unwrap_or_default();
	println!("detected file extension: {extension}");
	audio.new_wave_from_memory(&extension, &file_contents)
}

fn main() {
	let args = CliArgs::parse();

	// init systems
	let (mut rl, rl_thread) = raylib::init()
		.size(640 * 2, 480 * 2)
		.title("Spectrogram")
		.build();
	let audio = RaylibAudio::init_audio_device().unwrap();
	audio.set_audio_stream_buffer_size_default(AUDIO_STREAM_BUF_SIZE as i32);

	// get necessary resources
	let wave = get_wave_from_file(&audio, &args.audio_file_path).expect("Unsupported file format");
	let font = args
		.font_file
		.and_then(|path| rl.load_font(&rl_thread, &path).ok())
		.map(Font::make_weak); // leaks memory lol

	// set up app state and compute thread
	let (request_sender, request_receiver) = channel::<ComputeMessage>();
	let (result_sender, result_receiver) = channel::<StftResult>();
	let mut state = State::new(
		&mut rl,
		&rl_thread,
		&audio,
		wave,
		font,
		(request_sender, result_receiver),
	);
	let compute_thread_handle = start_compute_thread(request_receiver, result_sender);

	// misc variables
	let mut redraw = true;

	rl.set_target_fps(60);
	while !rl.window_should_close() {
		let dt = rl.get_frame_time();
		state.player.tick(dt);

		let ctrl_down = rl.is_key_down(KeyboardKey::KEY_LEFT_CONTROL)
			|| rl.is_key_down(KeyboardKey::KEY_RIGHT_CONTROL);
		let shift_down = rl.is_key_down(KeyboardKey::KEY_LEFT_SHIFT)
			|| rl.is_key_down(KeyboardKey::KEY_RIGHT_SHIFT);
		let shift_multiplier = if shift_down { 5 } else { 1 };

		// player controls
		if rl.is_key_pressed(KeyboardKey::KEY_SPACE) {
			if state.player.is_playing() {
				state.player.pause();
			} else {
				state.player.play();
			}
		}
		if rl.is_key_pressed(KeyboardKey::KEY_LEFT) {
			state
				.player
				.set_time(state.player.get_time() - shift_multiplier as f32);
		}
		if rl.is_key_pressed(KeyboardKey::KEY_RIGHT) {
			state
				.player
				.set_time(state.player.get_time() + shift_multiplier as f32);
		}

		// spectrogram controls
		if ctrl_down {
			if rl.is_key_pressed(KeyboardKey::KEY_DOWN) {
				state.time_overlap = state.time_overlap.saturating_sub(shift_multiplier);
				redraw = true;
			}
			if rl.is_key_pressed(KeyboardKey::KEY_UP) {
				state.time_overlap = state.time_overlap.saturating_add(shift_multiplier);
				redraw = true;
			}
		} else {
			if rl.is_key_pressed(KeyboardKey::KEY_DOWN) {
				state.time_resolution = state
					.time_resolution
					.saturating_sub(shift_multiplier)
					.max(2);
				redraw = true;
			}
			if rl.is_key_pressed(KeyboardKey::KEY_UP) {
				state.time_resolution = state
					.time_resolution
					.saturating_add(shift_multiplier)
					.max(2);
				redraw = true;
			}
		}

		// handle window resize
		if rl.is_window_resized() {
			// redraw = true;
		}

		// request new spectrogram texture if needed
		if redraw {
			state.start_generate_spectrogram_job();
			redraw = false;
		}
		state.handle_finished_compute(&mut rl, &rl_thread);

		// rendering
		rl.draw(&rl_thread, |mut d| {
			state.draw(&mut d);
		});
	}

	// clean up thread
	state
		.compute_channel
		.0
		.send(ComputeMessage::Goodbye)
		.expect("compute thread panicked (impossible)");
	let _ = compute_thread_handle.thread();
}

fn start_compute_thread(
	request_receiver: Receiver<ComputeMessage>,
	result_sender: Sender<StftResult>,
) -> thread::JoinHandle<(Receiver<ComputeMessage>, Sender<StftResult>)> {
	thread::spawn(move || {
		'thread: while let Ok(mut msg) = request_receiver.recv() {
			'skip: loop {
				match request_receiver.try_recv() {
					Ok(next) => msg = next, // skip to last sent request
					Err(TryRecvError::Empty) => break 'skip,
					Err(TryRecvError::Disconnected) => break 'thread,
				}
			}
			match msg {
				ComputeMessage::ComputeStft {
					samples,
					segment_length,
					sample_rate,
					offset_amount,
				} => {
					if result_sender
						.send(stft(&samples, sample_rate, segment_length, offset_amount))
						.is_err()
					{
						break 'thread;
					}
				}
				ComputeMessage::Goodbye => break 'thread,
			}
		}
		(request_receiver, result_sender)
	})
}
