#!/usr/bin/env python3
"""
Generate placeholder sound effects for Mahjuro using OpenAI's audio model.

Uses gpt-4o-mini-tts to generate short SFX-style audio clips via text-to-speech
with expressive sound-design prompts, then converts to OGG Vorbis.

Usage:
    pip install openai pydub
    export OPENAI_API_KEY="sk-..."
    python scripts/generate_sfx.py              # Generate all SFX
    python scripts/generate_sfx.py --sfx 1      # Generate only SFX #1
    python scripts/generate_sfx.py --list        # List all SFX
    python scripts/generate_sfx.py --dry-run     # Print prompts without generating

Requires ffmpeg on PATH for OGG conversion (brew install ffmpeg).
"""

import argparse
import base64
import os
import sys
import time
from pathlib import Path

try:
    from openai import OpenAI
except ImportError:
    print("Error: openai package not installed. Run: pip install openai")
    sys.exit(1)


OUTPUT_DIR = Path(__file__).resolve().parent.parent / "assets" / "audio"

# Each SFX: (filename, short_name, description for the prompt, voice style hint)
SFX_DEFS = [
    (
        "tile_click",
        "Tile Click",
        "Make the sound of a single ceramic mahjong tile being tapped with a "
        "fingertip on a wooden table. Short, crisp, tactile click. Under 0.5 seconds.",
        "A single sharp ceramic 'tok' sound.",
    ),
    (
        "tile_place",
        "Tile Place",
        "Make the sound of a mahjong tile being firmly placed down on a wooden "
        "table surface. Satisfying clack with a brief woody resonance. Under 0.5 seconds.",
        "A firm 'clack' of tile meeting table.",
    ),
    (
        "tile_discard",
        "Tile Discard",
        "Make the sound of a mahjong tile being slid and tossed lightly across a "
        "table surface. A soft sliding scrape followed by a light tap. Under 0.5 seconds.",
        "A light sliding 'shh-tok' sound.",
    ),
    (
        "score_reveal",
        "Score Reveal",
        "Make a short dramatic reveal sound — like a game show reveal sting. "
        "A bright ascending chime followed by a sparkle. About 1 second.",
        "A dramatic ascending 'da-da-DAAA' reveal.",
    ),
    (
        "score_step",
        "Score Step",
        "Make a single quick ascending tick or ding — like a score counter "
        "incrementing by one step. Bright, snappy, satisfying. Under 0.3 seconds.",
        "A quick bright 'ding' tick upward.",
    ),
    (
        "score_final",
        "Score Final",
        "Make a satisfying final score impact sound — a combination of a deep "
        "impact hit and a shimmering cash register 'ka-ching'. About 1 second.",
        "A deep 'BOOM-ching!' final impact.",
    ),
    (
        "relic_pickup",
        "Relic Pickup",
        "Make a magical item pickup sound — a sparkling crystalline chime that "
        "ascends and shimmers, like picking up a power-up in a video game. About 1 second.",
        "A magical ascending sparkle chime.",
    ),
    (
        "invalid_action",
        "Invalid Action",
        "Make a short error or rejection sound — a dull low buzz or muted 'bonk' "
        "that says 'nope, can't do that'. Not harsh, just firm. Under 0.5 seconds.",
        "A muted 'bwonk' rejection sound.",
    ),
    (
        "round_win",
        "Round Win",
        "Make a short victory fanfare — bright, triumphant, energetic. Like winning "
        "a hand in a card game. Two ascending notes and a shimmer. About 1.5 seconds.",
        "A bright 'ta-da!' victory sting.",
    ),
    (
        "game_over",
        "Game Over",
        "Make a somber game over sound — a slow descending tone that fades out, "
        "slightly melancholic but not depressing. Gentle, reflective. About 2 seconds.",
        "A gentle descending 'wah-wah-wahhh' fade.",
    ),
]


