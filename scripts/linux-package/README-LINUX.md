# Skate 3 Rust Engine: Linux

## Getting started
1. Unpack this folder anywhere you like, for example `~/Games/Skate3Rust`.
2. Run `./Play.sh`, or run `./Install-Shortcut.sh` once to add the game to your
   application menu.
3. On first launch, setup asks for **your own** Skate 3 Xbox 360 ISO, or the
   `default.xex` inside an already extracted copy of the disc. Setup converts it
   into this folder's `data` directory. It needs about 8 GB of temporary space
   for an ISO and leaves roughly 1.3 GB behind. No game files are included or
   downloaded.

Later launches go straight into the game. Plug in a controller before or after
launching.

## Requirements
- 64-bit Linux with glibc 2.31 or newer (Ubuntu 20.04+, Debian 11+, Fedora 32+,
  Arch, SteamOS 3 and similar).
- A Vulkan driver (Mesa or NVIDIA), ALSA/PipeWire audio and udev.
- Optional: `ffmpeg`, for board sounds, music and per-map ambience. Setup skips
  these if ffmpeg is missing. To add them later, install ffmpeg, delete
  `data/installation.json` and launch again.
- Optional: `zenity` or `kdialog` for the desktop's own file picker and error pop-ups.

## Sound and music
With `ffmpeg` installed, setup also converts the disc's board sounds (rolling,
pops, landings, grinds, powerslides, bails), per-map ambience and the 46-song
in-game soundtrack. In game, **N** skips to the next song and **M** mutes the
music. The mapping from board events to clips lives in
`data/installations/<id>/assets/private/audio/board/board.json` and can be edited
without running setup again.

## Custom deck graphics
In the customiser, choose Board > Deck > **Add image from computer…** and pick
a PNG or JPEG; it is equipped straight away (choose Done to keep it). The file
chooser can open behind a fullscreen game window. You can also drop images into
the `custom-boards` folder (created on first launch) and restart the game. They appear under Board > Deck as
"Custom: <file name>". `deck-template.png` in that folder shows where the art
sits on the texture; images are resized to 512x512.

## Adding to Steam
Use *Add a Non-Steam Game*, browse to `Play.sh` and launch it from Steam. The
launcher switches to XWayland under Steam so Steam Input can reach your pad.

## Differences from the Windows package
- Automatic updates are Windows-only. To update, unpack the new package over
  this folder. Your `data`, settings and mods are kept, and setup offers an
  asset refresh only if the converters changed.
- Custom Models import (Mixamo/FBX) is not bundled yet.
- Steam relay multiplayer is not included.

## Troubleshooting
- Game logs: `logs/`. Setup errors: `data/setup-error.log`. Converter output:
  `data/installations/<id>/setup.log`.
- To run setup again from scratch, delete the `data` folder.
