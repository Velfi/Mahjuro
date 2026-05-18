#!/usr/bin/env python3
"""Generate seamless looping main-menu rain ambience (OGG).

Requires ffmpeg on PATH:
    python scripts/generate_main_menu_rain.py

Writes: assets/audio/ambient/main_menu_rain.ogg
"""

from __future__ import annotations

import shutil
import struct
import subprocess
import sys
import tempfile
import wave
from pathlib import Path

REPO = Path(__file__).resolve().parents[1]
OUT = REPO / "assets/audio/ambient/main_menu_rain.ogg"
LOOP_SEC = 14.0
SAMPLE_RATE = 48_000
CHANNELS = 2
CROSSFADE_SEC = 0.45


def run(cmd: list[str]) -> None:
    print("+", " ".join(cmd))
    subprocess.run(cmd, check=True)


def generate_noise_wav(path: Path) -> None:
    """Band-limited pink rain bed (longer than loop; trimmed after crossfade)."""
    duration = LOOP_SEC + CROSSFADE_SEC * 2.0
    af = (
        "highpass=f=320,"
        "lowpass=f=7200,"
        "tremolo=f=0.11:d=0.48,"
        "volume=0.28"
    )
    run(
        [
            "ffmpeg",
            "-y",
            "-f",
            "lavfi",
            "-i",
            f"anoisesrc=color=pink:duration={duration}:sample_rate={SAMPLE_RATE}:amplitude=0.5",
            "-af",
            af,
            "-ac",
            str(CHANNELS),
            str(path),
        ]
    )


def read_wav_frames(path: Path) -> tuple[int, int, list[int]]:
    with wave.open(str(path), "rb") as w:
        ch = w.getnchannels()
        rate = w.getframerate()
        sampwidth = w.getsampwidth()
        if sampwidth != 2:
            raise ValueError(f"expected 16-bit PCM, got width {sampwidth}")
        raw = w.readframes(w.getnframes())
    samples = list(struct.unpack(f"<{len(raw) // 2}h", raw))
    return ch, rate, samples


def write_wav_frames(path: Path, channels: int, rate: int, samples: list[int]) -> None:
    raw = struct.pack(f"<{len(samples)}h", *samples)
    with wave.open(str(path), "wb") as w:
        w.setnchannels(channels)
        w.setframerate(rate)
        w.setsampwidth(2)
        w.writeframes(raw)


def seamless_loop(samples: list[int], channels: int, rate: int) -> list[int]:
    loop_frames = int(LOOP_SEC * rate)
    fade_frames = int(CROSSFADE_SEC * rate)
    frame_count = len(samples) // channels
    if frame_count < loop_frames + fade_frames:
        raise ValueError("source too short for loop crossfade")

    out_frames = loop_frames
    out = [0] * (out_frames * channels)

    for f in range(out_frames):
        for c in range(channels):
            i = f * channels + c
            s = samples[i]
            if f < fade_frames:
                t = f / fade_frames
                tail_f = loop_frames - fade_frames + f
                tail_i = tail_f * channels + c
                tail_s = samples[tail_i]
                s = int(s * t + tail_s * (1.0 - t))
            out[i] = max(-32768, min(32767, s))
    return out


def encode_ogg(wav_in: Path, ogg_out: Path) -> None:
    encoders = [
        ["-c:a", "libvorbis", "-q:a", "5"],
        ["-strict", "-2", "-c:a", "vorbis", "-q:a", "5"],
    ]
    last_err: subprocess.CalledProcessError | None = None
    for enc in encoders:
        try:
            run(["ffmpeg", "-y", "-i", str(wav_in), *enc, str(ogg_out)])
            return
        except subprocess.CalledProcessError as e:
            last_err = e
    if last_err:
        raise last_err


def main() -> int:
    if not shutil.which("ffmpeg"):
        print("ffmpeg not found on PATH", file=sys.stderr)
        return 1

    OUT.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory() as tmp:
        tmp_path = Path(tmp)
        raw_wav = tmp_path / "rain_raw.wav"
        loop_wav = tmp_path / "rain_loop.wav"

        generate_noise_wav(raw_wav)
        ch, rate, samples = read_wav_frames(raw_wav)
        loop_samples = seamless_loop(samples, ch, rate)
        write_wav_frames(loop_wav, ch, rate, loop_samples)
        encode_ogg(loop_wav, OUT)

    print(f"Wrote {OUT.relative_to(REPO)} ({LOOP_SEC:.1f}s loop)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
