use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Shared audio state between the emulator and the audio callback thread.
struct AudioBuffer {
    queue: VecDeque<(i16, i16)>,
    last_sample: (i16, i16),
}

pub struct AudioOutput {
    _stream: cpal::Stream,
    shared: Arc<Mutex<AudioBuffer>>,
    pub sample_rate: u32,
}

impl AudioOutput {
    pub fn new() -> Option<Self> {
        let host = cpal::default_host();
        eprintln!("Audio: using host: {}", host.id().name());

        let device = match host.default_output_device() {
            Some(d) => {
                eprintln!("Audio: found output device");
                d
            }
            None => {
                eprintln!("Audio: no default output device found");
                return None;
            }
        };

        let default_config = match device.default_output_config() {
            Ok(c) => {
                eprintln!(
                    "Audio: default device config: {} ch, {} Hz, format {:?}",
                    c.channels(),
                    c.sample_rate().0,
                    c.sample_format()
                );
                c
            }
            Err(e) => {
                eprintln!("Audio: failed to get default output config: {}", e);
                return None;
            }
        };

        let sample_rate = default_config.sample_rate();
        let config = cpal::StreamConfig {
            channels: 2,
            sample_rate,
            buffer_size: cpal::BufferSize::Default,
        };

        let shared = Arc::new(Mutex::new(AudioBuffer {
            queue: VecDeque::with_capacity(sample_rate.0 as usize / 5),
            last_sample: (0, 0),
        }));

        let shared_clone = Arc::clone(&shared);

        let stream = match default_config.sample_format() {
            cpal::SampleFormat::I16 => device.build_output_stream(
                &config,
                move |data: &mut [i16], _: &cpal::OutputCallbackInfo| {
                    let mut ab = shared_clone.lock().unwrap();
                    for frame in data.chunks_exact_mut(2) {
                        if let Some((l, r)) = ab.queue.pop_front() {
                            ab.last_sample = (l, r);
                        }
                        let (l, r) = ab.last_sample;
                        frame[0] = l;
                        frame[1] = r;
                    }
                },
                |err| eprintln!("Audio stream error: {}", err),
                None,
            ),
            cpal::SampleFormat::F32 => device.build_output_stream(
                &config,
                move |data: &mut [f32], _: &cpal::OutputCallbackInfo| {
                    let mut ab = shared_clone.lock().unwrap();
                    for frame in data.chunks_exact_mut(2) {
                        if let Some((l, r)) = ab.queue.pop_front() {
                            ab.last_sample = (l, r);
                        }
                        let (l, r) = ab.last_sample;
                        frame[0] = l as f32 / 32768.0;
                        frame[1] = r as f32 / 32768.0;
                    }
                },
                |err| eprintln!("Audio stream error: {}", err),
                None,
            ),
            format => {
                eprintln!("Audio: unsupported sample format {:?}", format);
                return None;
            }
        };

        let stream = match stream {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Audio: failed to build output stream: {}", e);
                return None;
            }
        };

        if let Err(e) = stream.play() {
            eprintln!("Audio: failed to play stream: {}", e);
            return None;
        }

        eprintln!("Audio: initialized successfully");
        Some(Self {
            _stream: stream,
            shared,
            sample_rate: sample_rate.0,
        })
    }

    pub fn push_samples(&self, samples: &[(i16, i16)]) {
        if let Ok(mut ab) = self.shared.lock() {
            ab.queue.extend(samples.iter());
            // Cap at ~100ms to prevent unbounded growth
            let cap = self.sample_rate as usize / 10;
            while ab.queue.len() > cap {
                ab.queue.pop_front();
            }
        }
    }
}
