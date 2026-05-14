---
category: changed
---

Zodiac silk ribbons now use a single tall portrait texture per zodiac (`zodiac_<slug>.png`) instead of a fragile 3-piece tile set (`_top` / `_mid` / `_bot`). `gpt-image-2` accepts portrait aspects up to 3:1 directly, so the entire ribbon — finial, embroidered animal, and tasselled tip — is generated in one shot. Renderer collapses to one mesh + one bind group per ribbon (3× fewer draw slots used per zodiac), shadow caster does the same, and `scripts/generate_zodiac_ribbons.py` is half the size with no joining-edge prompt hacks. Re-run the script to regenerate art (the 42 old `_top/_mid/_bot.png` files have been removed).
