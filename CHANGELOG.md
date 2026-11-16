# Changes

This file lists the changes of each version, newest first. The release
workflow copies the section of a version into the notes of its release.
Write the section before you make the tag. Keep each line short.

## 0.2.0

- The window program can update itself. It asks the GitHub page of the
  project for a newer version at its start and one time each day.
- The program asks you before it installs an update. It does not install
  while a voice session runs.
- The installer of each update has a signature. The program installs only
  an update with a correct signature.
- The new setting `updates.check` (Setup screen: "Check for updates at the
  start and one time each day") switches the automatic check off. The
  button "Check for updates" on the Diagnostics screen still works.

## 0.1.0

First public version.

- Voice control for EDCoPilot without VoiceAttack.
- Speech recognition on your own PC. An OpenAI-compatible speech server is
  an option.
- The program reads the voice commands from your EDCoPilot installation.
- Push-to-talk with a key or a joystick button, or always-listening with an
  optional wake word.
- Window program with setup, microphone test, command list, word tables,
  and diagnostics.
- Your own phrases in other languages through alias files.
