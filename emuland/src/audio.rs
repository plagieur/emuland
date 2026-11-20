//! La sortie son : un flux vers la carte son de Windows, alimenté par les échantillons de l'émulateur.
//!
//! L'émulateur écrit ses échantillons dans une file ; la carte son les lit à son propre rythme.
//! Si la file se vide (le jeu est en pause), on joue du silence ; si elle grossit trop (l'émulateur
//! va un peu plus vite que la carte son), on jette les plus anciens pour ne pas accumuler de retard.

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SizedSample};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

/// Volume général : les 4 voix additionnées restent largement sous 1, on laisse de la marge.
const GAIN: f32 = 0.6;

struct Shared {
    queue: Mutex<VecDeque<f32>>,
    muted: AtomicBool,
    // compteurs de diagnostic (voir `EMULAND_AUDIO_LOG`)
    pushed_frames: AtomicU64,
    played_frames: AtomicU64,
    silent_frames: AtomicU64,
}

/// La partie que l'émulateur utilise pour envoyer du son. Peut être copiée librement.
#[derive(Clone)]
pub struct Sink {
    shared: Arc<Shared>,
    pub sample_rate: u32,
}

impl Sink {
    /// Ajoute des échantillons (paires gauche/droite). Rien n'est joué si le son est coupé.
    pub fn push(&self, samples: &[f32]) {
        if self.shared.muted.load(Ordering::Relaxed) {
            return;
        }
        self.shared.pushed_frames.fetch_add(samples.len() as u64 / 2, Ordering::Relaxed);
        let mut q = self.shared.queue.lock().unwrap();
        q.extend(samples.iter().copied());
        let max = self.sample_rate as usize / 4 * 2; // 250 ms de stéréo
        if q.len() > max {
            let extra = q.len() - max;
            q.drain(..extra);
        }
    }

    pub fn is_muted(&self) -> bool {
        self.shared.muted.load(Ordering::Relaxed)
    }

    pub fn set_muted(&self, muted: bool) {
        self.shared.muted.store(muted, Ordering::Relaxed);
        if muted {
            self.clear();
        }
    }

    /// Vide la file (au changement de jeu ou au redémarrage).
    pub fn clear(&self) {
        self.shared.queue.lock().unwrap().clear();
    }
}

/// Ouvre la carte son par défaut. Renvoie le flux (à garder en vie) et le `Sink` pour y écrire,
/// ou `None` si aucune sortie son n'est disponible.
pub fn start() -> Option<(cpal::Stream, Sink)> {
    let device = cpal::default_host().default_output_device()?;
    let config = device.default_output_config().ok()?;
    let sample_rate = config.sample_rate();
    let channels = config.channels() as usize;
    let shared = Arc::new(Shared {
        queue: Mutex::new(VecDeque::new()),
        muted: AtomicBool::new(false),
        pushed_frames: AtomicU64::new(0),
        played_frames: AtomicU64::new(0),
        silent_frames: AtomicU64::new(0),
    });
    let stream_config = config.config();
    let stream = match config.sample_format() {
        cpal::SampleFormat::F32 => build::<f32>(&device, &stream_config, channels, sample_rate, shared.clone())?,
        cpal::SampleFormat::I16 => build::<i16>(&device, &stream_config, channels, sample_rate, shared.clone())?,
        cpal::SampleFormat::U16 => build::<u16>(&device, &stream_config, channels, sample_rate, shared.clone())?,
        _ => return None,
    };
    stream.play().ok()?;
    if std::env::var_os("EMULAND_AUDIO_LOG").is_some() {
        spawn_log(shared.clone(), format!("{:?} · {} Hz · {} canaux · {:?}", device.description().map(|d| d.to_string()).unwrap_or_default(), sample_rate, channels, config.sample_format()));
    }
    Some((stream, Sink { shared, sample_rate }))
}

/// Diagnostic : si la variable d'environnement `EMULAND_AUDIO_LOG` existe, écrit chaque seconde
/// dans `%APPDATA%\emuland\audio.log` combien d'échantillons ont été produits, joués et remplacés par du silence.
fn spawn_log(shared: Arc<Shared>, header: String) {
    let path = std::env::var_os("APPDATA").map(std::path::PathBuf::from).unwrap_or_default().join("emuland").join("audio.log");
    std::thread::spawn(move || {
        let mut log = format!("sortie : {header}\n");
        let _ = std::fs::write(&path, &log);
        for second in 1..=600 {
            std::thread::sleep(std::time::Duration::from_secs(1));
            let (pushed, played, silent) = (shared.pushed_frames.load(Ordering::Relaxed), shared.played_frames.load(Ordering::Relaxed), shared.silent_frames.load(Ordering::Relaxed));
            log.push_str(&format!("t={second}s produits={pushed} joués={played} silence={silent}\n"));
            let _ = std::fs::write(&path, &log);
        }
    });
}

fn build<T>(device: &cpal::Device, config: &cpal::StreamConfig, channels: usize, sample_rate: u32, shared: Arc<Shared>) -> Option<cpal::Stream>
where
    T: SizedSample + FromSample<f32>,
{
    // on ne commence à lire qu'une fois ~40 ms accumulés, pour absorber les petites irrégularités
    let prime = sample_rate as usize / 25 * 2;
    let mut primed = false;
    device
        .build_output_stream(
            config.clone(),
            move |data: &mut [T], _| {
                let mut q = shared.queue.lock().unwrap();
                if !primed && q.len() >= prime {
                    primed = true;
                }
                for frame in data.chunks_mut(channels) {
                    let (l, r) = if primed {
                        match (q.pop_front(), q.pop_front()) {
                            (Some(l), Some(r)) => {
                                shared.played_frames.fetch_add(1, Ordering::Relaxed);
                                (l * GAIN, r * GAIN)
                            }
                            _ => {
                                primed = false; // file vide : on attend qu'elle se remplisse de nouveau
                                shared.silent_frames.fetch_add(1, Ordering::Relaxed);
                                (0.0, 0.0)
                            }
                        }
                    } else {
                        shared.silent_frames.fetch_add(1, Ordering::Relaxed);
                        (0.0, 0.0)
                    };
                    for (i, out) in frame.iter_mut().enumerate() {
                        let v = match (channels, i) {
                            (1, _) => (l + r) / 2.0,
                            (_, 0) => l,
                            (_, 1) => r,
                            _ => 0.0,
                        };
                        *out = T::from_sample(v);
                    }
                }
            },
            |err| eprintln!("erreur audio : {err}"),
            None,
        )
        .ok()
}