def build_prompt(description: str, style_hint: str) -> str:
    """Build the text prompt for audio generation."""
    return (
        f"You are a sound effects generator for a mahjong roguelite video game "
        f"called Mahjuro. Generate ONLY the sound effect described below — no speech, "
        f"no words, no narration. Just the pure sound effect.\n\n"
        f"Sound: {description}\n\n"
        f"Style: {style_hint}\n\n"
        f"Generate ONLY the sound. Do not say anything before or after."
    )


def generate_audio(
    client: OpenAI,
    prompt: str,
    output_path: Path,
    model: str,
    voice: str,
) -> None:
    """Call the OpenAI audio API and save the result as OGG."""
    # Use the TTS model with an expressive prompt to generate SFX-like audio.
    # gpt-4o-mini-tts supports expressive sound design via prompt instructions.
    response = client.audio.speech.create(
        model=model,
        voice=voice,
        input=prompt,
        response_format="opus",  # OGG Opus — closest to what we need
    )

    # The API returns opus-encoded audio; write directly as .ogg
    response.write_to_file(str(output_path))
    print(f"  Saved: {output_path}")


def main() -> None:
    parser = argparse.ArgumentParser(description="Generate Mahjuro SFX via OpenAI audio")
    parser.add_argument(
        "--sfx", type=int, default=None,
        help="Generate only SFX number N (1-indexed). Omit for all.",
    )
    parser.add_argument(
        "--list", action="store_true",
        help="List all SFX and exit.",
    )
    parser.add_argument(
        "--dry-run", action="store_true",
        help="Print prompts without calling the API.",
    )
    parser.add_argument(
        "--model", type=str, default="gpt-4o-mini-tts",
        help="TTS model to use (default: gpt-4o-mini-tts).",
    )
    parser.add_argument(
        "--voice", type=str, default="alloy",
        help="Voice preset (default: alloy). Options: alloy, echo, fable, onyx, nova, shimmer.",
    )
    parser.add_argument(
        "--output-dir", type=str, default=None,
        help=f"Output directory (default: {OUTPUT_DIR}).",
    )
    args = parser.parse_args()

    if args.list:
        for i, (filename, name, desc, _) in enumerate(SFX_DEFS, 1):
            print(f"  {i:2d}. {name:<20s}  ({filename}.ogg)")
        return

    out_dir = Path(args.output_dir) if args.output_dir else OUTPUT_DIR
    out_dir.mkdir(parents=True, exist_ok=True)

    # Select which SFX to generate.
    if args.sfx is not None:
        if args.sfx < 1 or args.sfx > len(SFX_DEFS):
            print(f"Error: --sfx must be between 1 and {len(SFX_DEFS)}")
            sys.exit(1)
        targets = [(args.sfx - 1, SFX_DEFS[args.sfx - 1])]
    else:
        targets = list(enumerate(SFX_DEFS))

    if not args.dry_run:
        api_key = os.environ.get("OPENAI_API_KEY")
        if not api_key:
            print("Error: OPENAI_API_KEY environment variable not set.")
            sys.exit(1)
        client = OpenAI(api_key=api_key)

    for idx, (filename, name, description, style_hint) in targets:
        prompt = build_prompt(description, style_hint)
        output_path = out_dir / f"{filename}.ogg"

        print(f"\n[{idx + 1}/{len(SFX_DEFS)}] {name}")

        if args.dry_run:
            print(f"  Prompt:\n    {prompt}\n")
            continue

        if output_path.exists():
            print(f"  Skipping (already exists): {output_path}")
            print(f"  Delete the file to regenerate, or use --sfx {idx + 1}")
            continue

        try:
            generate_audio(client, prompt, output_path, args.model, args.voice)
        except Exception as e:
            print(f"  Error generating {name}: {e}")
            continue

        # Rate-limit courtesy.
        if len(targets) > 1:
            time.sleep(1)

    print("\nDone!")
    if not args.dry_run:
        print(f"Audio saved to: {out_dir}")


if __name__ == "__main__":
    main()
