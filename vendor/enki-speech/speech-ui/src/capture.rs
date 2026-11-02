use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use rubato::{FftFixedIn, Resampler};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc::{self, Receiver},
};
pub(super) struct Capture {
    stream: cpal::Stream,
    receiver: Receiver<Vec<f32>>,
    failed: Arc<AtomicBool>,
    mono: Vec<f32>,
    rate: u32,
}
impl Capture {
    pub fn start() -> Result<Self, String> {
        let device = cpal::default_host().default_input_device().ok_or("No microphone is available")?;
        let supported = device.default_input_config().map_err(|e| format!("Microphone unavailable: {e}"))?;
        let config: cpal::StreamConfig = supported.clone().into();
        if config.channels == 0 || config.channels > 8 || !(8_000..=192_000).contains(&config.sample_rate.0) {
            return Err("This microphone format is not supported".into());
        }
        let (send, receiver) = mpsc::sync_channel(8);
        let failed = Arc::new(AtomicBool::new(false));
        fn stream<T>(
            device: &cpal::Device,
            config: &cpal::StreamConfig,
            send: mpsc::SyncSender<Vec<f32>>,
            failed: Arc<AtomicBool>,
        ) -> Result<cpal::Stream, String>
        where
            T: cpal::SizedSample,
            f32: cpal::FromSample<T>,
        {
            let error = failed.clone();
            let channels = config.channels as usize;
            device
                .build_input_stream(
                    config,
                    move |data: &[T], _| {
                        if data.len() > 192_000 * 8 {
                            failed.store(true, Ordering::Relaxed);
                            return;
                        }
                        let values = data
                            .chunks_exact(channels)
                            .map(|frame| frame.iter().map(|s| s.to_sample::<f32>()).filter(|n| n.is_finite()).sum::<f32>() / channels as f32)
                            .collect();
                        if send.try_send(values).is_err() {
                            failed.store(true, Ordering::Relaxed);
                        }
                    },
                    move |_| {
                        error.store(true, Ordering::Relaxed);
                    },
                    None,
                )
                .map_err(|e| format!("Microphone access failed: {e}"))
        }
        let microphone = match supported.sample_format() {
            cpal::SampleFormat::F32 => stream::<f32>(&device, &config, send, failed.clone()),
            cpal::SampleFormat::I16 => stream::<i16>(&device, &config, send, failed.clone()),
            cpal::SampleFormat::U16 => stream::<u16>(&device, &config, send, failed.clone()),
            _ => return Err("This microphone sample format is not supported".into()),
        }?;
        microphone.play().map_err(|e| e.to_string())?;
        Ok(Self { stream: microphone, receiver, failed, mono: Vec::new(), rate: config.sample_rate.0 })
    }
    pub fn poll(&mut self) -> Result<(), String> {
        if self.failed.load(Ordering::Relaxed) {
            return Err("Microphone disconnected or audio could not be processed in time. Recording stopped.".into());
        }
        while let Ok(data) = self.receiver.try_recv() {
            let room = (self.rate as usize * 30).saturating_sub(self.mono.len());
            self.mono.extend(data.into_iter().take(room));
        }
        Ok(())
    }
    pub fn finish(mut self) -> Result<(Vec<f32>, u32), String> {
        let _ = self.stream.pause();
        self.poll()?;
        if self.mono.is_empty() {
            return Err("No microphone audio was captured".into());
        }
        Ok((self.mono, self.rate))
    }
}
/// Band-limited resampling stays off both the device callback and the GUI thread.
pub(super) fn normalize(audio: &[f32], rate: u32) -> Result<Vec<i16>, String> {
    if !(8_000..=192_000).contains(&rate) || audio.len() > rate as usize * 30 || audio.is_empty() {
        return Err("Invalid or oversized microphone audio".into());
    }
    let wanted = audio.len().saturating_mul(16_000) / rate as usize;
    let mut converter = FftFixedIn::<f32>::new(rate as usize, 16_000, 1024, 2, 1).map_err(|e| e.to_string())?;
    let mut delay = converter.output_delay();
    let mut pcm = Vec::with_capacity(wanted);
    for chunk in audio.chunks(1024).chain(std::iter::repeat_n(&[][..], 4)) {
        let mut frame = chunk.to_vec();
        frame.resize(1024, 0.0);
        let output = converter.process(&[frame], None).map_err(|e| e.to_string())?;
        if let Some(samples) = output.first() {
            let skip = delay.min(samples.len());
            delay -= skip;
            pcm.extend(samples.iter().skip(skip).take(wanted.saturating_sub(pcm.len())).map(|s| (s.clamp(-1.0, 1.0) * i16::MAX as f32) as i16));
        }
        if pcm.len() == wanted {
            return Ok(pcm);
        }
    }
    Err("Microphone conversion could not complete".into())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn converts_common_hardware_rates_to_bounded_mono_pcm() {
        for rate in [8_000, 16_000, 44_100, 48_000, 96_000, 192_000] {
            let audio = vec![0.25; rate / 10];
            let pcm = normalize(&audio, rate as u32).unwrap();
            assert_eq!(pcm.len(), 1600);
            assert!(pcm.iter().skip(200).take(1000).all(|s| (*s as i32 - 8191).abs() < 250));
        }
        assert!(normalize(&[], 48_000).is_err());
        assert!(normalize(&[0.0], 0).is_err());
        assert!(normalize(&vec![0.0; 48000 * 31], 48000).is_err());
    }
}
