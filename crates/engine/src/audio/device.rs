//! Sound card access through cpal (WASAPI on Windows, CoreAudio on macOS, ALSA on Linux).

use anyhow::{Context, Result, anyhow};
use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample};
use tracing::warn;

use super::resample::Resampler;

fn device_label(d: &cpal::Device) -> String {
    d.description().map(|x| x.name().to_string()).unwrap_or_else(|_| "Unknown device".to_string())
}

fn device_id(d: &cpal::Device) -> Option<String> {
    d.id().ok().map(|i| i.to_string())
}

/// (id, human name) for every input and output device.
pub fn list_devices() -> (Vec<(String, String)>, Vec<(String, String)>) {
    let host = cpal::default_host();
    let collect = |it: Option<Vec<cpal::Device>>| -> Vec<(String, String)> {
        it.unwrap_or_default().iter().filter_map(|d| Some((device_id(d)?, device_label(d)))).collect()
    };
    let inputs = collect(host.input_devices().ok().map(|i| i.collect()));
    let outputs = collect(host.output_devices().ok().map(|i| i.collect()));
    (inputs, outputs)
}

fn find_device(input: bool, id: Option<&str>) -> Result<cpal::Device> {
    let host = cpal::default_host();
    if let Some(id) = id {
        if let Ok(parsed) = id.parse::<cpal::DeviceId>() {
            if let Some(d) = host.device_by_id(&parsed) {
                return Ok(d);
            }
        }
        warn!("audio device {id} not found, using the default");
    }
    let d = if input { host.default_input_device() } else { host.default_output_device() };
    d.ok_or_else(|| anyhow!(if input { "no microphone found" } else { "no speakers or headphones found" }))
}

pub struct InputDevice {
    _stream: cpal::Stream,
    cons: rtrb::Consumer<f32>,
    resampler: Resampler,
    pending: Vec<f32>,
    raw: Vec<f32>,
    pub name: String,
}

fn build_input<T>(device: &cpal::Device, config: cpal::StreamConfig, mut prod: rtrb::Producer<f32>) -> Result<cpal::Stream>
where
    T: SizedSample + Send + 'static,
    f32: FromSample<T>,
{
    let ch = config.channels.max(1) as usize;
    let stream = device.build_input_stream(
        config,
        move |data: &[T], _: &cpal::InputCallbackInfo| {
            for frame in data.chunks(ch) {
                let mut s = 0f32;
                for v in frame {
                    s += f32::from_sample_(*v);
                }
                let _ = prod.push(s / ch as f32);
            }
        },
        |e| warn!("microphone stream error: {e}"),
        None,
    )?;
    Ok(stream)
}

impl InputDevice {
    pub fn open(id: Option<&str>) -> Result<InputDevice> {
        let device = find_device(true, id)?;
        let name = device_label(&device);
        let supported = device.default_input_config().context("microphone has no usable format")?;
        let format = supported.sample_format();
        let config = supported.config();
        let rate = config.sample_rate;
        let (prod, cons) = rtrb::RingBuffer::new((rate as usize).max(48_000));
        let stream = match format {
            SampleFormat::F32 => build_input::<f32>(&device, config, prod)?,
            SampleFormat::I16 => build_input::<i16>(&device, config, prod)?,
            SampleFormat::I32 => build_input::<i32>(&device, config, prod)?,
            SampleFormat::U16 => build_input::<u16>(&device, config, prod)?,
            SampleFormat::F64 => build_input::<f64>(&device, config, prod)?,
            other => return Err(anyhow!("unsupported microphone sample format {other:?}")),
        };
        stream.play()?;
        Ok(InputDevice {
            _stream: stream,
            cons,
            resampler: Resampler::new(rate, 48_000),
            pending: Vec::with_capacity(4096),
            raw: Vec::with_capacity(4096),
            name,
        })
    }

    /// Fill `out` with the next 10 ms (480 samples at 48 kHz) if available.
    pub fn read_frame(&mut self, out: &mut [f32]) -> bool {
        let avail = self.cons.slots();
        if avail > 0 {
            self.raw.clear();
            if let Ok(chunk) = self.cons.read_chunk(avail) {
                let (a, b) = chunk.as_slices();
                self.raw.extend_from_slice(a);
                self.raw.extend_from_slice(b);
                chunk.commit_all();
            }
            let raw = std::mem::take(&mut self.raw);
            self.resampler.process(&raw, &mut self.pending);
            self.raw = raw;
        }
        // Don't let a stalled consumer build up latency.
        if self.pending.len() > 48_000 / 5 {
            let drop = self.pending.len() - 480 * 2;
            self.pending.drain(..drop);
        }
        if self.pending.len() >= 480 {
            out[..480].copy_from_slice(&self.pending[..480]);
            self.pending.drain(..480);
            true
        } else {
            false
        }
    }
}

