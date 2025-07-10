use std::{
    sync::{
        Arc,
        OnceLock,
    },
    thread,
};

use tokio::sync::{
    mpsc,
    oneshot,
};
use tracing::Span;

use crate::{
    DirectSamplingMode,
    Error,
    Gain,
    TunerGainMode,
    TunerType,
    handle::Handle,
};

#[derive(Clone, Debug)]
pub(crate) struct Control {
    /// the handle for the rtlsdr. this also provides convenient methods. all
    /// methods except reads are synchronized.
    handle: Arc<Handle>,

    /// sender to send commands to control thread for slow control commands
    control_queue_sender: mpsc::Sender<ControlMessage>,
}

impl Control {
    pub fn new(handle: Arc<Handle>) -> Self {
        let control_queue_sender = get_control_queue_sender();

        Self {
            handle,
            control_queue_sender,
        }
    }

    pub async fn get_center_frequency(&self) -> Result<u32, Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::GetCenterFrequency {
                handle: self.handle.clone(),
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn set_center_frequency(&self, frequency: u32) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetCenterFrequency {
                handle: self.handle.clone(),
                frequency,
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn get_sample_rate(&self) -> Result<u32, Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::GetSampleRate {
                handle: self.handle.clone(),
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn set_sample_rate(&self, sample_rate: u32) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetSampleRate {
                handle: self.handle.clone(),
                sample_rate,
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub fn get_tuner_type(&self) -> TunerType {
        self.handle.tuner_type
    }

    pub fn get_tuner_gains(&self) -> &[i32] {
        self.handle.tuner_gains.as_ref()
    }

    pub async fn get_tuner_gain(&self) -> Result<i32, Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::GetTunerGain {
                handle: self.handle.clone(),
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn set_tuner_gain(&self, gain: Gain) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetTunerGain {
                handle: self.handle.clone(),
                gain,
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn set_tuner_if_gain(&self, stage: i32, gain: i32) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetTunerIfGain {
                handle: self.handle.clone(),
                stage,
                gain,
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn set_tuner_bandwidth(&self, bandwidth: u32) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetTunerBandwidth {
                handle: self.handle.clone(),
                bandwidth,
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn set_agc_mode(&self, enable: bool) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetAgcMode {
                handle: self.handle.clone(),
                enable,
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn get_frequency_correction(&self) -> Result<i32, Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::GetFrequencyCorrection {
                handle: self.handle.clone(),
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn set_frequency_correction(&self, ppm: i32) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetFrequencyCorrection {
                handle: self.handle.clone(),
                ppm,
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn get_offset_tuning(&self) -> Result<bool, Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::GetOffsetTuning {
                handle: self.handle.clone(),
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn set_offset_tuning(&self, enable: bool) -> Result<(), Error> {
        // from the rtlsdr_set_offset_tuning code:
        //
        // ```c
        // if ((dev->tuner_type == RTLSDR_TUNER_R820T) ||
        //   (dev->tuner_type == RTLSDR_TUNER_R828D)) {
        //   /* RTL-SDR-BLOG Hack, enables us to turn on the bias tee by
        //    * clicking on "offset tuning" in software that doesn't have
        //    * specified bias tee support. Offset tuning is not used for
        //    *R820T devices so it is no problem.
        //    */
        //    rtlsdr_set_bias_tee(dev, on);
        //    return -2;
        // }
        // ```
        //
        // we will return Error::Unsupported if anyone tries to **enable** Bias-T on an
        // R8xx. If we decided that we want to allow this hack, we need to also accept
        // -2 as a ok return value.
        if self.handle.tuner_type.is_r82xx() {
            return Err(Error::Unsupported);
        }

        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetOffsetTuning {
                handle: self.handle.clone(),
                enable,
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    async fn get_xtal_frequency(&self) -> Result<(u32, u32), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::GetXtalFrequency {
                handle: self.handle.clone(),
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    async fn set_xtal_frequency(
        &self,
        rtl_xtal_frequency: Option<u32>,
        tuner_xtal_frequency: Option<u32>,
    ) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetXtalFrequency {
                handle: self.handle.clone(),
                rtl_xtal_frequency,
                tuner_xtal_frequency,
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub async fn get_rtl_xtal(&self) -> Result<u32, Error> {
        let (rtl_frequency, _tuner_frequency) = self.get_xtal_frequency().await?;
        Ok(rtl_frequency)
    }

    pub async fn set_rtl_xtal(&self, frequency: u32) -> Result<(), Error> {
        self.set_xtal_frequency(Some(frequency), None).await
    }

    pub async fn get_tuner_xtal(&self) -> Result<u32, Error> {
        let (_rtl_frequency, tuner_frequency) = self.get_xtal_frequency().await?;
        Ok(tuner_frequency)
    }

    pub async fn set_tuner_xtal(&self, frequency: u32) -> Result<(), Error> {
        self.set_xtal_frequency(None, Some(frequency)).await
    }

    pub async fn set_bias_tee(&self, enable: bool) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetBiasTee {
                handle: self.handle.clone(),
                pin: 0,
                enable,
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }

    pub(crate) async fn set_direct_sampling(
        &self,
        mode: Option<DirectSamplingMode>,
    ) -> Result<(), Error> {
        let (result_sender, result_receiver) = oneshot::channel();
        self.control_queue_sender
            .send(ControlMessage::SetDirectSampling {
                handle: self.handle.clone(),
                mode,
                result_sender,
                span: Span::current(),
            })
            .await
            .map_err(|_| Error::ControlThreadDead)?;
        result_receiver
            .await
            .map_err(|_| Error::ControlThreadDead)?
    }
}

/// a call to rtlsdr_set_center_freq takes about 50ms. that's too long for async
/// - especially since we're shoving millions of samples per second at the same
/// time.
///
/// therefore we use a separate thread to run all the slow control commands.
/// we'll use one thread for all RtlSdr objects though.
fn control_thread(mut control_queue_receiver: mpsc::Receiver<ControlMessage>) {
    fn set_tuner_gain(handle: &Handle, gain: Gain) -> Result<(), Error> {
        match gain {
            Gain::ManualValue(gain) => {
                // we need to find a supported gain value
                let gain = handle
                    .tuner_gains
                    .iter()
                    .min_by_key(|supported| (**supported - gain).abs())
                    .ok_or(Error::NoSupportedGains)?;

                let mut handle = handle.lock();

                // manual gain mode must be enabled
                handle.set_tuner_gain_mode(TunerGainMode::Manual)?;

                handle.set_tuner_gain(*gain)
            }
            Gain::ManualIndex(index) => {
                // we need to find a supported gain value
                let gain = handle
                    .tuner_gains
                    .as_ref()
                    .get(index)
                    .ok_or(Error::InvalidGainIndex { index })?;

                let mut handle = handle.lock();

                // manual gain mode must be enabled
                handle.set_tuner_gain_mode(TunerGainMode::Manual)?;

                handle.set_tuner_gain(*gain)
            }
            Gain::Auto => {
                let mut handle = handle.lock();
                handle.set_tuner_gain_mode(TunerGainMode::Auto)
            }
        }
    }

    fn set_xtal_frequency(
        handle: &Handle,
        rtl_xtal_frequency: Option<u32>,
        tuner_xtal_frequency: Option<u32>,
    ) -> Result<(), Error> {
        let mut handle = handle.lock();
        let current = handle.get_xtal_frequency()?;
        handle.set_xtal_frequency(
            rtl_xtal_frequency.unwrap_or(current.0),
            tuner_xtal_frequency.unwrap_or(current.1),
        )
    }

    while let Some(command) = control_queue_receiver.blocking_recv() {
        match command {
            ControlMessage::GetCenterFrequency {
                handle,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.get_center_frequency();
                let _ = result_sender.send(result);
            }
            ControlMessage::SetCenterFrequency {
                handle,
                frequency,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.set_center_frequency(frequency);
                let _ = result_sender.send(result);
            }
            ControlMessage::GetSampleRate {
                handle,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.get_sample_rate();
                let _ = result_sender.send(result);
            }
            ControlMessage::SetSampleRate {
                handle,
                sample_rate,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.set_sample_rate(sample_rate);
                let _ = result_sender.send(result);
            }
            ControlMessage::GetTunerGain {
                handle,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.get_tuner_gain();
                let _ = result_sender.send(result);
            }
            ControlMessage::SetTunerGain {
                handle,
                gain,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let result = set_tuner_gain(&handle, gain);
                let _ = result_sender.send(result);
            }
            ControlMessage::SetTunerIfGain {
                handle,
                stage,
                gain,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.set_tuner_if_gain(stage, gain);
                let _ = result_sender.send(result);
            }
            ControlMessage::SetTunerBandwidth {
                handle,
                bandwidth,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.set_tuner_bandwidth(bandwidth);
                let _ = result_sender.send(result);
            }
            ControlMessage::SetAgcMode {
                handle,
                enable,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.set_agc_mode(enable);
                let _ = result_sender.send(result);
            }
            ControlMessage::GetFrequencyCorrection {
                handle,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.get_frequency_correction();
                let _ = result_sender.send(result);
            }
            ControlMessage::SetFrequencyCorrection {
                handle,
                ppm,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.set_frequency_correction(ppm);
                let _ = result_sender.send(result);
            }
            ControlMessage::GetOffsetTuning {
                handle,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.get_offset_tuning();
                let _ = result_sender.send(result);
            }
            ControlMessage::SetOffsetTuning {
                handle,
                enable,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.set_offset_tuning(enable);
                let _ = result_sender.send(result);
            }
            ControlMessage::GetXtalFrequency {
                handle,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.get_xtal_frequency();
                let _ = result_sender.send(result);
            }
            ControlMessage::SetXtalFrequency {
                handle,
                rtl_xtal_frequency,
                tuner_xtal_frequency,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let result = set_xtal_frequency(&handle, rtl_xtal_frequency, tuner_xtal_frequency);
                let _ = result_sender.send(result);
            }
            ControlMessage::SetBiasTee {
                handle,
                pin,
                enable,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.set_bias_tee(pin, enable);
                let _ = result_sender.send(result);
            }
            ControlMessage::SetDirectSampling {
                handle,
                mode,
                result_sender,
                span,
            } => {
                let _guard = span.enter();
                let mut handle = handle.lock();
                let result = handle.set_direct_sampling(mode);
                let _ = result_sender.send(result);
            }
        }
    }

    tracing::warn!("control thread terminating");
}

pub(crate) const CONTROL_QUEUE_SIZE: usize = 128;

/// returns a sender to send commands to the control handler thread.
fn get_control_queue_sender() -> mpsc::Sender<ControlMessage> {
    static CONTROL_QUEUE_SENDER: OnceLock<mpsc::Sender<ControlMessage>> = OnceLock::new();
    let control_queue_sender = CONTROL_QUEUE_SENDER.get_or_init(|| {
        tracing::debug!("spawning control thread");

        let (control_queue_sender, control_queue_receiver) = mpsc::channel(CONTROL_QUEUE_SIZE);

        thread::spawn(move || {
            control_thread(control_queue_receiver);
        });

        control_queue_sender
    });

    control_queue_sender.clone()
}

enum ControlMessage {
    GetCenterFrequency {
        handle: Arc<Handle>,
        result_sender: oneshot::Sender<Result<u32, Error>>,
        span: Span,
    },
    SetCenterFrequency {
        handle: Arc<Handle>,
        frequency: u32,
        result_sender: oneshot::Sender<Result<(), Error>>,
        span: Span,
    },
    GetSampleRate {
        handle: Arc<Handle>,
        result_sender: oneshot::Sender<Result<u32, Error>>,
        span: Span,
    },
    SetSampleRate {
        handle: Arc<Handle>,
        sample_rate: u32,
        result_sender: oneshot::Sender<Result<(), Error>>,
        span: Span,
    },
    GetTunerGain {
        handle: Arc<Handle>,
        result_sender: oneshot::Sender<Result<i32, Error>>,
        span: Span,
    },
    SetTunerGain {
        handle: Arc<Handle>,
        gain: Gain,
        result_sender: oneshot::Sender<Result<(), Error>>,
        span: Span,
    },
    SetTunerIfGain {
        handle: Arc<Handle>,
        stage: i32,
        gain: i32,
        result_sender: oneshot::Sender<Result<(), Error>>,
        span: Span,
    },
    SetTunerBandwidth {
        handle: Arc<Handle>,
        bandwidth: u32,
        result_sender: oneshot::Sender<Result<(), Error>>,
        span: Span,
    },
    SetAgcMode {
        handle: Arc<Handle>,
        enable: bool,
        result_sender: oneshot::Sender<Result<(), Error>>,
        span: Span,
    },
    GetFrequencyCorrection {
        handle: Arc<Handle>,
        result_sender: oneshot::Sender<Result<i32, Error>>,
        span: Span,
    },
    SetFrequencyCorrection {
        handle: Arc<Handle>,
        ppm: i32,
        result_sender: oneshot::Sender<Result<(), Error>>,
        span: Span,
    },
    GetOffsetTuning {
        handle: Arc<Handle>,
        result_sender: oneshot::Sender<Result<bool, Error>>,
        span: Span,
    },
    SetOffsetTuning {
        handle: Arc<Handle>,
        enable: bool,
        result_sender: oneshot::Sender<Result<(), Error>>,
        span: Span,
    },
    GetXtalFrequency {
        handle: Arc<Handle>,
        result_sender: oneshot::Sender<Result<(u32, u32), Error>>,
        span: Span,
    },
    SetXtalFrequency {
        handle: Arc<Handle>,
        rtl_xtal_frequency: Option<u32>,
        tuner_xtal_frequency: Option<u32>,
        result_sender: oneshot::Sender<Result<(), Error>>,
        span: Span,
    },
    SetBiasTee {
        handle: Arc<Handle>,
        pin: u8,
        enable: bool,
        result_sender: oneshot::Sender<Result<(), Error>>,
        span: Span,
    },
    SetDirectSampling {
        handle: Arc<Handle>,
        mode: Option<DirectSamplingMode>,
        result_sender: oneshot::Sender<Result<(), Error>>,
        span: Span,
    },
}
