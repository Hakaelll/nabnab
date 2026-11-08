export interface PlaybackSignal {
  bass: number;
  mids: number;
  highs: number;
  level: number;
  attack: number;
  stereo: number;
  waveform: Float32Array<ArrayBuffer>;
}

function rms(samples: Float32Array): number {
  let squares = 0;
  for (const value of samples) squares += value * value;
  return Math.sqrt(squares / samples.length);
}

export function connectPlaybackSignal(media: HTMLMediaElement) {
  const context = new AudioContext();
  const meter = new AnalyserNode(context, { fftSize: 2048, smoothingTimeConstant: 0.44 });
  const splitter = new ChannelSplitterNode(context, { numberOfOutputs: 2 });
  const left = new AnalyserNode(context, { fftSize: 256 });
  const right = new AnalyserNode(context, { fftSize: 256 });
  const master = new GainNode(context, { gain: media.muted ? 0 : media.volume });
  let source: MediaElementAudioSourceNode;
  try { source = context.createMediaElementSource(media); }
  catch (error) { void context.close(); throw error; }

  source.connect(meter).connect(master).connect(context.destination);
  source.connect(splitter);
  splitter.connect(left, 0);
  splitter.connect(right, 1);
  media.volume = 1;
  media.muted = false;

  const samples = new Float32Array(meter.fftSize);
  const frequencies = new Float32Array(meter.frequencyBinCount);
  const channels = [new Float32Array(left.fftSize), new Float32Array(right.fftSize)] as const;
  const signal: PlaybackSignal = { bass: 0, mids: 0, highs: 0, level: 0, attack: 0, stereo: 0, waveform: new Float32Array(48) };
  const frequencyStep = context.sampleRate / meter.fftSize;
  let floor = 0;
  let peak = 0.055;
  let inhibit = 0.24;

  function smooth(old: number, value: number, dt: number, rise = 0.065, fall = 0.32) {
    return value + (old - value) * Math.exp(-dt / (value > old ? rise : fall));
  }

  function band(low: number, high: number): number {
    const start = Math.ceil(low / frequencyStep);
    const end = Math.min(frequencies.length, Math.ceil(high / frequencyStep));
    let energy = 0;
    for (let bin = start; bin < end; bin++) energy += 10 ** (frequencies[bin] / 10);
    return Math.min(1, Math.sqrt(energy) * 2.6);
  }

  function read(dt: number, playing: boolean): PlaybackSignal {
    let level = 0, bass = 0, mids = 0, highs = 0, stereo = 0;
    if (playing) {
      meter.getFloatTimeDomainData(samples);
      level = rms(samples);
      if (level > 0.0008) {
        meter.getFloatFrequencyData(frequencies);
        bass = band(35, 180);
        mids = band(180, 2000);
        highs = band(2000, 11000);
        left.getFloatTimeDomainData(channels[0]);
        right.getFloatTimeDomainData(channels[1]);
        const a = rms(channels[0]), b = rms(channels[1]);
        if (a > 0.001 && b > 0.001) stereo = (a - b) / (a + b);
      }
    }
    peak = Math.max(0.055, smooth(peak, level, dt, 0.42, 9));
    const normalized = Math.min(1, level / peak);
    const impulse = Math.max(0, (normalized - floor - 0.12) * 2.8);
    floor = smooth(floor, normalized, dt, 0.28, 0.65);
    inhibit = Math.max(0, inhibit - dt);
    signal.bass = smooth(signal.bass, bass, dt);
    signal.mids = smooth(signal.mids, mids, dt);
    signal.highs = smooth(signal.highs, highs, dt, 0.045, 0.24);
    signal.stereo = smooth(signal.stereo, stereo, dt, 0.16, 0.16);
    signal.level = smooth(signal.level, normalized, dt, 0.08, 0.46);
    signal.attack = Math.max(inhibit ? 0 : Math.min(1, impulse), signal.attack * Math.exp(-dt / 0.23));
    for (let index = 0; index < signal.waveform.length; index++) {
      const sample = playing && level > 0.0008 ? samples[1024 + index * 16] : 0;
      signal.waveform[index] = smooth(signal.waveform[index], Math.max(-1, Math.min(1, sample / peak)), dt, 0.043, 0.09);
    }
    return signal;
  }

  return {
    read,
    reset() {
      floor = 0; peak = 0.055; inhibit = 0.24;
      signal.bass = signal.mids = signal.highs = signal.level = signal.attack = signal.stereo = 0;
      signal.waveform.fill(0);
    },
    volume(value: number) { master.gain.setTargetAtTime(value, context.currentTime, 0.012); },
    resume: () => context.resume(),
    suspend: () => context.suspend(),
  };
}