pub struct OutputDevice {
    _stream: cpal::Stream,
    prod: rtrb::Producer<f32>,
    capacity: usize,
    rate: u32,
    channels: usize,
    rs_l: Resampler,
    rs_r: Resampler,
    l: Vec<f32>,
    r: Vec<f32>,
    tmp_l: Vec<f32>,
    tmp_r: Vec<f32>,
    pub name: String,
}

fn build_output<T>(device: &cpal::Device, config: cpal::StreamConfig, mut cons: rtrb::Consumer<f32>) -> Result<cpal::Stream>
where
    T: SizedSample + FromSample<f32> + Send + 'static,
{
    let stream = device.build_output_stream(
        config,
        move |data: &mut [T], _: &cpal::OutputCallbackInfo| {
            let n = cons.slots().min(data.len());
            if let Ok(chunk) = cons.read_chunk(n) {
                let (a, b) = chunk.as_slices();
                for (d, s) in data.iter_mut().zip(a.iter().chain(b.iter())) {
                    *d = T::from_sample_(*s);
                }
                chunk.commit_all();
            }
            for d in data[n..].iter_mut() {
                *d = T::from_sample_(0.0f32);
            }
        },
        |e| warn!("speaker stream error: {e}"),
        None,
    )?;
    Ok(stream)
}

impl OutputDevice {
    pub fn open(id: Option<&str>) -> Result<OutputDevice> {
        let device = find_device(false, id)?;
        let name = device_label(&device);
        let supported = device.default_output_config().context("speakers have no usable format")?;
        let format = supported.sample_format();
        let config = supported.config();
        let rate = config.sample_rate;
        let channels = config.channels.max(1) as usize;
        let capacity = rate as usize * channels / 2; // 500 ms
        let (prod, cons) = rtrb::RingBuffer::new(capacity);
        let stream = match format {
            SampleFormat::F32 => build_output::<f32>(&device, config, cons)?,
            SampleFormat::I16 => build_output::<i16>(&device, config, cons)?,
            SampleFormat::I32 => build_output::<i32>(&device, config, cons)?,
            SampleFormat::U16 => build_output::<u16>(&device, config, cons)?,
            SampleFormat::F64 => build_output::<f64>(&device, config, cons)?,
            other => return Err(anyhow!("unsupported speaker sample format {other:?}")),
        };
        stream.play()?;
        Ok(OutputDevice {
            _stream: stream,
            prod,
            capacity,
            rate,
            channels,
            rs_l: Resampler::new(48_000, rate),
            rs_r: Resampler::new(48_000, rate),
            l: Vec::with_capacity(1024),
            r: Vec::with_capacity(1024),
            tmp_l: Vec::with_capacity(1024),
            tmp_r: Vec::with_capacity(1024),
            name,
        })
    }

    pub fn buffered_ms(&self) -> f32 {
        let used = self.capacity - self.prod.slots();
        used as f32 / self.channels as f32 / self.rate as f32 * 1000.0
    }

    /// Queue 10 ms of 48 kHz stereo (480 interleaved frames).
    pub fn write(&mut self, stereo: &[f32]) {
        self.l.clear();
        self.r.clear();
        for i in 0..480 {
            self.l.push(stereo[2 * i]);
            self.r.push(stereo[2 * i + 1]);
        }
        self.tmp_l.clear();
        self.tmp_r.clear();
        self.rs_l.process(&self.l, &mut self.tmp_l);
        self.rs_r.process(&self.r, &mut self.tmp_r);
        let frames = self.tmp_l.len().min(self.tmp_r.len());
        let n = frames * self.channels;
        if self.prod.slots() < n {
            return; // the device stalled; drop rather than add latency
        }
        if let Ok(mut chunk) = self.prod.write_chunk_uninit(n) {
            let (a, b) = chunk.as_mut_slices();
            let mut i = 0usize;
            let ch = self.channels;
            let (tl, tr) = (&self.tmp_l, &self.tmp_r);
            let put = |slot: &mut std::mem::MaybeUninit<f32>, idx: usize| {
                let frame = idx / ch;
                let c = idx % ch;
                let v = match (ch, c) {
                    (1, _) => 0.5 * (tl[frame] + tr[frame]),
                    (_, 0) => tl[frame],
                    (_, 1) => tr[frame],
                    _ => 0.0,
                };
                slot.write(v);
            };
            for s in a.iter_mut() {
                put(s, i);
                i += 1;
            }
            for s in b.iter_mut() {
                put(s, i);
                i += 1;
            }
            unsafe { chunk.commit_all() };
        }
    }
}
